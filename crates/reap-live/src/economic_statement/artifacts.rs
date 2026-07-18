use std::collections::BTreeSet;
use std::path::Path;

use reap_venue::okx::parse_okx_account_balance_response_json;
use sha2::{Digest, Sha256};

use super::support::{is_lower_sha256, issue};
use super::{
    BoundAccountBoundary, BoundEconomicSources, EconomicAccountBoundaryEvidence,
    EconomicIssueSource, EconomicReconciliationError, EconomicReconciliationFailure,
    EconomicReconciliationOptions, IssueSink,
};
use crate::{
    FillCollectionFileEvidence, MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES,
    verify_account_certification_artifact_path,
};

pub(super) fn read_account_boundary(
    path: &Path,
    label: &'static str,
) -> Result<BoundAccountBoundary, EconomicReconciliationError> {
    let (file_before, bytes_before) =
        read_input(path, label, MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES)?;
    let parsed_before: crate::AccountCertificationArtifact = serde_json::from_slice(&bytes_before)
        .map_err(|error| {
            EconomicReconciliationError::SourceMismatch(format!(
                "{label} is not a valid account-certification artifact: {error}"
            ))
        })?;
    let verified = verify_account_certification_artifact_path(path)?;
    let (file_after, bytes_after) =
        read_input(path, label, MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES)?;
    if file_before != file_after || bytes_before != bytes_after || parsed_before != verified {
        return Err(EconomicReconciliationError::SourceMismatch(format!(
            "{label} changed while it was being verified"
        )));
    }
    let balance = parse_okx_account_balance_response_json(verified.account_balance.body.as_bytes())
        .map_err(|error| {
            EconomicReconciliationError::SourceMismatch(format!(
                "{label} balance cannot be reparsed: {error}"
            ))
        })?;
    let total_equity_usd = verified.summary.equity.total_equity_usd.ok_or_else(|| {
        EconomicReconciliationError::SourceMismatch(format!(
            "{label} has no verified total account equity"
        ))
    })?;
    Ok(BoundAccountBoundary {
        evidence: EconomicAccountBoundaryEvidence {
            certification_file: file_before,
            certification_schema_version: verified.schema_version,
            collector_reap_version: verified.reap_version,
            collector_executable_sha256: verified.executable_sha256,
            collector_host_identity_sha256: verified.host_identity_sha256,
            start_server_ms: verified.start_clock.server_ms,
            finish_server_ms: verified.finish_clock.server_ms,
            window_gap_ms: 0,
            total_equity_usd,
            balance_currencies: balance.details.len() as u64,
        },
        account_id: verified.summary.account_id,
        environment: verified.summary.environment,
        account_identity_sha256: verified.summary.account_identity_sha256,
        config_fingerprint: verified.config_fingerprint,
        config_source_path: verified.config.source_path,
        config_sha256: verified.config.sha256,
        passed: verified.summary.passed,
        balance,
    })
}

pub(super) fn bind_account_boundaries(
    opening: &mut BoundAccountBoundary,
    closing: &mut BoundAccountBoundary,
    bills: &crate::BillCollectionManifest,
    options: &EconomicReconciliationOptions,
) -> Result<(), EconomicReconciliationError> {
    for (label, boundary) in [("opening", &*opening), ("closing", &*closing)] {
        if !boundary.passed {
            return Err(EconomicReconciliationError::SourceMismatch(format!(
                "{label} account certification did not pass"
            )));
        }
        if boundary.account_id != options.account_id
            || boundary.environment != bills.environment
            || boundary.account_identity_sha256 != bills.account_identity_sha256
        {
            return Err(EconomicReconciliationError::SourceMismatch(format!(
                "{label} account certification does not identify the collected exchange account"
            )));
        }
        if boundary.config_fingerprint != bills.config_fingerprint
            || boundary.config_source_path != bills.config_file.path
            || boundary.config_sha256 != bills.config_file.sha256
        {
            return Err(EconomicReconciliationError::SourceMismatch(format!(
                "{label} account certification does not bind the exact collection config"
            )));
        }
    }
    if opening.account_identity_sha256 != closing.account_identity_sha256
        || opening.config_fingerprint != closing.config_fingerprint
    {
        return Err(EconomicReconciliationError::SourceMismatch(
            "opening and closing account certifications do not bind each other".to_string(),
        ));
    }
    if opening.evidence.finish_server_ms > options.begin_ms {
        return Err(EconomicReconciliationError::SourceMismatch(format!(
            "opening account certification finished at {}, after begin-ms {}",
            opening.evidence.finish_server_ms, options.begin_ms
        )));
    }
    opening.evidence.window_gap_ms = options
        .begin_ms
        .saturating_sub(opening.evidence.finish_server_ms);
    if opening.evidence.window_gap_ms > options.maximum_account_boundary_gap_ms {
        return Err(EconomicReconciliationError::SourceMismatch(format!(
            "opening account boundary gap {} ms exceeds {} ms",
            opening.evidence.window_gap_ms, options.maximum_account_boundary_gap_ms
        )));
    }
    if closing.evidence.start_server_ms < options.end_ms {
        return Err(EconomicReconciliationError::SourceMismatch(format!(
            "closing account certification started at {}, before end-ms {}",
            closing.evidence.start_server_ms, options.end_ms
        )));
    }
    closing.evidence.window_gap_ms = closing
        .evidence
        .start_server_ms
        .saturating_sub(options.end_ms);
    if closing.evidence.window_gap_ms > options.maximum_account_boundary_gap_ms {
        return Err(EconomicReconciliationError::SourceMismatch(format!(
            "closing account boundary gap {} ms exceeds {} ms",
            closing.evidence.window_gap_ms, options.maximum_account_boundary_gap_ms
        )));
    }
    Ok(())
}

pub(super) fn bind_collection_manifests(
    fills: &crate::FillCollectionManifest,
    bills: &crate::BillCollectionManifest,
    options: &EconomicReconciliationOptions,
) -> Result<(), EconomicReconciliationError> {
    if fills.account_id != options.account_id || bills.account_id != options.account_id {
        return Err(EconomicReconciliationError::SourceMismatch(format!(
            "collection accounts {}/{} do not both match requested {}",
            fills.account_id, bills.account_id, options.account_id
        )));
    }
    if bills.window.begin_ms != options.begin_ms || bills.window.end_ms != options.end_ms {
        return Err(EconomicReconciliationError::SourceMismatch(format!(
            "bill window {}..={} does not match requested {}..={}",
            bills.window.begin_ms, bills.window.end_ms, options.begin_ms, options.end_ms
        )));
    }
    let required_fill_begin = options
        .begin_ms
        .saturating_sub(options.maximum_trade_bill_delay_ms);
    if fills.window.begin_ms > required_fill_begin || fills.window.end_ms < options.end_ms {
        return Err(EconomicReconciliationError::SourceMismatch(format!(
            "fill window {}..={} must cover trade matching window {}..={}",
            fills.window.begin_ms, fills.window.end_ms, required_fill_begin, options.end_ms
        )));
    }
    if fills.environment != bills.environment
        || fills.account_identity_sha256 != bills.account_identity_sha256
        || fills.account_level != bills.account_level
        || fills.position_mode != bills.position_mode
    {
        return Err(EconomicReconciliationError::SourceMismatch(
            "fill and bill collections do not identify the same exchange account".to_string(),
        ));
    }
    if fills.config_fingerprint != bills.config_fingerprint
        || fills.config_file != bills.config_file
    {
        return Err(EconomicReconciliationError::SourceMismatch(
            "fill and bill collections do not bind the same exact live config".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn validate_journal_identity(
    sources: &BoundEconomicSources,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) {
    let Some((strategy_name, config_fingerprint)) = sources
        .recovered
        .bootstrap_identities
        .get(&sources.account_id)
    else {
        issues.push(
            EconomicReconciliationFailure::JournalAccountBootstrapMissingOrInvalid,
            issue(
                EconomicIssueSource::Journal,
                None,
                None,
                None,
                "bootstrap",
                "account bootstrap identity",
                "missing",
                "journal does not contain a bootstrap identity for the requested account",
            ),
            failures,
        );
        if sources.recovered.ignored_truncated_tail {
            failures.insert(EconomicReconciliationFailure::JournalTruncatedTail);
        }
        return;
    };
    if strategy_name.trim().is_empty() || !is_lower_sha256(config_fingerprint) {
        issues.push(
            EconomicReconciliationFailure::JournalAccountBootstrapMissingOrInvalid,
            issue(
                EconomicIssueSource::Journal,
                None,
                None,
                None,
                "bootstrap",
                "non-empty strategy and SHA-256 config identity",
                "invalid",
                "journal account bootstrap identity is malformed",
            ),
            failures,
        );
    }
    if strategy_name != &sources.config.strategy.strategy_name {
        issues.push(
            EconomicReconciliationFailure::JournalStrategyMismatch,
            issue(
                EconomicIssueSource::Journal,
                None,
                None,
                None,
                "strategy_name",
                &sources.config.strategy.strategy_name,
                strategy_name,
                "journal bootstrap strategy does not match the live config",
            ),
            failures,
        );
    }
    if config_fingerprint != &sources.config_fingerprint {
        issues.push(
            EconomicReconciliationFailure::JournalConfigFingerprintMismatch,
            issue(
                EconomicIssueSource::Journal,
                None,
                None,
                None,
                "config_fingerprint",
                &sources.config_fingerprint,
                config_fingerprint,
                "journal bootstrap config does not match the verified collections",
            ),
            failures,
        );
    }
    if sources.recovered.ignored_truncated_tail {
        failures.insert(EconomicReconciliationFailure::JournalTruncatedTail);
    }
}

pub(super) fn read_input(
    path: &Path,
    label: &'static str,
    limit: u64,
) -> Result<(FillCollectionFileEvidence, Vec<u8>), EconomicReconciliationError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        EconomicReconciliationError::InvalidInputPath {
            label,
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(EconomicReconciliationError::InvalidInputPath {
            label,
            path: path.to_path_buf(),
            message: "must be a regular file and not a symbolic link".to_string(),
        });
    }
    if metadata.len() > limit {
        return Err(EconomicReconciliationError::InputTooLarge {
            label,
            path: path.to_path_buf(),
            actual: metadata.len(),
            limit,
        });
    }
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        EconomicReconciliationError::InvalidInputPath {
            label,
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    let bytes =
        std::fs::read(&canonical).map_err(|source| EconomicReconciliationError::ReadInput {
            label,
            path: canonical.clone(),
            source,
        })?;
    if bytes.len() as u64 > limit {
        return Err(EconomicReconciliationError::InputTooLarge {
            label,
            path: canonical,
            actual: bytes.len() as u64,
            limit,
        });
    }
    let path = canonical
        .to_str()
        .ok_or_else(|| EconomicReconciliationError::InvalidInputPath {
            label,
            path: canonical.clone(),
            message: "canonical path is not valid UTF-8".to_string(),
        })?;
    Ok((
        FillCollectionFileEvidence {
            path: path.to_string(),
            bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        },
        bytes,
    ))
}
