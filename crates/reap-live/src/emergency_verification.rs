use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use reap_core::PINNED_JAVA_REVISION;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::emergency::{
    ACCOUNT_WIDE_ORDER_SCOPE, EXCLUDED_ORDER_CLASSES, MAX_INCIDENT_MESSAGE_BYTES, MAX_INCIDENTS,
    MAX_REMAINING_ORDER_DETAILS, review_emergency_config,
};
use crate::{
    EMERGENCY_CANCEL_REPORT_SCHEMA_VERSION, EmergencyAccountReport, EmergencyCancelReport,
    TradingEnvironment,
};

pub const EMERGENCY_CANCEL_VERIFICATION_FORMAT_VERSION: u16 = 3;
pub const MAX_EMERGENCY_CANCEL_CONFIG_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_EMERGENCY_CANCEL_REPORT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EmergencyCancelVerificationOptions {
    pub require_all_configured_accounts: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmergencyCancelFileEvidence {
    pub source_path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum EmergencyCancelVerificationFailure {
    UnsupportedReportSchema {
        actual: u32,
        supported: u32,
    },
    ConfigFileMismatch,
    JavaRevisionMismatch,
    EnvironmentMismatch {
        configured: TradingEnvironment,
        reported: TradingEnvironment,
    },
    ScopeMismatch,
    ExcludedOrderClassesMismatch,
    InvalidProvenance {
        message: String,
    },
    ConfiguredAccountSetInvalid {
        message: String,
    },
    EmergencyConfigurationInvalid {
        message: String,
    },
    SelectedAccountSetInvalid {
        message: String,
    },
    AccountCoverageMismatch,
    ReportInvariant {
        message: String,
    },
    AccountInvariant {
        account_id: String,
        message: String,
    },
    RegularOrdersAllClearMismatch {
        reported: bool,
        derived: bool,
    },
    AlgoOrdersAllClearMismatch {
        reported: bool,
        derived: bool,
    },
    SpreadOrdersAllClearMismatch {
        reported: bool,
        derived: bool,
    },
    AccountWideOrdersAllClearMismatch {
        reported: bool,
        derived: bool,
    },
    EvidenceCompleteMismatch {
        reported: bool,
        derived: bool,
    },
    AllClearMismatch {
        reported: bool,
        derived: bool,
    },
    AllConfiguredAccountsRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmergencyCancelVerificationReport {
    pub format_version: u16,
    pub config: EmergencyCancelFileEvidence,
    pub emergency_report: EmergencyCancelFileEvidence,
    pub report_schema_version: u32,
    pub report_id: String,
    pub java_reference_revision: String,
    pub reap_version: String,
    pub executable_sha256: Option<String>,
    pub host_identity_sha256: Option<String>,
    pub environment: TradingEnvironment,
    pub started_at_ms: u64,
    pub elapsed_ms: u64,
    pub configured_accounts: Vec<String>,
    pub selected_accounts: Vec<String>,
    pub account_identity_sha256s: BTreeMap<String, String>,
    pub require_all_configured_accounts: bool,
    pub all_configured_accounts_selected: bool,
    pub reported_regular_orders_all_clear: bool,
    pub derived_regular_orders_all_clear: bool,
    pub reported_algo_orders_all_clear: bool,
    pub derived_algo_orders_all_clear: bool,
    pub reported_spread_orders_all_clear: bool,
    pub derived_spread_orders_all_clear: bool,
    pub reported_account_wide_orders_all_clear: bool,
    pub derived_account_wide_orders_all_clear: bool,
    pub reported_evidence_complete: bool,
    pub derived_evidence_complete: bool,
    pub reported_all_clear: bool,
    pub derived_all_clear: bool,
    pub failures: Vec<EmergencyCancelVerificationFailure>,
    pub limitations: Vec<String>,
    pub evidence_valid: bool,
    pub acceptance_passed: bool,
}

#[derive(Debug, Error)]
pub enum EmergencyCancelVerificationError {
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
    #[error("{label} {path} is not UTF-8: {source}")]
    InvalidUtf8 {
        label: &'static str,
        path: PathBuf,
        #[source]
        source: std::str::Utf8Error,
    },
    #[error("failed to parse emergency config {path}: {source}")]
    ParseConfig {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to parse emergency cancel report {path}: {source}")]
    ParseReport {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("emergency config and report resolve to the same file {0}")]
    PathCollision(PathBuf),
}

pub fn verify_emergency_cancel_paths(
    config_path: impl AsRef<Path>,
    report_path: impl AsRef<Path>,
    options: EmergencyCancelVerificationOptions,
) -> Result<EmergencyCancelVerificationReport, EmergencyCancelVerificationError> {
    let (config_path, config_bytes) = read_bounded_regular_file(
        config_path.as_ref(),
        "emergency config",
        MAX_EMERGENCY_CANCEL_CONFIG_BYTES,
    )?;
    let config_text = std::str::from_utf8(&config_bytes).map_err(|source| {
        EmergencyCancelVerificationError::InvalidUtf8 {
            label: "emergency config",
            path: config_path.clone(),
            source,
        }
    })?;
    let (report_path, report_bytes) = read_bounded_regular_file(
        report_path.as_ref(),
        "emergency cancel report",
        MAX_EMERGENCY_CANCEL_REPORT_BYTES,
    )?;
    if config_path == report_path {
        return Err(EmergencyCancelVerificationError::PathCollision(config_path));
    }
    let report: EmergencyCancelReport =
        serde_json::from_slice(&report_bytes).map_err(|source| {
            EmergencyCancelVerificationError::ParseReport {
                path: report_path.clone(),
                source,
            }
        })?;
    let config_review = review_emergency_config(
        config_text,
        &report.selected_accounts,
        report.account_timeout_ms,
        report.poll_interval_ms,
        report.deadman_timeout_secs,
    )
    .map_err(|source| EmergencyCancelVerificationError::ParseConfig {
        path: config_path.clone(),
        source,
    })?;
    let config = file_evidence(config_path, &config_bytes);
    let emergency_report = file_evidence(report_path, &report_bytes);
    let mut failures = Vec::new();

    let schema_matches = report.schema_version == EMERGENCY_CANCEL_REPORT_SCHEMA_VERSION;
    if !schema_matches {
        failures.push(
            EmergencyCancelVerificationFailure::UnsupportedReportSchema {
                actual: report.schema_version,
                supported: EMERGENCY_CANCEL_REPORT_SCHEMA_VERSION,
            },
        );
    }
    let config_file_matches = report.config_file_sha256 == config.sha256;
    if !config_file_matches {
        failures.push(EmergencyCancelVerificationFailure::ConfigFileMismatch);
    }
    let java_revision_matches = report.java_reference_revision == PINNED_JAVA_REVISION;
    if !java_revision_matches {
        failures.push(EmergencyCancelVerificationFailure::JavaRevisionMismatch);
    }
    let environment_matches = report.environment == config_review.environment;
    if !environment_matches {
        failures.push(EmergencyCancelVerificationFailure::EnvironmentMismatch {
            configured: config_review.environment,
            reported: report.environment,
        });
    }
    let emergency_configuration_valid = config_review.validation_error.is_none();
    if let Some(message) = &config_review.validation_error {
        failures.push(
            EmergencyCancelVerificationFailure::EmergencyConfigurationInvalid {
                message: message.clone(),
            },
        );
    }
    let scope_matches = report.scope == ACCOUNT_WIDE_ORDER_SCOPE;
    if !scope_matches {
        failures.push(EmergencyCancelVerificationFailure::ScopeMismatch);
    }
    let expected_excluded = EXCLUDED_ORDER_CLASSES
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let excluded_order_classes_match = report.excluded_order_classes == expected_excluded;
    if !excluded_order_classes_match {
        failures.push(EmergencyCancelVerificationFailure::ExcludedOrderClassesMismatch);
    }

    validate_report_provenance(&report, &mut failures);
    validate_report_shape(&report, &mut failures);

    let (configured_accounts, configured_accounts_valid) =
        normalized_account_ids(&config_review.account_ids);
    if configured_accounts.is_empty() || !configured_accounts_valid {
        failures.push(
            EmergencyCancelVerificationFailure::ConfiguredAccountSetInvalid {
                message: "configured account ids must be non-empty and unique".to_string(),
            },
        );
    }
    let (selected_accounts, selected_accounts_valid) =
        normalized_account_ids(&report.selected_accounts);
    if report.selected_accounts.is_empty()
        || !selected_accounts_valid
        || report.selected_accounts != selected_accounts
    {
        failures.push(
            EmergencyCancelVerificationFailure::SelectedAccountSetInvalid {
                message: "selected account ids must be non-empty, unique, and sorted".to_string(),
            },
        );
    }
    let configured_set = configured_accounts.iter().collect::<BTreeSet<_>>();
    for account_id in &selected_accounts {
        if !configured_set.contains(account_id) {
            failures.push(
                EmergencyCancelVerificationFailure::SelectedAccountSetInvalid {
                    message: format!("selected account {account_id} is not configured"),
                },
            );
        }
    }

    let reported_account_ids = report
        .accounts
        .iter()
        .map(|account| account.account_id.clone())
        .collect::<Vec<_>>();
    let account_coverage_complete = reported_account_ids == report.selected_accounts;
    if !account_coverage_complete {
        failures.push(EmergencyCancelVerificationFailure::AccountCoverageMismatch);
    }
    for account in &report.accounts {
        validate_account_shape(
            account,
            report.deadman_timeout_secs,
            report.account_timeout_ms,
            report.elapsed_ms,
            &mut failures,
        );
    }

    let account_identity_sha256s = report
        .accounts
        .iter()
        .filter_map(|account| {
            account
                .account_identity_sha256
                .as_ref()
                .map(|identity| (account.account_id.clone(), identity.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let derived_regular_orders_all_clear = account_coverage_complete
        && !report.accounts.is_empty()
        && report
            .accounts
            .iter()
            .all(|account| derived_regular_all_clear(account, report.deadman_timeout_secs));
    let derived_algo_orders_all_clear = account_coverage_complete
        && !report.accounts.is_empty()
        && report
            .accounts
            .iter()
            .all(|account| derived_algo_all_clear(account, report.deadman_timeout_secs));
    let derived_spread_orders_all_clear = account_coverage_complete
        && !report.accounts.is_empty()
        && report
            .accounts
            .iter()
            .all(|account| derived_spread_all_clear(account, report.deadman_timeout_secs));
    let derived_account_wide_orders_all_clear = derived_regular_orders_all_clear
        && derived_algo_orders_all_clear
        && derived_spread_orders_all_clear
        && report
            .accounts
            .iter()
            .all(|account| derived_account_all_clear(account, report.deadman_timeout_secs));
    let derived_evidence_complete = schema_matches
        && config_file_matches
        && java_revision_matches
        && environment_matches
        && emergency_configuration_valid
        && scope_matches
        && excluded_order_classes_match
        && is_lower_sha256(&report.config_file_sha256)
        && report.provenance_incident_count == 0
        && report.provenance_incidents.is_empty()
        && report
            .executable_sha256
            .as_deref()
            .is_some_and(is_lower_sha256)
        && report
            .host_identity_sha256
            .as_deref()
            .is_some_and(is_lower_sha256)
        && report.execution_incident_count == 0
        && report.execution_incidents.is_empty()
        && account_coverage_complete
        && report.accounts.iter().all(|account| {
            account
                .account_identity_sha256
                .as_deref()
                .is_some_and(is_lower_sha256)
        });
    let derived_all_clear = derived_account_wide_orders_all_clear && derived_evidence_complete;
    if report.regular_orders_all_clear != derived_regular_orders_all_clear {
        failures.push(
            EmergencyCancelVerificationFailure::RegularOrdersAllClearMismatch {
                reported: report.regular_orders_all_clear,
                derived: derived_regular_orders_all_clear,
            },
        );
    }
    if report.algo_orders_all_clear != derived_algo_orders_all_clear {
        failures.push(
            EmergencyCancelVerificationFailure::AlgoOrdersAllClearMismatch {
                reported: report.algo_orders_all_clear,
                derived: derived_algo_orders_all_clear,
            },
        );
    }
    if report.spread_orders_all_clear != derived_spread_orders_all_clear {
        failures.push(
            EmergencyCancelVerificationFailure::SpreadOrdersAllClearMismatch {
                reported: report.spread_orders_all_clear,
                derived: derived_spread_orders_all_clear,
            },
        );
    }
    if report.account_wide_orders_all_clear != derived_account_wide_orders_all_clear {
        failures.push(
            EmergencyCancelVerificationFailure::AccountWideOrdersAllClearMismatch {
                reported: report.account_wide_orders_all_clear,
                derived: derived_account_wide_orders_all_clear,
            },
        );
    }
    if report.evidence_complete != derived_evidence_complete {
        failures.push(
            EmergencyCancelVerificationFailure::EvidenceCompleteMismatch {
                reported: report.evidence_complete,
                derived: derived_evidence_complete,
            },
        );
    }
    if report.all_clear != derived_all_clear {
        failures.push(EmergencyCancelVerificationFailure::AllClearMismatch {
            reported: report.all_clear,
            derived: derived_all_clear,
        });
    }

    let all_configured_accounts_selected = configured_accounts_valid
        && selected_accounts_valid
        && !configured_accounts.is_empty()
        && selected_accounts == configured_accounts;
    if options.require_all_configured_accounts && !all_configured_accounts_selected {
        failures.push(EmergencyCancelVerificationFailure::AllConfiguredAccountsRequired);
    }
    let evidence_valid = failures.is_empty();
    let acceptance_passed = evidence_valid && derived_all_clear;
    Ok(EmergencyCancelVerificationReport {
        format_version: EMERGENCY_CANCEL_VERIFICATION_FORMAT_VERSION,
        config,
        emergency_report,
        report_schema_version: report.schema_version,
        report_id: report.report_id,
        java_reference_revision: report.java_reference_revision,
        reap_version: report.reap_version,
        executable_sha256: report.executable_sha256,
        host_identity_sha256: report.host_identity_sha256,
        environment: report.environment,
        started_at_ms: report.started_at_ms,
        elapsed_ms: report.elapsed_ms,
        configured_accounts,
        selected_accounts: report.selected_accounts,
        account_identity_sha256s,
        require_all_configured_accounts: options.require_all_configured_accounts,
        all_configured_accounts_selected,
        reported_regular_orders_all_clear: report.regular_orders_all_clear,
        derived_regular_orders_all_clear,
        reported_algo_orders_all_clear: report.algo_orders_all_clear,
        derived_algo_orders_all_clear,
        reported_spread_orders_all_clear: report.spread_orders_all_clear,
        derived_spread_orders_all_clear,
        reported_account_wide_orders_all_clear: report.account_wide_orders_all_clear,
        derived_account_wide_orders_all_clear,
        reported_evidence_complete: report.evidence_complete,
        derived_evidence_complete,
        reported_all_clear: report.all_clear,
        derived_all_clear,
        failures,
        limitations: vec![
            "the report records Reap's authenticated REST outcomes but does not embed raw exchange responses for independent replay".to_string(),
            "OKX has no documented algo-order Cancel All After endpoint; algo zero therefore relies on explicit cancellation, authoritative polling, and the operator's producer-stop confirmation".to_string(),
            "config, executable, host, and account hashes are provenance identifiers, not externally signed attestations".to_string(),
            "this verifier does not prove every external order producer was stopped or replace operator review of incident timing".to_string(),
        ],
        evidence_valid,
        acceptance_passed,
    })
}

fn validate_report_provenance(
    report: &EmergencyCancelReport,
    failures: &mut Vec<EmergencyCancelVerificationFailure>,
) {
    if report.report_id.is_empty()
        || report.report_id.len() > 32
        || !report
            .report_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        failures.push(EmergencyCancelVerificationFailure::InvalidProvenance {
            message: "report id is not a bounded lowercase hexadecimal identifier".to_string(),
        });
    }
    if !is_lower_sha256(&report.config_file_sha256) {
        failures.push(EmergencyCancelVerificationFailure::InvalidProvenance {
            message: "config SHA-256 is invalid".to_string(),
        });
    }
    if report.reap_version.trim().is_empty() || report.reap_version.len() > 128 {
        failures.push(EmergencyCancelVerificationFailure::InvalidProvenance {
            message: "Reap version is empty or unbounded".to_string(),
        });
    }
    for (label, value) in [
        ("executable", report.executable_sha256.as_deref()),
        ("host identity", report.host_identity_sha256.as_deref()),
    ] {
        if value.is_some_and(|value| !is_lower_sha256(value)) {
            failures.push(EmergencyCancelVerificationFailure::InvalidProvenance {
                message: format!("{label} SHA-256 is invalid"),
            });
        }
    }
    if report.started_at_ms == 0 {
        failures.push(EmergencyCancelVerificationFailure::InvalidProvenance {
            message: "start timestamp is zero".to_string(),
        });
    }
}

fn validate_report_shape(
    report: &EmergencyCancelReport,
    failures: &mut Vec<EmergencyCancelVerificationFailure>,
) {
    if !(1..=300_000).contains(&report.account_timeout_ms) {
        failures.push(EmergencyCancelVerificationFailure::ReportInvariant {
            message: "account timeout is outside 1ms..=300s".to_string(),
        });
    }
    if !(50..=5_000).contains(&report.poll_interval_ms) {
        failures.push(EmergencyCancelVerificationFailure::ReportInvariant {
            message: "poll interval is outside 50ms..=5s".to_string(),
        });
    }
    if !(10..=120).contains(&report.deadman_timeout_secs) {
        failures.push(EmergencyCancelVerificationFailure::ReportInvariant {
            message: "deadman timeout is outside 10..=120s".to_string(),
        });
    }
    validate_incidents(
        "provenance",
        report.provenance_incident_count,
        &report.provenance_incidents,
        failures,
    );
    validate_incidents(
        "execution",
        report.execution_incident_count,
        &report.execution_incidents,
        failures,
    );
}

fn validate_incidents(
    label: &str,
    count: u64,
    incidents: &[String],
    failures: &mut Vec<EmergencyCancelVerificationFailure>,
) {
    let expected_retained = count.min(MAX_INCIDENTS as u64) as usize;
    if incidents.len() != expected_retained
        || incidents
            .iter()
            .any(|incident| incident.is_empty() || incident.len() > MAX_INCIDENT_MESSAGE_BYTES)
    {
        failures.push(EmergencyCancelVerificationFailure::ReportInvariant {
            message: format!("{label} incident count or retained messages are inconsistent"),
        });
    }
}

fn validate_account_shape(
    account: &EmergencyAccountReport,
    deadman_timeout_secs: u64,
    account_timeout_ms: u64,
    report_elapsed_ms: u64,
    failures: &mut Vec<EmergencyCancelVerificationFailure>,
) {
    let mut account_failures = Vec::new();
    if account.account_id.trim().is_empty() {
        account_failures.push("account id is empty".to_string());
    }
    if account.exchange_clock_sampled != account.exchange_clock_skew_ms.is_some() {
        account_failures.push("exchange clock sample and skew evidence disagree".to_string());
    }
    if account.enumeration_failures > account.enumeration_attempts {
        account_failures.push("enumeration failures exceed attempts".to_string());
    }
    let enumeration_coverage = [
        account.initial_open_orders.is_some(),
        account.initial_algo_orders.is_some(),
        account.initial_spread_orders.is_some(),
        account.final_open_orders.is_some(),
        account.final_algo_orders.is_some(),
        account.final_spread_orders.is_some(),
    ];
    if enumeration_coverage
        .iter()
        .any(|covered| *covered != enumeration_coverage[0])
    {
        account_failures.push("initial/final order-domain evidence coverage disagrees".to_string());
    }
    if account.initial_open_orders.is_some()
        && account.enumeration_attempts <= account.enumeration_failures
    {
        account_failures.push("enumerated order evidence has no successful attempt".to_string());
    }
    if account
        .initial_open_orders
        .is_some_and(|count| count > account.unique_orders_seen)
    {
        account_failures.push("initial open orders exceed unique orders seen".to_string());
    }
    if account
        .final_open_orders
        .is_some_and(|count| count > account.unique_orders_seen)
    {
        account_failures.push("final open orders exceed unique orders seen".to_string());
    }
    if account
        .initial_algo_orders
        .is_some_and(|count| count > account.unique_algo_orders_seen)
        || account
            .final_algo_orders
            .is_some_and(|count| count > account.unique_algo_orders_seen)
    {
        account_failures.push("algo order counts exceed unique orders seen".to_string());
    }
    if account
        .initial_spread_orders
        .is_some_and(|count| count > account.unique_spread_orders_seen)
        || account
            .final_spread_orders
            .is_some_and(|count| count > account.unique_spread_orders_seen)
    {
        account_failures.push("spread order counts exceed unique orders seen".to_string());
    }
    if account.cancel_batch_failures > account.cancel_batches {
        account_failures.push("cancel batch failures exceed attempts".to_string());
    }
    if account.algo_cancel_batch_failures > account.algo_cancel_batches {
        account_failures.push("algo cancel batch failures exceed attempts".to_string());
    }
    if account.spread_mass_cancel_failures > account.spread_mass_cancel_attempts {
        account_failures.push("spread mass-cancel failures exceed attempts".to_string());
    }
    for (label, final_count, remaining_count) in [
        (
            "regular",
            account.final_open_orders,
            account.remaining_orders.len(),
        ),
        (
            "algo",
            account.final_algo_orders,
            account.remaining_algo_orders.len(),
        ),
        (
            "spread",
            account.final_spread_orders,
            account.remaining_spread_orders.len(),
        ),
    ] {
        let expected_remaining = final_count
            .unwrap_or_default()
            .min(MAX_REMAINING_ORDER_DETAILS);
        if remaining_count != expected_remaining {
            account_failures.push(format!(
                "{label} remaining-order details do not match the final count"
            ));
        }
        if final_count.is_none() && remaining_count != 0 {
            account_failures.push(format!(
                "{label} remaining orders exist without a final enumeration"
            ));
        }
    }
    let verified_domains = [
        account.verified_zero_after_deadman,
        account.verified_algo_zero_after_deadman,
        account.verified_spread_zero_after_deadman,
    ];
    if verified_domains
        .iter()
        .any(|verified| *verified != verified_domains[0])
    {
        account_failures.push("order-domain zero claims disagree".to_string());
    }
    if verified_domains[0] {
        if account.final_open_orders != Some(0)
            || account.final_algo_orders != Some(0)
            || account.final_spread_orders != Some(0)
        {
            account_failures
                .push("account-wide zero is claimed with a nonzero final order domain".to_string());
        }
        let minimum_elapsed_ms = deadman_timeout_secs.saturating_add(2).saturating_mul(1_000);
        if account.elapsed_ms < minimum_elapsed_ms {
            account_failures
                .push("account-wide zero was recorded before the deadman horizon".to_string());
        }
    }
    if account.elapsed_ms > report_elapsed_ms
        || account.elapsed_ms > account_timeout_ms.saturating_add(1_000)
    {
        account_failures.push("account elapsed time exceeds its enclosing deadline".to_string());
    }
    let derived_all_clear = derived_account_all_clear(account, deadman_timeout_secs);
    if account.all_clear != derived_all_clear {
        account_failures
            .push("stored account all-clear does not match deadman/zero evidence".to_string());
    }
    if !account.all_clear && account.account_identity_sha256.is_some() {
        account_failures
            .push("account identity was recorded without account all-clear".to_string());
    }
    if account
        .account_identity_sha256
        .as_deref()
        .is_some_and(|identity| !is_lower_sha256(identity))
    {
        account_failures.push("account identity SHA-256 is invalid".to_string());
    }
    if !is_sorted_unique_nonempty(&account.unmanaged_symbols) {
        account_failures.push("unmanaged symbols are empty, duplicated, or unsorted".to_string());
    }
    let expected_incidents = account.incident_count.min(MAX_INCIDENTS as u64) as usize;
    if account.incidents.len() != expected_incidents
        || account
            .incidents
            .iter()
            .any(|incident| incident.is_empty() || incident.len() > MAX_INCIDENT_MESSAGE_BYTES)
    {
        account_failures.push("incident count or retained messages are inconsistent".to_string());
    }
    for message in account_failures {
        failures.push(EmergencyCancelVerificationFailure::AccountInvariant {
            account_id: account.account_id.clone(),
            message,
        });
    }
}

fn derived_account_all_clear(account: &EmergencyAccountReport, deadman_timeout_secs: u64) -> bool {
    derived_regular_all_clear(account, deadman_timeout_secs)
        && derived_algo_all_clear(account, deadman_timeout_secs)
        && derived_spread_all_clear(account, deadman_timeout_secs)
}

fn derived_regular_all_clear(account: &EmergencyAccountReport, deadman_timeout_secs: u64) -> bool {
    account.deadman_armed
        && account.verified_zero_after_deadman
        && complete_enumeration_coverage(account)
        && account.final_open_orders == Some(0)
        && account.remaining_orders.is_empty()
        && account.elapsed_ms >= deadman_timeout_secs.saturating_add(2).saturating_mul(1_000)
}

fn derived_algo_all_clear(account: &EmergencyAccountReport, deadman_timeout_secs: u64) -> bool {
    account.verified_algo_zero_after_deadman
        && complete_enumeration_coverage(account)
        && account.final_algo_orders == Some(0)
        && account.remaining_algo_orders.is_empty()
        && account.elapsed_ms >= deadman_timeout_secs.saturating_add(2).saturating_mul(1_000)
}

fn derived_spread_all_clear(account: &EmergencyAccountReport, deadman_timeout_secs: u64) -> bool {
    account.spread_deadman_armed
        && account.verified_spread_zero_after_deadman
        && complete_enumeration_coverage(account)
        && account.final_spread_orders == Some(0)
        && account.remaining_spread_orders.is_empty()
        && account.elapsed_ms >= deadman_timeout_secs.saturating_add(2).saturating_mul(1_000)
}

fn complete_enumeration_coverage(account: &EmergencyAccountReport) -> bool {
    account.enumeration_attempts > account.enumeration_failures
        && account.initial_open_orders.is_some()
        && account.initial_algo_orders.is_some()
        && account.initial_spread_orders.is_some()
        && account.final_open_orders.is_some()
        && account.final_algo_orders.is_some()
        && account.final_spread_orders.is_some()
}

fn normalized_account_ids(account_ids: &[String]) -> (Vec<String>, bool) {
    let normalized = account_ids.iter().cloned().collect::<BTreeSet<_>>();
    let valid = normalized.len() == account_ids.len()
        && account_ids
            .iter()
            .all(|account_id| !account_id.trim().is_empty());
    (normalized.into_iter().collect(), valid)
}

fn is_sorted_unique_nonempty(values: &[String]) -> bool {
    values.iter().all(|value| !value.trim().is_empty())
        && values.windows(2).all(|pair| pair[0] < pair[1])
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn read_bounded_regular_file(
    path: &Path,
    label: &'static str,
    limit: u64,
) -> Result<(PathBuf, Vec<u8>), EmergencyCancelVerificationError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        EmergencyCancelVerificationError::InvalidPath {
            label,
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(EmergencyCancelVerificationError::InvalidPath {
            label,
            path: path.to_path_buf(),
            message: "must be a regular file and not a symbolic link".to_string(),
        });
    }
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        EmergencyCancelVerificationError::InvalidPath {
            label,
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    if metadata.len() > limit {
        return Err(EmergencyCancelVerificationError::InputTooLarge {
            label,
            path: canonical,
            actual: metadata.len(),
            limit,
        });
    }
    let bytes = std::fs::read(&canonical).map_err(|source| {
        EmergencyCancelVerificationError::ReadInput {
            label,
            path: canonical.clone(),
            source,
        }
    })?;
    if bytes.len() as u64 > limit {
        return Err(EmergencyCancelVerificationError::InputTooLarge {
            label,
            path: canonical,
            actual: bytes.len() as u64,
            limit,
        });
    }
    Ok((canonical, bytes))
}

fn file_evidence(path: PathBuf, bytes: &[u8]) -> EmergencyCancelFileEvidence {
    EmergencyCancelFileEvidence {
        source_path: path,
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{EmergencyCancelOptions, run_emergency_cancel_path};

    struct Fixture {
        _directory: tempfile::TempDir,
        config_path: PathBuf,
        report_path: PathBuf,
        report: EmergencyCancelReport,
    }

    fn fixture(configured_accounts: &[&str], selected_accounts: &[&str]) -> Fixture {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("live.toml");
        let report_path = directory.path().join("emergency.json");
        let mut config =
            String::from("[venue]\nenvironment = \"demo\"\nrest_url = \"https://www.okx.com\"\n");
        for account in configured_accounts {
            config.push_str(&format!(
                "\n[[accounts]]\nid = \"{account}\"\napi_key_env = \"REAP_EMERGENCY_VERIFY_TEST_MISSING_KEY\"\nsecret_key_env = \"REAP_EMERGENCY_VERIFY_TEST_MISSING_SECRET\"\npassphrase_env = \"REAP_EMERGENCY_VERIFY_TEST_MISSING_PASSPHRASE\"\n"
            ));
        }
        std::fs::write(&config_path, &config).unwrap();
        let selected_accounts = selected_accounts
            .iter()
            .map(|account| (*account).to_string())
            .collect::<Vec<_>>();
        let accounts = selected_accounts
            .iter()
            .map(|account_id| EmergencyAccountReport {
                account_id: account_id.clone(),
                account_identity_sha256: Some(format!(
                    "{:064x}",
                    account_id.bytes().map(u64::from).sum::<u64>()
                )),
                exchange_clock_sampled: true,
                exchange_clock_skew_ms: Some(1),
                deadman_armed: true,
                spread_deadman_armed: true,
                enumeration_attempts: 18,
                enumeration_failures: 0,
                initial_open_orders: Some(1),
                initial_algo_orders: Some(1),
                initial_spread_orders: Some(1),
                unique_orders_seen: 1,
                unique_algo_orders_seen: 1,
                unique_spread_orders_seen: 1,
                cancel_batches: 1,
                cancel_batch_failures: 0,
                accepted_cancel_requests: 1,
                rejected_cancel_requests: 0,
                unacknowledged_cancel_requests: 0,
                algo_cancel_batches: 1,
                algo_cancel_batch_failures: 0,
                accepted_algo_cancel_requests: 1,
                rejected_algo_cancel_requests: 0,
                unacknowledged_algo_cancel_requests: 0,
                spread_mass_cancel_attempts: 1,
                spread_mass_cancel_failures: 0,
                verified_zero_after_deadman: true,
                verified_algo_zero_after_deadman: true,
                verified_spread_zero_after_deadman: true,
                final_open_orders: Some(0),
                final_algo_orders: Some(0),
                final_spread_orders: Some(0),
                unmanaged_symbols: Vec::new(),
                remaining_orders: Vec::new(),
                remaining_algo_orders: Vec::new(),
                remaining_spread_orders: Vec::new(),
                incident_count: 0,
                incidents: Vec::new(),
                elapsed_ms: 13_000,
                all_clear: true,
            })
            .collect::<Vec<_>>();
        let report = EmergencyCancelReport {
            schema_version: EMERGENCY_CANCEL_REPORT_SCHEMA_VERSION,
            report_id: "123abc".to_string(),
            config_file_sha256: format!("{:x}", Sha256::digest(config.as_bytes())),
            java_reference_revision: PINNED_JAVA_REVISION.to_string(),
            reap_version: env!("CARGO_PKG_VERSION").to_string(),
            executable_sha256: Some("1".repeat(64)),
            host_identity_sha256: Some("2".repeat(64)),
            provenance_incident_count: 0,
            provenance_incidents: Vec::new(),
            environment: TradingEnvironment::Demo,
            scope: ACCOUNT_WIDE_ORDER_SCOPE.to_string(),
            excluded_order_classes: EXCLUDED_ORDER_CLASSES
                .into_iter()
                .map(str::to_string)
                .collect(),
            started_at_ms: 1_000,
            elapsed_ms: 13_100,
            account_timeout_ms: 40_000,
            poll_interval_ms: 250,
            deadman_timeout_secs: 10,
            selected_accounts,
            accounts,
            execution_incident_count: 0,
            execution_incidents: Vec::new(),
            regular_orders_all_clear: true,
            algo_orders_all_clear: true,
            spread_orders_all_clear: true,
            account_wide_orders_all_clear: true,
            evidence_complete: true,
            all_clear: true,
        };
        write_report(&report_path, &report);
        Fixture {
            _directory: directory,
            config_path,
            report_path,
            report,
        }
    }

    fn write_report(path: &Path, report: &EmergencyCancelReport) {
        std::fs::write(path, serde_json::to_vec_pretty(report).unwrap()).unwrap();
    }

    #[test]
    fn complete_emergency_report_passes_independent_verification() {
        let fixture = fixture(&["main"], &["main"]);

        let verification = verify_emergency_cancel_paths(
            &fixture.config_path,
            &fixture.report_path,
            EmergencyCancelVerificationOptions {
                require_all_configured_accounts: true,
            },
        )
        .unwrap();

        assert!(verification.evidence_valid);
        assert!(verification.acceptance_passed);
        assert!(verification.all_configured_accounts_selected);
        assert_eq!(verification.account_identity_sha256s.len(), 1);
        assert!(verification.failures.is_empty());
    }

    #[tokio::test]
    async fn collector_failure_report_remains_structurally_verifiable() {
        for name in [
            "REAP_EMERGENCY_VERIFY_TEST_MISSING_KEY",
            "REAP_EMERGENCY_VERIFY_TEST_MISSING_SECRET",
            "REAP_EMERGENCY_VERIFY_TEST_MISSING_PASSPHRASE",
        ] {
            assert!(std::env::var_os(name).is_none());
        }
        let fixture = fixture(&["main"], &["main"]);
        let report = run_emergency_cancel_path(
            &fixture.config_path,
            EmergencyCancelOptions {
                account_ids: vec!["main".to_string()],
                confirm_account_wide_cancel: true,
                confirm_order_producers_stopped: true,
                account_timeout: Duration::from_secs(40),
                ..EmergencyCancelOptions::default()
            },
        )
        .await
        .unwrap();
        assert!(!report.all_clear);
        write_report(&fixture.report_path, &report);

        let verification = verify_emergency_cancel_paths(
            &fixture.config_path,
            &fixture.report_path,
            EmergencyCancelVerificationOptions {
                require_all_configured_accounts: true,
            },
        )
        .unwrap();

        assert!(verification.evidence_valid, "{:?}", verification.failures);
        assert!(!verification.acceptance_passed);
        assert!(!verification.derived_regular_orders_all_clear);
        assert!(!verification.derived_evidence_complete);
    }

    #[test]
    fn verifier_rejects_config_byte_tampering() {
        let fixture = fixture(&["main"], &["main"]);
        let mut config = std::fs::read_to_string(&fixture.config_path).unwrap();
        config.push_str("\n# byte-level mutation\n");
        std::fs::write(&fixture.config_path, config).unwrap();

        let verification = verify_emergency_cancel_paths(
            &fixture.config_path,
            &fixture.report_path,
            EmergencyCancelVerificationOptions::default(),
        )
        .unwrap();

        assert!(!verification.evidence_valid);
        assert!(
            verification
                .failures
                .contains(&EmergencyCancelVerificationFailure::ConfigFileMismatch)
        );
        assert!(!verification.derived_evidence_complete);
    }

    #[test]
    fn verifier_rejects_tampered_completion_flags() {
        let fixture = fixture(&["main"], &["main"]);
        let mut report = fixture.report;
        report.all_clear = false;
        write_report(&fixture.report_path, &report);

        let verification = verify_emergency_cancel_paths(
            &fixture.config_path,
            &fixture.report_path,
            EmergencyCancelVerificationOptions::default(),
        )
        .unwrap();

        assert!(!verification.evidence_valid);
        assert!(verification.failures.contains(
            &EmergencyCancelVerificationFailure::AllClearMismatch {
                reported: false,
                derived: true,
            }
        ));
    }

    #[test]
    fn verifier_replays_emergency_timing_budget_validation() {
        let fixture = fixture(&["main"], &["main"]);
        let mut report = fixture.report;
        report.account_timeout_ms = 30_000;
        write_report(&fixture.report_path, &report);

        let verification = verify_emergency_cancel_paths(
            &fixture.config_path,
            &fixture.report_path,
            EmergencyCancelVerificationOptions::default(),
        )
        .unwrap();

        assert!(!verification.evidence_valid);
        assert!(verification.failures.iter().any(|failure| matches!(
            failure,
            EmergencyCancelVerificationFailure::EmergencyConfigurationInvalid { message }
                if message.contains("account timeout must be at least")
        )));
    }

    #[test]
    fn verifier_can_require_every_configured_account() {
        let fixture = fixture(&["backup", "main"], &["main"]);

        let selected = verify_emergency_cancel_paths(
            &fixture.config_path,
            &fixture.report_path,
            EmergencyCancelVerificationOptions::default(),
        )
        .unwrap();
        assert!(selected.acceptance_passed);
        assert!(!selected.all_configured_accounts_selected);

        let all = verify_emergency_cancel_paths(
            &fixture.config_path,
            &fixture.report_path,
            EmergencyCancelVerificationOptions {
                require_all_configured_accounts: true,
            },
        )
        .unwrap();
        assert!(!all.acceptance_passed);
        assert!(
            all.failures
                .contains(&EmergencyCancelVerificationFailure::AllConfiguredAccountsRequired)
        );
    }

    #[test]
    fn verifier_rejects_zero_claim_with_remaining_orders() {
        let fixture = fixture(&["main"], &["main"]);
        let mut report = fixture.report;
        report.accounts[0].final_open_orders = Some(1);
        report.accounts[0]
            .remaining_orders
            .push(crate::EmergencyOrderRef {
                symbol: "BTC-USDT".to_string(),
                exchange_order_id: "42".to_string(),
                client_order_id: String::new(),
            });
        write_report(&fixture.report_path, &report);

        let verification = verify_emergency_cancel_paths(
            &fixture.config_path,
            &fixture.report_path,
            EmergencyCancelVerificationOptions::default(),
        )
        .unwrap();

        assert!(!verification.evidence_valid);
        assert!(verification.failures.iter().any(|failure| matches!(
            failure,
            EmergencyCancelVerificationFailure::AccountInvariant { message, .. }
                if message.contains("nonzero final order domain")
        )));
    }

    #[test]
    fn verifier_rejects_duplicate_report_account_coverage() {
        let fixture = fixture(&["main"], &["main"]);
        let mut report = fixture.report;
        report.accounts.push(report.accounts[0].clone());
        write_report(&fixture.report_path, &report);

        let verification = verify_emergency_cancel_paths(
            &fixture.config_path,
            &fixture.report_path,
            EmergencyCancelVerificationOptions::default(),
        )
        .unwrap();

        assert!(!verification.evidence_valid);
        assert!(
            verification
                .failures
                .contains(&EmergencyCancelVerificationFailure::AccountCoverageMismatch)
        );
    }

    #[cfg(unix)]
    #[test]
    fn verifier_rejects_symbolic_link_inputs() {
        use std::os::unix::fs::symlink;

        let fixture = fixture(&["main"], &["main"]);
        let linked = fixture._directory.path().join("linked-report.json");
        symlink(&fixture.report_path, &linked).unwrap();

        let error = verify_emergency_cancel_paths(
            &fixture.config_path,
            linked,
            EmergencyCancelVerificationOptions::default(),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            EmergencyCancelVerificationError::InvalidPath { .. }
        ));
    }
}
