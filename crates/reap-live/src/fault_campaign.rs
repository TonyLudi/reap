use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use reap_core::PINNED_JAVA_REVISION;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    LiveConfig, LiveConfigError, LiveConfigFileEvidence, LiveMode, LiveRunFileEvidence,
    LiveRunReport, LiveRunVerificationError, LiveRunVerificationFailure, LiveStopReason,
    MAX_LIVE_RUN_REPORT_BYTES, verify_live_run_paths,
};

pub const LIVE_FAULT_MATRIX_MANIFEST_SCHEMA_VERSION: u32 = 3;
pub const LIVE_FAULT_MATRIX_REPORT_FORMAT_VERSION: u32 = 5;
pub const MAX_LIVE_FAULT_MATRIX_MANIFEST_BYTES: u64 = 1024 * 1024;
pub const MAX_LIVE_FAULT_INJECTOR_EVIDENCE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_LIVE_FAULT_MATRIX_RUNS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveFaultScenario {
    CleanObserve,
    CleanDemo,
    PublicReconnect,
    PrivateReconnect,
    OrderTransportReconnect,
    AmbiguousSubmit,
    AmbiguousCancel,
    PartialFill,
    FillConvergenceTimeout,
    OrderConvergenceTimeout,
    RestoredSafetyLatch,
    DeadmanHeartbeatFailure,
    ExchangeClockFailure,
    ExchangeStatusFailure,
    ExchangeInstrumentFailure,
    ExchangeFeeFailure,
    AccountConfigFailure,
}

impl LiveFaultScenario {
    pub const REQUIRED: [Self; 17] = [
        Self::CleanObserve,
        Self::CleanDemo,
        Self::PublicReconnect,
        Self::PrivateReconnect,
        Self::OrderTransportReconnect,
        Self::AmbiguousSubmit,
        Self::AmbiguousCancel,
        Self::PartialFill,
        Self::FillConvergenceTimeout,
        Self::OrderConvergenceTimeout,
        Self::RestoredSafetyLatch,
        Self::DeadmanHeartbeatFailure,
        Self::ExchangeClockFailure,
        Self::ExchangeStatusFailure,
        Self::ExchangeInstrumentFailure,
        Self::ExchangeFeeFailure,
        Self::AccountConfigFailure,
    ];

    const fn requires_injector_evidence(self) -> bool {
        !matches!(self, Self::CleanObserve | Self::CleanDemo)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveFaultMatrixRunManifest {
    pub scenario: LiveFaultScenario,
    pub report: PathBuf,
    #[serde(default)]
    pub injector_evidence: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveFaultMatrixManifest {
    pub schema_version: u32,
    pub runs: Vec<LiveFaultMatrixRunManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveFaultFileEvidence {
    pub source_path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveFaultMatrixIdentity {
    pub reap_version: String,
    pub executable_sha256: String,
    pub host_identity_sha256: String,
    pub account_identity_sha256s: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveFaultMatrixConfigFailure {
    EnvironmentIsNotDemo,
    HostGuardDisabled,
    SynchronizedClockNotRequired,
    AlertsDisabled,
    AlertDeliveryFailureNotFatal,
    OperatorServiceDisabled,
    PublicWebsocketRedundancyBelowTwo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveFaultScenarioFailure {
    LiveReportEvidenceInvalid,
    InjectorEvidenceMissing,
    InjectorEvidenceInvalid,
    ExpectedObserveMode,
    ExpectedDemoMode,
    ExpectedObserveOrDemoMode,
    CleanSoakRequired,
    SafeBoundedShutdownRequired,
    SafeRuntimeFailureShutdownRequired,
    PublicDisconnectMissing,
    PrivateDisconnectMissing,
    OrderTransportDisconnectMissing,
    OrderTransportStaleMissing,
    ReadinessLossMissing,
    AmbiguousSubmitMissing,
    AmbiguousCancelMissing,
    PartialFillMissing,
    FillConvergenceTimeoutMissing,
    OrderConvergenceTimeoutMissing,
    ReconciliationDriftResponseMissing,
    RestoredSafetyLatchMissing,
    DeadmanHeartbeatFailureMissing,
    ExchangeClockFailureMissing,
    ExchangeStatusFailureMissing,
    ExchangeInstrumentFailureMissing,
    ExchangeFeeFailureMissing,
    AccountConfigFailureMissing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum LiveFaultMatrixFailure {
    UnsupportedManifestSchema {
        actual: u32,
        supported: u32,
    },
    ConfigGate {
        failure: LiveFaultMatrixConfigFailure,
    },
    MissingScenario {
        scenario: LiveFaultScenario,
    },
    DuplicateScenario {
        scenario: LiveFaultScenario,
    },
    DuplicateReport {
        scenario: LiveFaultScenario,
    },
    DuplicateSession {
        scenario: LiveFaultScenario,
    },
    DuplicateInjectorEvidence {
        scenario: LiveFaultScenario,
    },
    EvidencePathCollision {
        scenario: LiveFaultScenario,
    },
    RunIdentityMismatch {
        scenario: LiveFaultScenario,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveFaultObservedEvidence {
    pub reached_ready: bool,
    pub readiness_at_stop_ready: bool,
    pub readiness_loss_count: u64,
    pub reconciliation_drift_events: u64,
    pub public_connection_disconnect_events: u64,
    pub private_connection_disconnect_events: u64,
    pub order_transport_disconnect_events: u64,
    pub order_transport_stale_events: u64,
    pub ambiguous_submit_events: u64,
    pub ambiguous_cancel_events: u64,
    pub partial_fill_events: u64,
    pub fill_convergence_timeout_events: u64,
    pub order_convergence_timeout_events: u64,
    pub restored_safety_latches: u64,
    pub dropped_storage_records: u64,
    pub active_orders_after_shutdown: usize,
    pub alert_delivery_failures: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveFaultMatrixRunVerification {
    pub scenario: LiveFaultScenario,
    pub report: LiveRunFileEvidence,
    pub injector_evidence: Option<LiveFaultFileEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reap_fault_proxy_evidence: Option<LiveFaultProxyEvidenceSummary>,
    pub session_id: Option<String>,
    pub mode: LiveMode,
    pub stop_reason: LiveStopReason,
    pub evidence: LiveFaultObservedEvidence,
    pub live_evidence_valid: bool,
    pub clean_soak_accepted: bool,
    pub identity_matches: bool,
    pub live_verification_failures: Vec<LiveRunVerificationFailure>,
    pub scenario_failures: Vec<LiveFaultScenarioFailure>,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveFaultProxyEvidenceSummary {
    pub format_version: u32,
    pub proxy_session_id: String,
    pub proxy_config_fingerprint: String,
    pub command_id: String,
    pub command_kind: String,
    pub armed_at_ms: u64,
    pub completed_at_ms: u64,
    pub effect_count: usize,
    pub passed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReapFaultProxyEvidenceWire {
    format_version: u32,
    proxy_session_id: String,
    proxy_config_fingerprint: String,
    java_reference_revision: String,
    command_id: String,
    command: ReapFaultCommandWire,
    armed_at_ms: u64,
    completed_at_ms: u64,
    effects: Vec<ReapFaultEffectWire>,
    passed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReapFaultWebSocketTarget {
    Public,
    Private,
    Order,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReapFaultWebSocketDirection {
    ClientToExchange,
    ExchangeToClient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReapFaultWebSocketFrameKind {
    Text,
    Binary,
    Ping,
    Pong,
    Close,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReapFaultRestMatcherWire {
    method: String,
    path: String,
    #[serde(default)]
    query: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReapFaultWebSocketMatcherWire {
    kind: ReapFaultWebSocketFrameKind,
    #[serde(default)]
    json: Option<ReapFaultWebSocketJsonMatcherWire>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReapFaultWebSocketJsonMatcherWire {
    #[serde(default)]
    op: Option<String>,
    #[serde(default)]
    event: Option<String>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    instrument_type: Option<String>,
    #[serde(default)]
    symbol: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ReapFaultCommandWire {
    DisconnectWebsockets {
        target: ReapFaultWebSocketTarget,
        connections: usize,
    },
    RestResponse {
        matcher: ReapFaultRestMatcherWire,
        status: u16,
        response_headers: BTreeMap<String, String>,
        response_body_bytes: u64,
        response_body_sha256: String,
        times: u32,
    },
    WebsocketDrop {
        target: ReapFaultWebSocketTarget,
        direction: ReapFaultWebSocketDirection,
        matcher: ReapFaultWebSocketMatcherWire,
        frames: u32,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ReapFaultEffectWire {
    WebsocketDisconnected {
        sequence: u32,
        applied_at_ms: u64,
        connection_id: u64,
        target: ReapFaultWebSocketTarget,
    },
    RestResponseInjected {
        sequence: u32,
        applied_at_ms: u64,
        method: String,
        path: String,
        query_sha256: String,
    },
    WebsocketFrameDropped {
        sequence: u32,
        applied_at_ms: u64,
        connection_id: u64,
        target: ReapFaultWebSocketTarget,
        direction: ReapFaultWebSocketDirection,
        frame_kind: ReapFaultWebSocketFrameKind,
        frame_bytes: u64,
        frame_sha256: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveFaultMatrixVerificationReport {
    pub format_version: u32,
    pub manifest: LiveFaultFileEvidence,
    pub manifest_schema_version: u32,
    pub config: LiveConfigFileEvidence,
    pub config_fingerprint: String,
    pub evidence_config_fingerprint: String,
    pub identity: Option<LiveFaultMatrixIdentity>,
    pub runs: Vec<LiveFaultMatrixRunVerification>,
    pub failures: Vec<LiveFaultMatrixFailure>,
    pub limitations: Vec<String>,
    pub live_fault_matrix_passed: bool,
}

#[derive(Debug, Error)]
pub enum LiveFaultMatrixError {
    #[error("invalid {label} path {path}: {message}")]
    InvalidPath {
        label: &'static str,
        path: PathBuf,
        message: String,
    },
    #[error("{label} {path} is {actual} bytes; limit is {limit}")]
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
    #[error("failed to parse live fault matrix manifest {path}: {source}")]
    ParseManifest {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("live fault matrix has {actual} runs; limit is {limit}")]
    TooManyRuns { actual: usize, limit: usize },
    #[error("failed to verify live fault report {path}: {source}")]
    VerifyRun {
        path: PathBuf,
        #[source]
        source: LiveRunVerificationError,
    },
    #[error("failed to parse live fault report {path}: {source}")]
    ParseRun {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("live fault report {0} changed while it was being verified")]
    RunChanged(PathBuf),
    #[error(transparent)]
    Config(#[from] LiveConfigError),
}

struct LoadedRun {
    report: LiveRunReport,
    verification: LiveFaultMatrixRunVerification,
}

pub fn verify_live_fault_matrix_paths(
    config_path: impl AsRef<Path>,
    manifest_path: impl AsRef<Path>,
) -> Result<LiveFaultMatrixVerificationReport, LiveFaultMatrixError> {
    let config_path = config_path.as_ref();
    let (config, config_source) = LiveConfig::load_with_evidence(config_path)?;
    let config_fingerprint = config.fingerprint()?;
    let evidence_config_fingerprint = config.evidence_fingerprint()?;
    let (manifest_path, manifest_bytes) = read_bounded_regular_file(
        manifest_path.as_ref(),
        "live fault matrix manifest",
        MAX_LIVE_FAULT_MATRIX_MANIFEST_BYTES,
    )?;
    let manifest: LiveFaultMatrixManifest =
        toml::from_str(std::str::from_utf8(&manifest_bytes).map_err(|error| {
            LiveFaultMatrixError::InvalidPath {
                label: "live fault matrix manifest",
                path: manifest_path.clone(),
                message: format!("is not UTF-8: {error}"),
            }
        })?)
        .map_err(|source| LiveFaultMatrixError::ParseManifest {
            path: manifest_path.clone(),
            source,
        })?;
    if manifest.runs.len() > MAX_LIVE_FAULT_MATRIX_RUNS {
        return Err(LiveFaultMatrixError::TooManyRuns {
            actual: manifest.runs.len(),
            limit: MAX_LIVE_FAULT_MATRIX_RUNS,
        });
    }
    let manifest_evidence = file_evidence(manifest_path.clone(), &manifest_bytes);
    let base = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let mut failures = Vec::new();
    if manifest.schema_version != LIVE_FAULT_MATRIX_MANIFEST_SCHEMA_VERSION {
        failures.push(LiveFaultMatrixFailure::UnsupportedManifestSchema {
            actual: manifest.schema_version,
            supported: LIVE_FAULT_MATRIX_MANIFEST_SCHEMA_VERSION,
        });
    }
    for failure in campaign_config_failures(&config) {
        failures.push(LiveFaultMatrixFailure::ConfigGate { failure });
    }

    let mut scenario_counts = BTreeMap::<LiveFaultScenario, usize>::new();
    let mut report_paths = HashSet::new();
    let mut injector_paths = HashSet::new();
    let mut injector_hashes = HashSet::new();
    let mut evidence_paths =
        HashSet::from([config_source.source_path.clone(), manifest_path.clone()]);
    let mut session_ids = HashSet::new();
    let mut loaded = Vec::with_capacity(manifest.runs.len());
    for run in manifest.runs {
        *scenario_counts.entry(run.scenario).or_default() += 1;
        let report_path = resolve_path(base, &run.report);
        let live_verification =
            verify_live_run_paths(config_path, &report_path, None).map_err(|source| {
                LiveFaultMatrixError::VerifyRun {
                    path: report_path.clone(),
                    source,
                }
            })?;
        let (canonical_report, report_bytes) = read_bounded_regular_file(
            &report_path,
            "live fault report",
            MAX_LIVE_RUN_REPORT_BYTES,
        )?;
        if live_verification.run_report.source_path != canonical_report
            || live_verification.run_report.bytes != report_bytes.len() as u64
            || live_verification.run_report.sha256 != sha256_hex(&report_bytes)
        {
            return Err(LiveFaultMatrixError::RunChanged(canonical_report));
        }
        let report: LiveRunReport = serde_json::from_slice(&report_bytes).map_err(|source| {
            LiveFaultMatrixError::ParseRun {
                path: canonical_report.clone(),
                source,
            }
        })?;
        if !evidence_paths.insert(canonical_report.clone()) {
            failures.push(LiveFaultMatrixFailure::EvidencePathCollision {
                scenario: run.scenario,
            });
        }
        if !report_paths.insert(canonical_report) {
            failures.push(LiveFaultMatrixFailure::DuplicateReport {
                scenario: run.scenario,
            });
        }
        if let Some(session_id) = &report.session_id
            && !session_ids.insert(session_id.clone())
        {
            failures.push(LiveFaultMatrixFailure::DuplicateSession {
                scenario: run.scenario,
            });
        }

        let mut reap_fault_proxy_evidence = None;
        let mut injector_evidence_invalid = false;
        let injector_evidence = run
            .injector_evidence
            .as_ref()
            .map(|path| {
                let path = resolve_path(base, path);
                let (canonical, bytes) = read_bounded_regular_file(
                    &path,
                    "fault injector evidence",
                    MAX_LIVE_FAULT_INJECTOR_EVIDENCE_BYTES,
                )?;
                if bytes.is_empty() {
                    return Err(LiveFaultMatrixError::InvalidPath {
                        label: "fault injector evidence",
                        path: canonical,
                        message: "must not be empty".to_string(),
                    });
                }
                let evidence = file_evidence(canonical.clone(), &bytes);
                if !evidence_paths.insert(canonical.clone()) {
                    failures.push(LiveFaultMatrixFailure::EvidencePathCollision {
                        scenario: run.scenario,
                    });
                }
                if !injector_paths.insert(canonical)
                    || !injector_hashes.insert(evidence.sha256.clone())
                {
                    failures.push(LiveFaultMatrixFailure::DuplicateInjectorEvidence {
                        scenario: run.scenario,
                    });
                }
                match reap_fault_proxy_evidence_summary(&bytes, run.scenario) {
                    Ok(summary) => reap_fault_proxy_evidence = summary,
                    Err(()) => injector_evidence_invalid = true,
                }
                Ok(evidence)
            })
            .transpose()?;
        let mut scenario_failures =
            evaluate_scenario(run.scenario, &report, live_verification.acceptance_passed);
        if !live_verification.evidence_valid {
            scenario_failures.push(LiveFaultScenarioFailure::LiveReportEvidenceInvalid);
        }
        if run.scenario.requires_injector_evidence() && injector_evidence.is_none() {
            scenario_failures.push(LiveFaultScenarioFailure::InjectorEvidenceMissing);
        }
        if injector_evidence_invalid {
            scenario_failures.push(LiveFaultScenarioFailure::InjectorEvidenceInvalid);
        }
        scenario_failures.sort_by_key(fault_failure_rank);
        scenario_failures.dedup();
        let verification = LiveFaultMatrixRunVerification {
            scenario: run.scenario,
            report: live_verification.run_report,
            injector_evidence,
            reap_fault_proxy_evidence,
            session_id: report.session_id.clone(),
            mode: report.mode,
            stop_reason: report.stop_reason,
            evidence: observed_evidence(&report),
            live_evidence_valid: live_verification.evidence_valid,
            clean_soak_accepted: live_verification.acceptance_passed,
            identity_matches: true,
            live_verification_failures: live_verification.failures,
            passed: scenario_failures.is_empty(),
            scenario_failures,
        };
        loaded.push(LoadedRun {
            report,
            verification,
        });
    }

    for scenario in LiveFaultScenario::REQUIRED {
        match scenario_counts.get(&scenario).copied().unwrap_or_default() {
            0 => failures.push(LiveFaultMatrixFailure::MissingScenario { scenario }),
            1 => {}
            _ => failures.push(LiveFaultMatrixFailure::DuplicateScenario { scenario }),
        }
    }

    loaded.sort_by_key(|run| run.verification.scenario);
    let identity = loaded
        .iter()
        .find(|run| run.verification.scenario == LiveFaultScenario::CleanObserve)
        .or_else(|| loaded.first())
        .map(|run| campaign_identity(&run.report));
    if let Some(identity) = &identity {
        for run in &mut loaded {
            if campaign_identity(&run.report) != *identity {
                run.verification.identity_matches = false;
                run.verification.passed = false;
                failures.push(LiveFaultMatrixFailure::RunIdentityMismatch {
                    scenario: run.verification.scenario,
                });
            }
        }
    }
    let runs = loaded
        .into_iter()
        .map(|run| run.verification)
        .collect::<Vec<_>>();
    failures.sort_by_key(matrix_failure_rank);
    failures.dedup();
    let live_fault_matrix_passed = failures.is_empty()
        && runs.len() == LiveFaultScenario::REQUIRED.len()
        && runs.iter().all(|run| run.passed);
    Ok(LiveFaultMatrixVerificationReport {
        format_version: LIVE_FAULT_MATRIX_REPORT_FORMAT_VERSION,
        manifest: manifest_evidence,
        manifest_schema_version: manifest.schema_version,
        config: config_source,
        config_fingerprint,
        evidence_config_fingerprint,
        identity,
        runs,
        failures,
        limitations: vec![
            "typed proxy evidence proves a matched transport intervention, while response meaning, strategy causality, supervisor timing, and opaque external injector evidence still require operator review".to_string(),
            "process-death Cancel All After expiry and independent emergency cancellation require their separate certification artifacts".to_string(),
            "partial-fill coverage still requires authenticated fill/fee reconciliation for the exact run window".to_string(),
            "this matrix does not certify target-host deployment, account economics, research calibration, or production approval".to_string(),
        ],
        live_fault_matrix_passed,
    })
}

fn campaign_config_failures(config: &LiveConfig) -> Vec<LiveFaultMatrixConfigFailure> {
    let mut failures = Vec::new();
    if !config.venue.environment.is_demo() {
        failures.push(LiveFaultMatrixConfigFailure::EnvironmentIsNotDemo);
    }
    if !config.host_guard.enabled {
        failures.push(LiveFaultMatrixConfigFailure::HostGuardDisabled);
    }
    if !config.host_guard.require_clock_synchronized {
        failures.push(LiveFaultMatrixConfigFailure::SynchronizedClockNotRequired);
    }
    if !config.alerts.enabled {
        failures.push(LiveFaultMatrixConfigFailure::AlertsDisabled);
    }
    if !config.alerts.delivery_failure_is_fatal {
        failures.push(LiveFaultMatrixConfigFailure::AlertDeliveryFailureNotFatal);
    }
    if !config.operator.enabled {
        failures.push(LiveFaultMatrixConfigFailure::OperatorServiceDisabled);
    }
    if config.runtime.public_connections_per_subscription < 2 {
        failures.push(LiveFaultMatrixConfigFailure::PublicWebsocketRedundancyBelowTwo);
    }
    failures
}

fn evaluate_scenario(
    scenario: LiveFaultScenario,
    report: &LiveRunReport,
    acceptance_passed: bool,
) -> Vec<LiveFaultScenarioFailure> {
    let mut failures = Vec::new();
    match scenario {
        LiveFaultScenario::CleanObserve => {
            require_observe(report, &mut failures);
            require_clean(acceptance_passed, &mut failures);
        }
        LiveFaultScenario::CleanDemo => {
            require_demo(report, &mut failures);
            require_clean(acceptance_passed, &mut failures);
        }
        LiveFaultScenario::PublicReconnect => {
            require_live_mode(report, &mut failures);
            require_clean(acceptance_passed, &mut failures);
            if report.public_connection_disconnect_events == 0 {
                failures.push(LiveFaultScenarioFailure::PublicDisconnectMissing);
            }
        }
        LiveFaultScenario::PrivateReconnect => {
            require_live_mode(report, &mut failures);
            require_clean(acceptance_passed, &mut failures);
            if report.private_connection_disconnect_events == 0 {
                failures.push(LiveFaultScenarioFailure::PrivateDisconnectMissing);
            }
            if report.readiness_loss_count == 0 {
                failures.push(LiveFaultScenarioFailure::ReadinessLossMissing);
            }
        }
        LiveFaultScenario::OrderTransportReconnect => {
            require_demo(report, &mut failures);
            require_clean(acceptance_passed, &mut failures);
            if report.order_transport_disconnect_events == 0 {
                failures.push(LiveFaultScenarioFailure::OrderTransportDisconnectMissing);
            }
            if report.order_transport_stale_events == 0 {
                failures.push(LiveFaultScenarioFailure::OrderTransportStaleMissing);
            }
            if report.readiness_loss_count == 0 {
                failures.push(LiveFaultScenarioFailure::ReadinessLossMissing);
            }
        }
        LiveFaultScenario::AmbiguousSubmit => {
            require_demo(report, &mut failures);
            require_safe_bounded_shutdown(report, &mut failures);
            if report.ambiguous_submit_events == 0 {
                failures.push(LiveFaultScenarioFailure::AmbiguousSubmitMissing);
            }
        }
        LiveFaultScenario::AmbiguousCancel => {
            require_demo(report, &mut failures);
            require_safe_bounded_shutdown(report, &mut failures);
            if report.ambiguous_cancel_events == 0 {
                failures.push(LiveFaultScenarioFailure::AmbiguousCancelMissing);
            }
        }
        LiveFaultScenario::PartialFill => {
            require_demo(report, &mut failures);
            require_clean(acceptance_passed, &mut failures);
            if report.partial_fill_events == 0 {
                failures.push(LiveFaultScenarioFailure::PartialFillMissing);
            }
        }
        LiveFaultScenario::FillConvergenceTimeout => {
            require_demo(report, &mut failures);
            require_safe_bounded_shutdown(report, &mut failures);
            if report.fill_convergence_timeout_events == 0 {
                failures.push(LiveFaultScenarioFailure::FillConvergenceTimeoutMissing);
            }
            if report.reconciliation_drift_events == 0 {
                failures.push(LiveFaultScenarioFailure::ReconciliationDriftResponseMissing);
            }
        }
        LiveFaultScenario::OrderConvergenceTimeout => {
            require_demo(report, &mut failures);
            require_safe_bounded_shutdown(report, &mut failures);
            if report.order_convergence_timeout_events == 0 {
                failures.push(LiveFaultScenarioFailure::OrderConvergenceTimeoutMissing);
            }
            if report.reconciliation_drift_events == 0 {
                failures.push(LiveFaultScenarioFailure::ReconciliationDriftResponseMissing);
            }
        }
        LiveFaultScenario::RestoredSafetyLatch => {
            require_observe(report, &mut failures);
            require_clean(acceptance_passed, &mut failures);
            if report.restored_safety_latches == 0 {
                failures.push(LiveFaultScenarioFailure::RestoredSafetyLatchMissing);
            }
        }
        LiveFaultScenario::DeadmanHeartbeatFailure => {
            require_demo(report, &mut failures);
            require_safe_runtime_failure_shutdown(report, &mut failures);
            if report.failure.as_ref().map(|failure| failure.code.as_str())
                != Some("deadman_heartbeat")
            {
                failures.push(LiveFaultScenarioFailure::DeadmanHeartbeatFailureMissing);
            }
        }
        LiveFaultScenario::ExchangeClockFailure => {
            require_demo(report, &mut failures);
            require_safe_runtime_failure_shutdown(report, &mut failures);
            if !report.failure.as_ref().is_some_and(|failure| {
                matches!(
                    failure.code.as_str(),
                    "exchange_clock_skew" | "exchange_clock_check"
                )
            }) {
                failures.push(LiveFaultScenarioFailure::ExchangeClockFailureMissing);
            }
        }
        LiveFaultScenario::ExchangeStatusFailure => {
            require_demo(report, &mut failures);
            require_safe_runtime_failure_shutdown(report, &mut failures);
            if !report.failure.as_ref().is_some_and(|failure| {
                matches!(
                    failure.code.as_str(),
                    "exchange_status" | "exchange_status_check"
                )
            }) {
                failures.push(LiveFaultScenarioFailure::ExchangeStatusFailureMissing);
            }
        }
        LiveFaultScenario::ExchangeInstrumentFailure => {
            require_demo(report, &mut failures);
            require_safe_runtime_failure_shutdown(report, &mut failures);
            if !report.failure.as_ref().is_some_and(|failure| {
                matches!(
                    failure.code.as_str(),
                    "exchange_instrument_drift" | "exchange_instrument_check"
                )
            }) {
                failures.push(LiveFaultScenarioFailure::ExchangeInstrumentFailureMissing);
            }
        }
        LiveFaultScenario::ExchangeFeeFailure => {
            require_demo(report, &mut failures);
            require_safe_runtime_failure_shutdown(report, &mut failures);
            if !report.failure.as_ref().is_some_and(|failure| {
                matches!(
                    failure.code.as_str(),
                    "exchange_fee_drift" | "exchange_fee_check"
                )
            }) {
                failures.push(LiveFaultScenarioFailure::ExchangeFeeFailureMissing);
            }
        }
        LiveFaultScenario::AccountConfigFailure => {
            require_demo(report, &mut failures);
            require_safe_runtime_failure_shutdown(report, &mut failures);
            if !report.failure.as_ref().is_some_and(|failure| {
                matches!(
                    failure.code.as_str(),
                    "account_config_drift" | "account_config_check"
                )
            }) {
                failures.push(LiveFaultScenarioFailure::AccountConfigFailureMissing);
            }
        }
    }
    failures
}

fn require_observe(report: &LiveRunReport, failures: &mut Vec<LiveFaultScenarioFailure>) {
    if report.mode != LiveMode::Observe {
        failures.push(LiveFaultScenarioFailure::ExpectedObserveMode);
    }
}

fn require_demo(report: &LiveRunReport, failures: &mut Vec<LiveFaultScenarioFailure>) {
    if report.mode != LiveMode::Demo {
        failures.push(LiveFaultScenarioFailure::ExpectedDemoMode);
    }
}

fn require_live_mode(report: &LiveRunReport, failures: &mut Vec<LiveFaultScenarioFailure>) {
    if !matches!(report.mode, LiveMode::Observe | LiveMode::Demo) {
        failures.push(LiveFaultScenarioFailure::ExpectedObserveOrDemoMode);
    }
}

fn require_clean(acceptance_passed: bool, failures: &mut Vec<LiveFaultScenarioFailure>) {
    if !acceptance_passed {
        failures.push(LiveFaultScenarioFailure::CleanSoakRequired);
    }
}

fn require_safe_bounded_shutdown(
    report: &LiveRunReport,
    failures: &mut Vec<LiveFaultScenarioFailure>,
) {
    if report.stop_reason != LiveStopReason::DurationElapsed
        || report.failure.is_some()
        || report.elapsed_ms == 0
        || !report.reached_ready
        || !report.readiness_at_stop.is_ready()
        || report.dropped_storage_records != 0
        || report.active_orders_after_shutdown != 0
        || report.alert_delivery_failures != 0
    {
        failures.push(LiveFaultScenarioFailure::SafeBoundedShutdownRequired);
    }
}

fn require_safe_runtime_failure_shutdown(
    report: &LiveRunReport,
    failures: &mut Vec<LiveFaultScenarioFailure>,
) {
    if report.stop_reason != LiveStopReason::RuntimeFailure
        || report.failure.is_none()
        || report.elapsed_ms == 0
        || !report.reached_ready
        || report.dropped_storage_records != 0
        || report.active_orders_after_shutdown != 0
        || report.alert_delivery_failures != 0
    {
        failures.push(LiveFaultScenarioFailure::SafeRuntimeFailureShutdownRequired);
    }
}

fn campaign_identity(report: &LiveRunReport) -> LiveFaultMatrixIdentity {
    LiveFaultMatrixIdentity {
        reap_version: report.reap_version.clone(),
        executable_sha256: report.executable_sha256.clone(),
        host_identity_sha256: report.host_identity_sha256.clone().unwrap_or_default(),
        account_identity_sha256s: report.account_identity_sha256s.clone(),
    }
}

fn observed_evidence(report: &LiveRunReport) -> LiveFaultObservedEvidence {
    LiveFaultObservedEvidence {
        reached_ready: report.reached_ready,
        readiness_at_stop_ready: report.readiness_at_stop.is_ready(),
        readiness_loss_count: report.readiness_loss_count,
        reconciliation_drift_events: report.reconciliation_drift_events,
        public_connection_disconnect_events: report.public_connection_disconnect_events,
        private_connection_disconnect_events: report.private_connection_disconnect_events,
        order_transport_disconnect_events: report.order_transport_disconnect_events,
        order_transport_stale_events: report.order_transport_stale_events,
        ambiguous_submit_events: report.ambiguous_submit_events,
        ambiguous_cancel_events: report.ambiguous_cancel_events,
        partial_fill_events: report.partial_fill_events,
        fill_convergence_timeout_events: report.fill_convergence_timeout_events,
        order_convergence_timeout_events: report.order_convergence_timeout_events,
        restored_safety_latches: report.restored_safety_latches,
        dropped_storage_records: report.dropped_storage_records,
        active_orders_after_shutdown: report.active_orders_after_shutdown,
        alert_delivery_failures: report.alert_delivery_failures,
    }
}

fn resolve_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn read_bounded_regular_file(
    path: &Path,
    label: &'static str,
    limit: u64,
) -> Result<(PathBuf, Vec<u8>), LiveFaultMatrixError> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|error| LiveFaultMatrixError::InvalidPath {
            label,
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(LiveFaultMatrixError::InvalidPath {
            label,
            path: path.to_path_buf(),
            message: "must be a regular file and not a symbolic link".to_string(),
        });
    }
    let canonical =
        std::fs::canonicalize(path).map_err(|error| LiveFaultMatrixError::InvalidPath {
            label,
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    if metadata.len() > limit {
        return Err(LiveFaultMatrixError::InputTooLarge {
            label,
            path: canonical,
            actual: metadata.len(),
            limit,
        });
    }
    let bytes = std::fs::read(&canonical).map_err(|source| LiveFaultMatrixError::ReadInput {
        label,
        path: canonical.clone(),
        source,
    })?;
    if bytes.len() as u64 > limit {
        return Err(LiveFaultMatrixError::InputTooLarge {
            label,
            path: canonical,
            actual: bytes.len() as u64,
            limit,
        });
    }
    Ok((canonical, bytes))
}

fn file_evidence(path: PathBuf, bytes: &[u8]) -> LiveFaultFileEvidence {
    LiveFaultFileEvidence {
        source_path: path,
        bytes: bytes.len() as u64,
        sha256: sha256_hex(bytes),
    }
}

fn reap_fault_proxy_evidence_summary(
    bytes: &[u8],
    scenario: LiveFaultScenario,
) -> Result<Option<LiveFaultProxyEvidenceSummary>, ()> {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return Ok(None);
    };
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    let is_reap_proxy_evidence = object.contains_key("proxy_session_id")
        && object.contains_key("proxy_config_fingerprint")
        && object.contains_key("command_id")
        && object.contains_key("effects");
    if !is_reap_proxy_evidence {
        return Ok(None);
    }
    let wire: ReapFaultProxyEvidenceWire = serde_json::from_value(value).map_err(|_| ())?;
    let command_kind = wire.command.kind().to_string();
    if wire.format_version != 1
        || wire.java_reference_revision != PINNED_JAVA_REVISION
        || !valid_fault_identifier(&wire.proxy_session_id)
        || !is_sha256(&wire.proxy_config_fingerprint)
        || !valid_fault_identifier(&wire.command_id)
        || wire.completed_at_ms < wire.armed_at_ms
        || !wire.passed
        || !wire.command.is_valid()
        || !wire.command.matches_scenario(scenario)
        || !wire
            .command
            .effects_are_valid(&wire.effects, wire.armed_at_ms, wire.completed_at_ms)
    {
        return Err(());
    }
    Ok(Some(LiveFaultProxyEvidenceSummary {
        format_version: wire.format_version,
        proxy_session_id: wire.proxy_session_id,
        proxy_config_fingerprint: wire.proxy_config_fingerprint,
        command_id: wire.command_id,
        command_kind,
        armed_at_ms: wire.armed_at_ms,
        completed_at_ms: wire.completed_at_ms,
        effect_count: wire.effects.len(),
        passed: wire.passed,
    }))
}

impl ReapFaultCommandWire {
    const fn kind(&self) -> &'static str {
        match self {
            Self::DisconnectWebsockets { .. } => "disconnect_websockets",
            Self::RestResponse { .. } => "rest_response",
            Self::WebsocketDrop { .. } => "websocket_drop",
        }
    }

    fn is_valid(&self) -> bool {
        match self {
            Self::DisconnectWebsockets { connections, .. } => (1..=1024).contains(connections),
            Self::RestResponse {
                matcher,
                status,
                response_headers,
                response_body_bytes,
                response_body_sha256,
                times,
            } => {
                !matcher.method.is_empty()
                    && matcher
                        .method
                        .bytes()
                        .all(|byte| byte.is_ascii_uppercase() || byte == b'-')
                    && matcher.path.starts_with('/')
                    && !matcher.path.contains('?')
                    && !matcher.path.contains('#')
                    && matcher.query.len() <= 128
                    && response_headers.len() <= 128
                    && response_headers.iter().all(|(name, value)| {
                        !name.is_empty()
                            && name.len() <= 256
                            && !value.contains('\r')
                            && !value.contains('\n')
                            && value.len() <= 8 * 1024
                    })
                    && (200..=599).contains(status)
                    && *response_body_bytes <= 16 * 1024 * 1024
                    && is_sha256(response_body_sha256)
                    && (1..=100).contains(times)
            }
            Self::WebsocketDrop {
                matcher, frames, ..
            } => {
                (1..=100).contains(frames)
                    && matcher.json.as_ref().is_none_or(|json| {
                        matches!(
                            matcher.kind,
                            ReapFaultWebSocketFrameKind::Text | ReapFaultWebSocketFrameKind::Binary
                        ) && !json.is_empty()
                    })
            }
        }
    }

    fn matches_scenario(&self, scenario: LiveFaultScenario) -> bool {
        match scenario {
            LiveFaultScenario::CleanObserve
            | LiveFaultScenario::CleanDemo
            | LiveFaultScenario::PartialFill
            | LiveFaultScenario::RestoredSafetyLatch => false,
            LiveFaultScenario::PublicReconnect => matches!(
                self,
                Self::DisconnectWebsockets {
                    target: ReapFaultWebSocketTarget::Public,
                    ..
                }
            ),
            LiveFaultScenario::PrivateReconnect => matches!(
                self,
                Self::DisconnectWebsockets {
                    target: ReapFaultWebSocketTarget::Private,
                    ..
                }
            ),
            LiveFaultScenario::OrderTransportReconnect => matches!(
                self,
                Self::DisconnectWebsockets {
                    target: ReapFaultWebSocketTarget::Order,
                    ..
                }
            ),
            LiveFaultScenario::AmbiguousSubmit => self.matches_order_ack_drop("order"),
            LiveFaultScenario::AmbiguousCancel => self.matches_order_ack_drop("cancel-order"),
            LiveFaultScenario::FillConvergenceTimeout => {
                self.matches_private_channel_drop(&["positions", "account"])
            }
            LiveFaultScenario::OrderConvergenceTimeout => {
                self.matches_private_channel_drop(&["orders"])
            }
            LiveFaultScenario::DeadmanHeartbeatFailure => {
                self.matches_rest_response("POST", "/api/v5/trade/cancel-all-after")
            }
            LiveFaultScenario::ExchangeClockFailure => {
                self.matches_rest_response("GET", "/api/v5/public/time")
            }
            LiveFaultScenario::ExchangeStatusFailure => {
                self.matches_rest_response("GET", "/api/v5/system/status")
            }
            LiveFaultScenario::ExchangeInstrumentFailure => {
                self.matches_rest_response("GET", "/api/v5/account/instruments")
            }
            LiveFaultScenario::ExchangeFeeFailure => {
                self.matches_rest_response("GET", "/api/v5/account/trade-fee")
            }
            LiveFaultScenario::AccountConfigFailure => {
                self.matches_rest_response("GET", "/api/v5/account/config")
            }
        }
    }

    fn matches_rest_response(&self, method: &str, path: &str) -> bool {
        matches!(
            self,
            Self::RestResponse { matcher, .. }
                if matcher.method == method && matcher.path == path
        )
    }

    fn matches_private_channel_drop(&self, channels: &[&str]) -> bool {
        matches!(
            self,
            Self::WebsocketDrop {
                target: ReapFaultWebSocketTarget::Private,
                direction: ReapFaultWebSocketDirection::ExchangeToClient,
                matcher: ReapFaultWebSocketMatcherWire {
                    kind: ReapFaultWebSocketFrameKind::Text
                        | ReapFaultWebSocketFrameKind::Binary,
                    json: Some(ReapFaultWebSocketJsonMatcherWire {
                        channel: Some(channel),
                        ..
                    }),
                },
                ..
            } if channels.contains(&channel.as_str())
        )
    }

    fn matches_order_ack_drop(&self, operation: &str) -> bool {
        matches!(
            self,
            Self::WebsocketDrop {
                target: ReapFaultWebSocketTarget::Order,
                direction: ReapFaultWebSocketDirection::ExchangeToClient,
                matcher: ReapFaultWebSocketMatcherWire {
                    kind: ReapFaultWebSocketFrameKind::Text
                        | ReapFaultWebSocketFrameKind::Binary,
                    json: Some(ReapFaultWebSocketJsonMatcherWire { op: Some(op), .. }),
                },
                ..
            } if op == operation
        )
    }

    fn effects_are_valid(
        &self,
        effects: &[ReapFaultEffectWire],
        armed_at_ms: u64,
        completed_at_ms: u64,
    ) -> bool {
        let expected = match self {
            Self::DisconnectWebsockets { connections, .. } => *connections,
            Self::RestResponse { times, .. } => *times as usize,
            Self::WebsocketDrop { frames, .. } => *frames as usize,
        };
        effects.len() == expected
            && effects.iter().enumerate().all(|(index, effect)| {
                effect.sequence() == index as u32 + 1
                    && (armed_at_ms..=completed_at_ms).contains(&effect.applied_at_ms())
                    && self.matches_effect(effect)
            })
    }

    fn matches_effect(&self, effect: &ReapFaultEffectWire) -> bool {
        match (self, effect) {
            (
                Self::DisconnectWebsockets { target, .. },
                ReapFaultEffectWire::WebsocketDisconnected {
                    target: effect_target,
                    connection_id,
                    ..
                },
            ) => target == effect_target && *connection_id > 0,
            (
                Self::RestResponse { matcher, .. },
                ReapFaultEffectWire::RestResponseInjected {
                    method,
                    path,
                    query_sha256,
                    ..
                },
            ) => {
                matcher.method == *method
                    && matcher.path == *path
                    && is_sha256(query_sha256)
                    && matcher.query.len() <= 128
            }
            (
                Self::WebsocketDrop {
                    target,
                    direction,
                    matcher,
                    ..
                },
                ReapFaultEffectWire::WebsocketFrameDropped {
                    target: effect_target,
                    direction: effect_direction,
                    frame_kind,
                    connection_id,
                    frame_bytes,
                    frame_sha256,
                    ..
                },
            ) => {
                target == effect_target
                    && direction == effect_direction
                    && matcher.kind == *frame_kind
                    && *connection_id > 0
                    && *frame_bytes <= 64 * 1024 * 1024
                    && is_sha256(frame_sha256)
            }
            _ => false,
        }
    }
}

impl ReapFaultWebSocketJsonMatcherWire {
    fn is_empty(&self) -> bool {
        self.op.is_none()
            && self.event.is_none()
            && self.channel.is_none()
            && self.instrument_type.is_none()
            && self.symbol.is_none()
    }
}

impl ReapFaultEffectWire {
    const fn sequence(&self) -> u32 {
        match self {
            Self::WebsocketDisconnected { sequence, .. }
            | Self::RestResponseInjected { sequence, .. }
            | Self::WebsocketFrameDropped { sequence, .. } => *sequence,
        }
    }

    const fn applied_at_ms(&self) -> u64 {
        match self {
            Self::WebsocketDisconnected { applied_at_ms, .. }
            | Self::RestResponseInjected { applied_at_ms, .. }
            | Self::WebsocketFrameDropped { applied_at_ms, .. } => *applied_at_ms,
        }
    }
}

fn valid_fault_identifier(value: &str) -> bool {
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

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn fault_failure_rank(failure: &LiveFaultScenarioFailure) -> u8 {
    match failure {
        LiveFaultScenarioFailure::LiveReportEvidenceInvalid => 0,
        LiveFaultScenarioFailure::InjectorEvidenceMissing => 1,
        LiveFaultScenarioFailure::InjectorEvidenceInvalid => 2,
        LiveFaultScenarioFailure::ExpectedObserveMode => 3,
        LiveFaultScenarioFailure::ExpectedDemoMode => 4,
        LiveFaultScenarioFailure::ExpectedObserveOrDemoMode => 5,
        LiveFaultScenarioFailure::CleanSoakRequired => 6,
        LiveFaultScenarioFailure::SafeBoundedShutdownRequired => 7,
        LiveFaultScenarioFailure::SafeRuntimeFailureShutdownRequired => 8,
        LiveFaultScenarioFailure::PublicDisconnectMissing => 9,
        LiveFaultScenarioFailure::PrivateDisconnectMissing => 10,
        LiveFaultScenarioFailure::OrderTransportDisconnectMissing => 11,
        LiveFaultScenarioFailure::OrderTransportStaleMissing => 12,
        LiveFaultScenarioFailure::ReadinessLossMissing => 13,
        LiveFaultScenarioFailure::AmbiguousSubmitMissing => 14,
        LiveFaultScenarioFailure::AmbiguousCancelMissing => 15,
        LiveFaultScenarioFailure::PartialFillMissing => 16,
        LiveFaultScenarioFailure::FillConvergenceTimeoutMissing => 17,
        LiveFaultScenarioFailure::OrderConvergenceTimeoutMissing => 18,
        LiveFaultScenarioFailure::ReconciliationDriftResponseMissing => 19,
        LiveFaultScenarioFailure::RestoredSafetyLatchMissing => 20,
        LiveFaultScenarioFailure::DeadmanHeartbeatFailureMissing => 21,
        LiveFaultScenarioFailure::ExchangeClockFailureMissing => 22,
        LiveFaultScenarioFailure::ExchangeStatusFailureMissing => 23,
        LiveFaultScenarioFailure::ExchangeInstrumentFailureMissing => 24,
        LiveFaultScenarioFailure::ExchangeFeeFailureMissing => 25,
        LiveFaultScenarioFailure::AccountConfigFailureMissing => 26,
    }
}

fn matrix_failure_rank(failure: &LiveFaultMatrixFailure) -> (u8, LiveFaultScenario) {
    match failure {
        LiveFaultMatrixFailure::UnsupportedManifestSchema { .. } => {
            (0, LiveFaultScenario::CleanObserve)
        }
        LiveFaultMatrixFailure::ConfigGate { .. } => (1, LiveFaultScenario::CleanObserve),
        LiveFaultMatrixFailure::MissingScenario { scenario } => (2, *scenario),
        LiveFaultMatrixFailure::DuplicateScenario { scenario } => (3, *scenario),
        LiveFaultMatrixFailure::DuplicateReport { scenario } => (4, *scenario),
        LiveFaultMatrixFailure::DuplicateSession { scenario } => (5, *scenario),
        LiveFaultMatrixFailure::DuplicateInjectorEvidence { scenario } => (6, *scenario),
        LiveFaultMatrixFailure::EvidencePathCollision { scenario } => (7, *scenario),
        LiveFaultMatrixFailure::RunIdentityMismatch { scenario } => (8, *scenario),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use reap_core::PINNED_JAVA_REVISION;

    use super::*;
    use crate::{
        HostHealthSnapshot, LIVE_RUN_REPORT_SCHEMA_VERSION, LiveFailureEvidence,
        LiveLatencyEvidence, LivePhase, ReadinessSnapshot,
    };

    struct Fixture {
        _directory: tempfile::TempDir,
        config_path: PathBuf,
        manifest_path: PathBuf,
        manifest: LiveFaultMatrixManifest,
    }

    fn ready_snapshot() -> ReadinessSnapshot {
        ReadinessSnapshot {
            phase: LivePhase::Ready,
            metadata_verified: true,
            storage_ready: true,
            public_connectivity_ready: true,
            missing_reconciliation: Vec::new(),
            missing_account_snapshots: Vec::new(),
            missing_books: Vec::new(),
            missing_private_streams: Vec::new(),
            missing_order_transports: Vec::new(),
            missing_stablecoin_rates: Vec::new(),
            faults: BTreeMap::new(),
        }
    }

    fn build_fixture() -> Fixture {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("live.toml");
        let manifest_path = directory.path().join("matrix.toml");
        let mut config =
            LiveConfig::from_toml(include_str!("../../../examples/live-okx-demo.toml")).unwrap();
        config.host_guard.enabled = true;
        config.alerts.enabled = true;
        config.ensure_valid().unwrap();
        let config_bytes = toml::to_string_pretty(&config).unwrap().into_bytes();
        std::fs::write(&config_path, &config_bytes).unwrap();
        let config_source = LiveConfigFileEvidence {
            source_path: std::fs::canonicalize(&config_path).unwrap(),
            bytes: config_bytes.len() as u64,
            sha256: sha256_hex(&config_bytes),
        };
        let fingerprint = config.fingerprint().unwrap();
        let evidence_fingerprint = config.evidence_fingerprint().unwrap();
        let mut runs = Vec::new();
        for (index, scenario) in LiveFaultScenario::REQUIRED.into_iter().enumerate() {
            let report_path = directory.path().join(format!("{scenario:?}.json"));
            let report = scenario_report(
                scenario,
                index as u64,
                &config,
                config_source.clone(),
                &fingerprint,
                &evidence_fingerprint,
            );
            std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
            let injector_evidence = scenario.requires_injector_evidence().then(|| {
                let path = directory.path().join(format!("{scenario:?}-injector.log"));
                std::fs::write(&path, format!("injected {scenario:?}\n")).unwrap();
                path
            });
            runs.push(LiveFaultMatrixRunManifest {
                scenario,
                report: report_path,
                injector_evidence,
            });
        }
        let manifest = LiveFaultMatrixManifest {
            schema_version: LIVE_FAULT_MATRIX_MANIFEST_SCHEMA_VERSION,
            runs,
        };
        write_manifest(&manifest_path, &manifest);
        Fixture {
            _directory: directory,
            config_path,
            manifest_path,
            manifest,
        }
    }

    fn scenario_report(
        scenario: LiveFaultScenario,
        index: u64,
        config: &LiveConfig,
        config_source: LiveConfigFileEvidence,
        fingerprint: &str,
        evidence_fingerprint: &str,
    ) -> LiveRunReport {
        let started_at_ms = 10_000 + index * 20_000;
        let host = HostHealthSnapshot {
            checked_at_ms: started_at_ms + 1,
            disk_available_bytes: config.host_guard.min_disk_available_bytes,
            memory_available_bytes: config.host_guard.min_memory_available_bytes,
            clock_synchronized: true,
        };
        let ready = ready_snapshot();
        let mut report = LiveRunReport {
            schema_version: LIVE_RUN_REPORT_SCHEMA_VERSION,
            session_id: Some(format!("session-{index}")),
            session_started_at_ms: started_at_ms,
            config_source: Some(config_source),
            config_fingerprint: fingerprint.to_string(),
            evidence_config_fingerprint: evidence_fingerprint.to_string(),
            java_reference_revision: PINNED_JAVA_REVISION.to_string(),
            reap_version: env!("CARGO_PKG_VERSION").to_string(),
            executable_sha256: "1".repeat(64),
            host_identity_sha256: Some("2".repeat(64)),
            account_identity_sha256s: BTreeMap::from([("main".to_string(), "3".repeat(64))]),
            mode: LiveMode::Demo,
            stop_reason: LiveStopReason::DurationElapsed,
            failure: None,
            elapsed_ms: 10_000,
            reached_ready: true,
            time_to_ready_ms: Some(100),
            readiness_loss_count: 0,
            max_readiness_outage_ms: 0,
            reconciliation_drift_events: 0,
            book_recovery_events: 0,
            stream_stale_events: 0,
            connection_disconnect_events: 0,
            public_connection_disconnect_events: 0,
            private_connection_disconnect_events: 0,
            order_transport_disconnect_events: 0,
            order_transport_stale_events: 0,
            ambiguous_submit_events: 0,
            ambiguous_cancel_events: 0,
            partial_fill_events: 0,
            fill_convergence_timeout_events: 0,
            order_convergence_timeout_events: 0,
            restored_safety_latches: 0,
            operator_commands: 0,
            operator_mutations: 0,
            max_storage_queue_depth: 1,
            alerts_delivered: 1,
            alert_delivery_failures: 0,
            alert_failure_notifications_dropped: 0,
            max_alert_queue_depth: 1,
            host_preflight: Some(host.clone()),
            host_checks: 2,
            host_last_snapshot: Some(HostHealthSnapshot {
                checked_at_ms: started_at_ms + 9_000,
                ..host
            }),
            readiness_at_stop: ready.clone(),
            readiness: ready,
            dropped_storage_records: 0,
            active_orders_after_shutdown: 0,
            latency_evidence: LiveLatencyEvidence::default(),
            clean_soak: true,
        };
        match scenario {
            LiveFaultScenario::CleanObserve => report.mode = LiveMode::Observe,
            LiveFaultScenario::CleanDemo => {}
            LiveFaultScenario::PublicReconnect => {
                report.mode = LiveMode::Observe;
                report.public_connection_disconnect_events = 1;
            }
            LiveFaultScenario::PrivateReconnect => {
                report.mode = LiveMode::Observe;
                report.private_connection_disconnect_events = 1;
                report.readiness_loss_count = 1;
                report.max_readiness_outage_ms = 250;
            }
            LiveFaultScenario::OrderTransportReconnect => {
                report.order_transport_disconnect_events = 1;
                report.order_transport_stale_events = 1;
                report.readiness_loss_count = 1;
                report.max_readiness_outage_ms = 250;
            }
            LiveFaultScenario::AmbiguousSubmit => {
                report.ambiguous_submit_events = 1;
                report.reconciliation_drift_events = 1;
            }
            LiveFaultScenario::AmbiguousCancel => {
                report.ambiguous_cancel_events = 1;
                report.reconciliation_drift_events = 1;
            }
            LiveFaultScenario::PartialFill => report.partial_fill_events = 1,
            LiveFaultScenario::FillConvergenceTimeout => {
                report.fill_convergence_timeout_events = 1;
                report.reconciliation_drift_events = 1;
            }
            LiveFaultScenario::OrderConvergenceTimeout => {
                report.order_convergence_timeout_events = 1;
                report.reconciliation_drift_events = 1;
            }
            LiveFaultScenario::RestoredSafetyLatch => {
                report.mode = LiveMode::Observe;
                report.restored_safety_latches = 1;
            }
            LiveFaultScenario::DeadmanHeartbeatFailure => {
                set_runtime_failure(&mut report, "deadman_heartbeat");
            }
            LiveFaultScenario::ExchangeClockFailure => {
                set_runtime_failure(&mut report, "exchange_clock_skew");
            }
            LiveFaultScenario::ExchangeStatusFailure => {
                set_runtime_failure(&mut report, "exchange_status");
            }
            LiveFaultScenario::ExchangeInstrumentFailure => {
                set_runtime_failure(&mut report, "exchange_instrument_drift");
            }
            LiveFaultScenario::ExchangeFeeFailure => {
                set_runtime_failure(&mut report, "exchange_fee_drift");
            }
            LiveFaultScenario::AccountConfigFailure => {
                set_runtime_failure(&mut report, "account_config_drift");
            }
        }
        report.connection_disconnect_events = report
            .public_connection_disconnect_events
            .saturating_add(report.private_connection_disconnect_events)
            .saturating_add(report.order_transport_disconnect_events);
        report.clean_soak = report.stop_reason == LiveStopReason::DurationElapsed
            && report.reached_ready
            && report.readiness_at_stop.is_ready()
            && report.reconciliation_drift_events == 0
            && report.operator_mutations == 0
            && report.dropped_storage_records == 0
            && report.active_orders_after_shutdown == 0
            && report.alert_delivery_failures == 0;
        report
    }

    fn set_runtime_failure(report: &mut LiveRunReport, code: &str) {
        report.stop_reason = LiveStopReason::RuntimeFailure;
        report.failure = Some(LiveFailureEvidence {
            code: code.to_string(),
            message: "injected campaign failure".to_string(),
        });
    }

    fn write_manifest(path: &Path, manifest: &LiveFaultMatrixManifest) {
        std::fs::write(path, toml::to_string_pretty(manifest).unwrap()).unwrap();
    }

    #[test]
    fn checked_in_fault_matrix_template_covers_every_required_role() {
        let manifest: LiveFaultMatrixManifest =
            toml::from_str(include_str!("../../../examples/live-fault-matrix.toml")).unwrap();
        let roles = manifest
            .runs
            .iter()
            .map(|run| run.scenario)
            .collect::<BTreeSet<_>>();

        assert_eq!(roles, LiveFaultScenario::REQUIRED.into_iter().collect(),);
        assert_eq!(manifest.runs.len(), LiveFaultScenario::REQUIRED.len());
    }

    #[test]
    fn typed_proxy_commands_match_only_their_supported_scenario() {
        let body_sha256 = "a".repeat(64);
        let cases = [
            (
                LiveFaultScenario::PublicReconnect,
                serde_json::json!({
                    "kind": "disconnect_websockets",
                    "target": "public",
                    "connections": 1
                }),
            ),
            (
                LiveFaultScenario::PrivateReconnect,
                serde_json::json!({
                    "kind": "disconnect_websockets",
                    "target": "private",
                    "connections": 1
                }),
            ),
            (
                LiveFaultScenario::OrderTransportReconnect,
                serde_json::json!({
                    "kind": "disconnect_websockets",
                    "target": "order",
                    "connections": 1
                }),
            ),
            (
                LiveFaultScenario::AmbiguousSubmit,
                websocket_drop_command("order", "op", "order"),
            ),
            (
                LiveFaultScenario::AmbiguousCancel,
                websocket_drop_command("order", "op", "cancel-order"),
            ),
            (
                LiveFaultScenario::FillConvergenceTimeout,
                websocket_drop_command("private", "channel", "positions"),
            ),
            (
                LiveFaultScenario::OrderConvergenceTimeout,
                websocket_drop_command("private", "channel", "orders"),
            ),
            (
                LiveFaultScenario::DeadmanHeartbeatFailure,
                rest_failure_command("POST", "/api/v5/trade/cancel-all-after", &body_sha256),
            ),
            (
                LiveFaultScenario::ExchangeClockFailure,
                rest_failure_command("GET", "/api/v5/public/time", &body_sha256),
            ),
            (
                LiveFaultScenario::ExchangeStatusFailure,
                rest_failure_command("GET", "/api/v5/system/status", &body_sha256),
            ),
            (
                LiveFaultScenario::ExchangeInstrumentFailure,
                rest_failure_command("GET", "/api/v5/account/instruments", &body_sha256),
            ),
            (
                LiveFaultScenario::ExchangeFeeFailure,
                rest_failure_command("GET", "/api/v5/account/trade-fee", &body_sha256),
            ),
            (
                LiveFaultScenario::AccountConfigFailure,
                rest_failure_command("GET", "/api/v5/account/config", &body_sha256),
            ),
        ];

        for (scenario, value) in cases {
            let command: ReapFaultCommandWire = serde_json::from_value(value).unwrap();
            assert!(command.is_valid(), "invalid command for {scenario:?}");
            for candidate in LiveFaultScenario::REQUIRED {
                assert_eq!(
                    command.matches_scenario(candidate),
                    candidate == scenario,
                    "{scenario:?} command was misclassified as {candidate:?}"
                );
            }
        }

        let spot_fill: ReapFaultCommandWire =
            serde_json::from_value(websocket_drop_command("private", "channel", "account"))
                .unwrap();
        assert!(spot_fill.matches_scenario(LiveFaultScenario::FillConvergenceTimeout));

        let drifted_clock_response: ReapFaultCommandWire =
            serde_json::from_value(serde_json::json!({
                "kind": "rest_response",
                "matcher": {"method": "GET", "path": "/api/v5/public/time"},
                "status": 200,
                "response_headers": {},
                "response_body_bytes": 2,
                "response_body_sha256": body_sha256,
                "times": 1
            }))
            .unwrap();
        assert!(drifted_clock_response.matches_scenario(LiveFaultScenario::ExchangeClockFailure));
    }

    fn websocket_drop_command(target: &str, matcher_key: &str, matcher_value: &str) -> Value {
        let json_matcher = match matcher_key {
            "op" => serde_json::json!({"op": matcher_value}),
            "channel" => serde_json::json!({"channel": matcher_value}),
            _ => panic!("unsupported matcher key {matcher_key}"),
        };
        serde_json::json!({
            "kind": "websocket_drop",
            "target": target,
            "direction": "exchange_to_client",
            "matcher": {
                "kind": "text",
                "json": json_matcher
            },
            "frames": 1
        })
    }

    fn rest_failure_command(method: &str, path: &str, body_sha256: &str) -> Value {
        serde_json::json!({
            "kind": "rest_response",
            "matcher": {"method": method, "path": path},
            "status": 503,
            "response_headers": {"content-type": "application/json"},
            "response_body_bytes": 2,
            "response_body_sha256": body_sha256,
            "times": 1
        })
    }

    #[test]
    fn complete_fault_matrix_passes_with_bound_identity_and_injectors() {
        let fixture = build_fixture();

        let report =
            verify_live_fault_matrix_paths(&fixture.config_path, &fixture.manifest_path).unwrap();

        assert!(report.live_fault_matrix_passed, "{report:#?}");
        assert_eq!(report.runs.len(), LiveFaultScenario::REQUIRED.len());
        assert!(report.runs.iter().all(|run| run.passed));
        assert!(
            report
                .runs
                .iter()
                .filter(|run| run.scenario.requires_injector_evidence())
                .all(|run| run.injector_evidence.is_some())
        );
    }

    #[test]
    fn fault_matrix_validates_typed_reap_proxy_evidence() {
        let fixture = build_fixture();
        let injector = fixture
            .manifest
            .runs
            .iter()
            .find(|run| run.scenario == LiveFaultScenario::PublicReconnect)
            .unwrap()
            .injector_evidence
            .clone()
            .unwrap();
        let evidence = serde_json::json!({
            "format_version": 1,
            "proxy_session_id": "proxy-session",
            "proxy_config_fingerprint": "a".repeat(64),
            "java_reference_revision": PINNED_JAVA_REVISION,
            "command_id": "public-reconnect",
            "command": {
                "kind": "disconnect_websockets",
                "target": "public",
                "connections": 1
            },
            "armed_at_ms": 100,
            "completed_at_ms": 101,
            "effects": [{
                "kind": "websocket_disconnected",
                "sequence": 1,
                "applied_at_ms": 100,
                "connection_id": 1,
                "target": "public"
            }],
            "passed": true
        });
        std::fs::write(&injector, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();

        let report =
            verify_live_fault_matrix_paths(&fixture.config_path, &fixture.manifest_path).unwrap();

        assert!(report.live_fault_matrix_passed, "{report:#?}");
        let public = report
            .runs
            .iter()
            .find(|run| run.scenario == LiveFaultScenario::PublicReconnect)
            .unwrap();
        let proxy = public.reap_fault_proxy_evidence.as_ref().unwrap();
        assert_eq!(proxy.command_id, "public-reconnect");
        assert_eq!(proxy.command_kind, "disconnect_websockets");
        assert_eq!(proxy.armed_at_ms, 100);
        assert_eq!(proxy.completed_at_ms, 101);
        assert_eq!(proxy.effect_count, 1);
    }

    #[test]
    fn fault_matrix_validates_typed_runtime_endpoint_evidence() {
        let fixture = build_fixture();
        let injector = fixture
            .manifest
            .runs
            .iter()
            .find(|run| run.scenario == LiveFaultScenario::ExchangeStatusFailure)
            .unwrap()
            .injector_evidence
            .clone()
            .unwrap();
        let evidence = serde_json::json!({
            "format_version": 1,
            "proxy_session_id": "proxy-session",
            "proxy_config_fingerprint": "a".repeat(64),
            "java_reference_revision": PINNED_JAVA_REVISION,
            "command_id": "exchange-status-failure",
            "command": {
                "kind": "rest_response",
                "matcher": {
                    "method": "GET",
                    "path": "/api/v5/system/status"
                },
                "status": 503,
                "response_headers": {"content-type": "application/json"},
                "response_body_bytes": 2,
                "response_body_sha256": "b".repeat(64),
                "times": 1
            },
            "armed_at_ms": 100,
            "completed_at_ms": 101,
            "effects": [{
                "kind": "rest_response_injected",
                "sequence": 1,
                "applied_at_ms": 100,
                "method": "GET",
                "path": "/api/v5/system/status",
                "query_sha256": "c".repeat(64)
            }],
            "passed": true
        });
        std::fs::write(&injector, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();

        let report =
            verify_live_fault_matrix_paths(&fixture.config_path, &fixture.manifest_path).unwrap();

        assert!(report.live_fault_matrix_passed, "{report:#?}");
        let status = report
            .runs
            .iter()
            .find(|run| run.scenario == LiveFaultScenario::ExchangeStatusFailure)
            .unwrap();
        let proxy = status.reap_fault_proxy_evidence.as_ref().unwrap();
        assert_eq!(proxy.command_id, "exchange-status-failure");
        assert_eq!(proxy.command_kind, "rest_response");
        assert_eq!(proxy.armed_at_ms, 100);
        assert_eq!(proxy.completed_at_ms, 101);
        assert_eq!(proxy.effect_count, 1);
    }

    #[test]
    fn fault_matrix_rejects_typed_proxy_evidence_for_a_different_scenario() {
        let fixture = build_fixture();
        let injector = fixture
            .manifest
            .runs
            .iter()
            .find(|run| run.scenario == LiveFaultScenario::PublicReconnect)
            .unwrap()
            .injector_evidence
            .clone()
            .unwrap();
        let evidence = serde_json::json!({
            "format_version": 1,
            "proxy_session_id": "proxy-session",
            "proxy_config_fingerprint": "a".repeat(64),
            "java_reference_revision": PINNED_JAVA_REVISION,
            "command_id": "wrong-reconnect",
            "command": {
                "kind": "disconnect_websockets",
                "target": "private",
                "connections": 1
            },
            "armed_at_ms": 100,
            "completed_at_ms": 101,
            "effects": [{
                "kind": "websocket_disconnected",
                "sequence": 1,
                "applied_at_ms": 100,
                "connection_id": 1,
                "target": "private"
            }],
            "passed": true
        });
        std::fs::write(&injector, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();

        let report =
            verify_live_fault_matrix_paths(&fixture.config_path, &fixture.manifest_path).unwrap();

        assert!(!report.live_fault_matrix_passed);
        let public = report
            .runs
            .iter()
            .find(|run| run.scenario == LiveFaultScenario::PublicReconnect)
            .unwrap();
        assert!(
            public
                .scenario_failures
                .contains(&LiveFaultScenarioFailure::InjectorEvidenceInvalid)
        );
    }

    #[test]
    fn fault_matrix_rejects_failed_typed_reap_proxy_evidence() {
        let fixture = build_fixture();
        let injector = fixture
            .manifest
            .runs
            .iter()
            .find(|run| run.scenario == LiveFaultScenario::OrderTransportReconnect)
            .unwrap()
            .injector_evidence
            .clone()
            .unwrap();
        let evidence = serde_json::json!({
            "format_version": 1,
            "proxy_session_id": "proxy-session",
            "proxy_config_fingerprint": "a".repeat(64),
            "java_reference_revision": PINNED_JAVA_REVISION,
            "command_id": "order-reconnect",
            "command": {
                "kind": "disconnect_websockets",
                "target": "order",
                "connections": 1
            },
            "armed_at_ms": 100,
            "completed_at_ms": 101,
            "effects": [{
                "kind": "websocket_disconnected",
                "sequence": 1,
                "applied_at_ms": 100,
                "connection_id": 1,
                "target": "order"
            }],
            "passed": false
        });
        std::fs::write(&injector, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();

        let report =
            verify_live_fault_matrix_paths(&fixture.config_path, &fixture.manifest_path).unwrap();

        assert!(!report.live_fault_matrix_passed);
        let order = report
            .runs
            .iter()
            .find(|run| run.scenario == LiveFaultScenario::OrderTransportReconnect)
            .unwrap();
        assert!(
            order
                .scenario_failures
                .contains(&LiveFaultScenarioFailure::InjectorEvidenceInvalid)
        );
    }

    #[test]
    fn fault_matrix_rejects_a_different_typed_exchange_failure() {
        let fixture = build_fixture();
        let instrument_path = fixture
            .manifest
            .runs
            .iter()
            .find(|run| run.scenario == LiveFaultScenario::ExchangeInstrumentFailure)
            .unwrap()
            .report
            .clone();
        let mut instrument: LiveRunReport =
            serde_json::from_slice(&std::fs::read(&instrument_path).unwrap()).unwrap();
        instrument.failure.as_mut().unwrap().code = "exchange_fee_drift".to_string();
        std::fs::write(
            &instrument_path,
            serde_json::to_vec_pretty(&instrument).unwrap(),
        )
        .unwrap();

        let report =
            verify_live_fault_matrix_paths(&fixture.config_path, &fixture.manifest_path).unwrap();

        assert!(!report.live_fault_matrix_passed);
        let instrument = report
            .runs
            .iter()
            .find(|run| run.scenario == LiveFaultScenario::ExchangeInstrumentFailure)
            .unwrap();
        assert!(
            instrument
                .scenario_failures
                .contains(&LiveFaultScenarioFailure::ExchangeInstrumentFailureMissing)
        );
    }

    #[test]
    fn fault_matrix_rejects_a_missing_role_and_injector() {
        let mut fixture = build_fixture();
        fixture
            .manifest
            .runs
            .retain(|run| run.scenario != LiveFaultScenario::PartialFill);
        let public = fixture
            .manifest
            .runs
            .iter_mut()
            .find(|run| run.scenario == LiveFaultScenario::PublicReconnect)
            .unwrap();
        public.injector_evidence = None;
        write_manifest(&fixture.manifest_path, &fixture.manifest);

        let report =
            verify_live_fault_matrix_paths(&fixture.config_path, &fixture.manifest_path).unwrap();

        assert!(!report.live_fault_matrix_passed);
        assert!(
            report
                .failures
                .contains(&LiveFaultMatrixFailure::MissingScenario {
                    scenario: LiveFaultScenario::PartialFill,
                })
        );
        let public = report
            .runs
            .iter()
            .find(|run| run.scenario == LiveFaultScenario::PublicReconnect)
            .unwrap();
        assert!(
            public
                .scenario_failures
                .contains(&LiveFaultScenarioFailure::InjectorEvidenceMissing)
        );
    }

    #[test]
    fn fault_matrix_rejects_cross_run_account_identity_drift() {
        let fixture = build_fixture();
        let partial_path = fixture
            .manifest
            .runs
            .iter()
            .find(|run| run.scenario == LiveFaultScenario::PartialFill)
            .unwrap()
            .report
            .clone();
        let mut partial: LiveRunReport =
            serde_json::from_slice(&std::fs::read(&partial_path).unwrap()).unwrap();
        partial
            .account_identity_sha256s
            .insert("main".to_string(), "4".repeat(64));
        std::fs::write(&partial_path, serde_json::to_vec_pretty(&partial).unwrap()).unwrap();

        let report =
            verify_live_fault_matrix_paths(&fixture.config_path, &fixture.manifest_path).unwrap();

        assert!(!report.live_fault_matrix_passed);
        assert!(
            report
                .failures
                .contains(&LiveFaultMatrixFailure::RunIdentityMismatch {
                    scenario: LiveFaultScenario::PartialFill,
                })
        );
    }

    #[test]
    fn fault_matrix_rejects_an_artifact_reused_as_injector_evidence() {
        let mut fixture = build_fixture();
        let private_report = fixture
            .manifest
            .runs
            .iter()
            .find(|run| run.scenario == LiveFaultScenario::PrivateReconnect)
            .unwrap()
            .report
            .clone();
        fixture
            .manifest
            .runs
            .iter_mut()
            .find(|run| run.scenario == LiveFaultScenario::PublicReconnect)
            .unwrap()
            .injector_evidence = Some(private_report);
        write_manifest(&fixture.manifest_path, &fixture.manifest);

        let report =
            verify_live_fault_matrix_paths(&fixture.config_path, &fixture.manifest_path).unwrap();

        assert!(!report.live_fault_matrix_passed);
        assert!(report.failures.iter().any(|failure| matches!(
            failure,
            LiveFaultMatrixFailure::EvidencePathCollision { .. }
        )));
    }

    #[test]
    fn fault_matrix_rejects_copied_injector_content() {
        let fixture = build_fixture();
        let injector = |scenario| {
            fixture
                .manifest
                .runs
                .iter()
                .find(|run| run.scenario == scenario)
                .unwrap()
                .injector_evidence
                .clone()
                .unwrap()
        };
        let public = injector(LiveFaultScenario::PublicReconnect);
        let private = injector(LiveFaultScenario::PrivateReconnect);
        std::fs::write(&private, std::fs::read(public).unwrap()).unwrap();

        let report =
            verify_live_fault_matrix_paths(&fixture.config_path, &fixture.manifest_path).unwrap();

        assert!(!report.live_fault_matrix_passed);
        assert!(
            report
                .failures
                .contains(&LiveFaultMatrixFailure::DuplicateInjectorEvidence {
                    scenario: LiveFaultScenario::PrivateReconnect,
                })
        );
    }

    #[test]
    fn fault_matrix_rejects_an_unrecovered_bounded_fault() {
        let fixture = build_fixture();
        let ambiguous_path = fixture
            .manifest
            .runs
            .iter()
            .find(|run| run.scenario == LiveFaultScenario::AmbiguousSubmit)
            .unwrap()
            .report
            .clone();
        let mut ambiguous: LiveRunReport =
            serde_json::from_slice(&std::fs::read(&ambiguous_path).unwrap()).unwrap();
        ambiguous.readiness_at_stop.phase = LivePhase::Degraded;
        std::fs::write(
            &ambiguous_path,
            serde_json::to_vec_pretty(&ambiguous).unwrap(),
        )
        .unwrap();

        let report =
            verify_live_fault_matrix_paths(&fixture.config_path, &fixture.manifest_path).unwrap();

        assert!(!report.live_fault_matrix_passed);
        let ambiguous = report
            .runs
            .iter()
            .find(|run| run.scenario == LiveFaultScenario::AmbiguousSubmit)
            .unwrap();
        assert!(
            ambiguous
                .scenario_failures
                .contains(&LiveFaultScenarioFailure::SafeBoundedShutdownRequired)
        );
    }
}
