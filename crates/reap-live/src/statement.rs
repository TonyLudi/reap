use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use reap_core::{FillLiquidity, PINNED_JAVA_REVISION, Side};
use reap_storage::{
    FillRecord, RecoveredStorage, StorageError, acquire_storage_lease, recover_jsonl_bytes,
};
use reap_venue::RemoteFill;
use reap_venue::okx::{RestError, parse_okx_fills_response_json};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::provenance::current_executable_sha256;
use crate::{
    FillCollectionError, FillCollectionFileEvidence, verify_fill_collection_manifest_path,
};

pub const FILL_STATEMENT_REPORT_SCHEMA_VERSION: u32 = 2;
pub const MAX_FILL_STATEMENT_JOURNAL_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_FILL_STATEMENT_PAGE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_FILL_STATEMENT_TOTAL_PAGE_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_FILL_STATEMENT_PAGES: usize = 1_000;
const MAX_OKX_STATEMENT_PAGE_ROWS: usize = 100;

#[derive(Debug, Clone)]
pub struct FillStatementReconciliationOptions {
    pub account_id: String,
    pub begin_ms: u64,
    pub end_ms: u64,
    pub minimum_fills: u64,
    pub tolerances: FillStatementTolerances,
    pub statement_account_and_window_completeness_attested: bool,
}

impl FillStatementReconciliationOptions {
    fn validate(&self) -> Result<(), FillStatementError> {
        if self.account_id.is_empty() || self.account_id.trim() != self.account_id {
            return Err(FillStatementError::InvalidOptions(
                "account id must be non-empty and contain no surrounding whitespace".to_string(),
            ));
        }
        if self.account_id.len() > 128 {
            return Err(FillStatementError::InvalidOptions(
                "account id exceeds 128 bytes".to_string(),
            ));
        }
        if self.begin_ms > self.end_ms {
            return Err(FillStatementError::InvalidOptions(
                "begin-ms must be less than or equal to end-ms".to_string(),
            ));
        }
        if self.minimum_fills == 0 {
            return Err(FillStatementError::InvalidOptions(
                "minimum-fills must be positive".to_string(),
            ));
        }
        for (name, value) in [
            ("price-tolerance", self.tolerances.price_abs),
            ("quantity-tolerance", self.tolerances.quantity_abs),
            ("fee-tolerance", self.tolerances.fee_abs),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(FillStatementError::InvalidOptions(format!(
                    "{name} must be finite and non-negative"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FillStatementTolerances {
    pub price_abs: f64,
    pub quantity_abs: f64,
    pub fee_abs: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillStatementScope {
    FillsAndFeesOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillStatementCoverage {
    ManualRawPages,
    AuthenticatedRecentFillCollection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillStatementSource {
    Journal,
    OkxStatement,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StatementFillKey {
    pub symbol: String,
    pub fill_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FillStatementFileEvidence {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FillStatementWindow {
    pub begin_ms: u64,
    pub end_ms: u64,
    pub endpoints_inclusive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FillJournalRecoveryEvidence {
    pub records: u64,
    pub ignored_truncated_tail: bool,
    pub exclusive_lease_held_while_reading: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FillStatementCounts {
    pub journal_records_total: u64,
    pub journal_records_in_window: u64,
    pub journal_records_selected: u64,
    pub journal_records_for_other_accounts: u64,
    pub journal_records_outside_window: u64,
    pub statement_records_total: u64,
    pub statement_records_selected: u64,
    pub statement_records_outside_window: u64,
    pub compared: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FillRecordIssue {
    pub source: FillStatementSource,
    pub record_index: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<StatementFillKey>,
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FillEvidenceGap {
    pub source: FillStatementSource,
    pub key: StatementFillKey,
    pub field: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FillFieldMismatch {
    pub key: StatementFillKey,
    pub field: String,
    pub journal: String,
    pub statement: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub absolute_difference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerance: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FillStatementComparison {
    pub counts: FillStatementCounts,
    pub invalid_records: Vec<FillRecordIssue>,
    pub duplicate_journal_keys: Vec<StatementFillKey>,
    pub duplicate_statement_keys: Vec<StatementFillKey>,
    pub missing_in_journal: Vec<StatementFillKey>,
    pub missing_in_statement: Vec<StatementFillKey>,
    pub missing_exact_fees: Vec<FillEvidenceGap>,
    pub journal_missing_liquidity: Vec<StatementFillKey>,
    pub mismatches: Vec<FillFieldMismatch>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillStatementFailure {
    StatementAccountAndWindowCompletenessNotAttested,
    JournalAccountBootstrapMissingOrInvalid,
    CollectionConfigFingerprintMismatch,
    JournalTruncatedTail,
    InvalidRecords,
    DuplicateJournalFills,
    DuplicateStatementFills,
    MissingInJournal,
    MissingInStatement,
    MissingExactFees,
    FieldMismatches,
    MinimumFillsNotMet,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FillStatementReconciliationReport {
    pub schema_version: u32,
    pub scope: FillStatementScope,
    pub coverage: FillStatementCoverage,
    pub java_reference_revision: String,
    pub reap_version: String,
    pub executable_sha256: String,
    pub account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journal_strategy_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journal_config_fingerprint: Option<String>,
    pub window: FillStatementWindow,
    pub minimum_fills: u64,
    pub tolerances: FillStatementTolerances,
    pub statement_account_and_window_completeness_attested: bool,
    pub statement_account_and_window_completeness_established: bool,
    pub journal: FillStatementFileEvidence,
    pub journal_recovery: FillJournalRecoveryEvidence,
    pub statement_pages: Vec<FillStatementFileEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection_manifest: Option<FillStatementFileEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_identity_sha256: Option<String>,
    pub comparison: FillStatementComparison,
    pub limitations: Vec<String>,
    pub failures: Vec<FillStatementFailure>,
    pub passed: bool,
}

#[derive(Debug, Error)]
pub enum FillStatementError {
    #[error("invalid fill-statement options: {0}")]
    InvalidOptions(String),
    #[error("at least one unmodified OKX statement response page is required")]
    MissingStatementPages,
    #[error("statement page count {actual} exceeds limit {limit}")]
    TooManyStatementPages { actual: usize, limit: usize },
    #[error("invalid {label} path {path}: {message}")]
    InvalidInputPath {
        label: &'static str,
        path: PathBuf,
        message: String,
    },
    #[error("failed to read {label} {path}: {source}")]
    ReadInput {
        label: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{label} {path} is {actual} bytes; limit is {limit}")]
    InputTooLarge {
        label: &'static str,
        path: PathBuf,
        actual: u64,
        limit: u64,
    },
    #[error("statement path resolves to a duplicate input: {0}")]
    DuplicateStatementPath(PathBuf),
    #[error("statement path resolves to the journal input: {0}")]
    StatementIsJournal(PathBuf),
    #[error("journal recovery failed: {0}")]
    Journal(#[source] StorageError),
    #[error("failed to parse OKX statement page {path}: {source}")]
    StatementParse {
        path: PathBuf,
        #[source]
        source: RestError,
    },
    #[error("OKX statement page {path} has {rows} rows; maximum is {limit}")]
    StatementPageRows {
        path: PathBuf,
        rows: usize,
        limit: usize,
    },
    #[error("statement pages total {actual} bytes; aggregate limit is {limit}")]
    StatementPagesTooLarge { actual: u64, limit: u64 },
    #[error("failed to fingerprint the running executable: {0}")]
    ExecutableHash(String),
    #[error("fill collection failed verification: {0}")]
    Collection(#[from] FillCollectionError),
    #[error("fill collection does not match reconciliation options: {0}")]
    CollectionMismatch(String),
    #[error("collected fill page changed after manifest verification: {0}")]
    CollectionPageChanged(PathBuf),
}

#[derive(Debug)]
enum FillEvidenceContext {
    OperatorPages,
    AuthenticatedCollection {
        manifest: FillStatementFileEvidence,
        account_identity_sha256: String,
        config_fingerprint: String,
        pages: Vec<FillStatementFileEvidence>,
    },
}

impl FillEvidenceContext {
    fn coverage(&self) -> FillStatementCoverage {
        match self {
            Self::OperatorPages => FillStatementCoverage::ManualRawPages,
            Self::AuthenticatedCollection { .. } => {
                FillStatementCoverage::AuthenticatedRecentFillCollection
            }
        }
    }

    fn expected_pages(&self) -> Option<&[FillStatementFileEvidence]> {
        match self {
            Self::OperatorPages => None,
            Self::AuthenticatedCollection { pages, .. } => Some(pages),
        }
    }
}

/// Reconciles a stopped runtime's canonical journal against raw OKX fill pages.
///
/// The journal lease prevents a cooperating live runtime from writing while the
/// exact fingerprinted bytes are recovered. OKX response files are parsed as
/// supplied and are never rewritten or normalized before hashing.
pub fn reconcile_okx_fill_statement_paths(
    journal_path: impl AsRef<Path>,
    statement_paths: &[PathBuf],
    options: FillStatementReconciliationOptions,
) -> Result<FillStatementReconciliationReport, FillStatementError> {
    reconcile_okx_fill_statement_paths_with_context(
        journal_path.as_ref(),
        statement_paths,
        options,
        FillEvidenceContext::OperatorPages,
    )
}

/// Reconciles a stopped runtime against an authenticated, verified recent-fill
/// collection. Account and window coverage come from the collection manifest,
/// rather than an operator assertion.
pub fn reconcile_okx_fill_collection_paths(
    journal_path: impl AsRef<Path>,
    collection_manifest_path: impl AsRef<Path>,
    options: FillStatementReconciliationOptions,
) -> Result<FillStatementReconciliationReport, FillStatementError> {
    options.validate()?;
    let verified = verify_fill_collection_manifest_path(collection_manifest_path)?;
    if verified.manifest.account_id != options.account_id {
        return Err(FillStatementError::CollectionMismatch(format!(
            "account {} does not match requested account {}",
            verified.manifest.account_id, options.account_id
        )));
    }
    if verified.manifest.window.begin_ms != options.begin_ms
        || verified.manifest.window.end_ms != options.end_ms
    {
        return Err(FillStatementError::CollectionMismatch(format!(
            "inclusive window {}..={} does not match requested {}..={}",
            verified.manifest.window.begin_ms,
            verified.manifest.window.end_ms,
            options.begin_ms,
            options.end_ms
        )));
    }
    let context = FillEvidenceContext::AuthenticatedCollection {
        manifest: statement_file_evidence(&verified.manifest_file),
        account_identity_sha256: verified.manifest.account_identity_sha256.clone(),
        config_fingerprint: verified.manifest.config_fingerprint.clone(),
        pages: verified
            .manifest
            .pages
            .iter()
            .map(|page| statement_file_evidence(&page.response))
            .collect(),
    };
    reconcile_okx_fill_statement_paths_with_context(
        journal_path.as_ref(),
        &verified.page_paths,
        options,
        context,
    )
}

fn reconcile_okx_fill_statement_paths_with_context(
    journal_path: &Path,
    statement_paths: &[PathBuf],
    options: FillStatementReconciliationOptions,
    context: FillEvidenceContext,
) -> Result<FillStatementReconciliationReport, FillStatementError> {
    options.validate()?;
    if context
        .expected_pages()
        .is_some_and(|pages| pages.len() != statement_paths.len())
    {
        return Err(FillStatementError::CollectionMismatch(
            "verified page-path and page-evidence counts differ".to_string(),
        ));
    }
    if statement_paths.is_empty() {
        return Err(FillStatementError::MissingStatementPages);
    }
    if statement_paths.len() > MAX_FILL_STATEMENT_PAGES {
        return Err(FillStatementError::TooManyStatementPages {
            actual: statement_paths.len(),
            limit: MAX_FILL_STATEMENT_PAGES,
        });
    }
    let executable_sha256 =
        current_executable_sha256().map_err(FillStatementError::ExecutableHash)?;

    let lease = acquire_storage_lease(journal_path).map_err(FillStatementError::Journal)?;
    let journal_path = lease.journal_path().to_path_buf();
    let (journal, journal_bytes) =
        read_input(&journal_path, "journal", MAX_FILL_STATEMENT_JOURNAL_BYTES)?;
    let recovered = recover_jsonl_bytes(&journal_bytes).map_err(FillStatementError::Journal)?;

    let mut canonical_statement_paths = BTreeSet::new();
    let mut statement_pages = Vec::with_capacity(statement_paths.len());
    let mut statement_fills = Vec::new();
    let mut statement_bytes = 0_u64;
    for (index, path) in statement_paths.iter().enumerate() {
        let canonical = canonical_regular_file(path, "statement page")?;
        if canonical == journal_path {
            return Err(FillStatementError::StatementIsJournal(canonical));
        }
        if !canonical_statement_paths.insert(canonical.clone()) {
            return Err(FillStatementError::DuplicateStatementPath(canonical));
        }
        let (evidence, bytes) =
            read_input(&canonical, "statement page", MAX_FILL_STATEMENT_PAGE_BYTES)?;
        if context
            .expected_pages()
            .and_then(|pages| pages.get(index))
            .is_some_and(|expected| expected != &evidence)
        {
            return Err(FillStatementError::CollectionPageChanged(canonical));
        }
        statement_bytes = statement_bytes.saturating_add(evidence.bytes);
        if statement_bytes > MAX_FILL_STATEMENT_TOTAL_PAGE_BYTES {
            return Err(FillStatementError::StatementPagesTooLarge {
                actual: statement_bytes,
                limit: MAX_FILL_STATEMENT_TOTAL_PAGE_BYTES,
            });
        }
        let fills = parse_okx_fills_response_json(&bytes).map_err(|source| {
            FillStatementError::StatementParse {
                path: canonical.clone(),
                source,
            }
        })?;
        if fills.len() > MAX_OKX_STATEMENT_PAGE_ROWS {
            return Err(FillStatementError::StatementPageRows {
                path: canonical,
                rows: fills.len(),
                limit: MAX_OKX_STATEMENT_PAGE_ROWS,
            });
        }
        statement_pages.push(evidence);
        statement_fills.extend(fills);
    }

    Ok(build_report(
        options,
        executable_sha256,
        journal,
        recovered,
        statement_pages,
        statement_fills,
        context,
    ))
}

fn build_report(
    options: FillStatementReconciliationOptions,
    executable_sha256: String,
    journal: FillStatementFileEvidence,
    recovered: RecoveredStorage,
    statement_pages: Vec<FillStatementFileEvidence>,
    statement_fills: Vec<RemoteFill>,
    context: FillEvidenceContext,
) -> FillStatementReconciliationReport {
    let comparison = compare_fills(&recovered.fills, &statement_fills, &options);
    let coverage = context.coverage();
    let operator_attested = coverage == FillStatementCoverage::ManualRawPages
        && options.statement_account_and_window_completeness_attested;
    let completeness_established =
        operator_attested || coverage == FillStatementCoverage::AuthenticatedRecentFillCollection;
    let (collection_manifest, account_identity_sha256, collection_config_fingerprint) =
        match &context {
            FillEvidenceContext::OperatorPages => (None, None, None),
            FillEvidenceContext::AuthenticatedCollection {
                manifest,
                account_identity_sha256,
                config_fingerprint,
                ..
            } => (
                Some(manifest.clone()),
                Some(account_identity_sha256.clone()),
                Some(config_fingerprint.as_str()),
            ),
        };
    let (journal_strategy_name, journal_config_fingerprint) = recovered
        .bootstrap_identities
        .get(&options.account_id)
        .cloned()
        .map_or((None, None), |(strategy_name, config_fingerprint)| {
            (Some(strategy_name), Some(config_fingerprint))
        });
    let journal_identity_valid = journal_strategy_name
        .as_deref()
        .is_some_and(|strategy_name| !strategy_name.trim().is_empty())
        && journal_config_fingerprint
            .as_deref()
            .is_some_and(is_lower_sha256);
    let mut failures = Vec::new();
    if !completeness_established {
        failures.push(FillStatementFailure::StatementAccountAndWindowCompletenessNotAttested);
    }
    if !journal_identity_valid {
        failures.push(FillStatementFailure::JournalAccountBootstrapMissingOrInvalid);
    }
    if collection_config_fingerprint.is_some_and(|expected| {
        journal_config_fingerprint
            .as_deref()
            .is_some_and(|actual| actual != expected)
    }) {
        failures.push(FillStatementFailure::CollectionConfigFingerprintMismatch);
    }
    if recovered.ignored_truncated_tail {
        failures.push(FillStatementFailure::JournalTruncatedTail);
    }
    if !comparison.invalid_records.is_empty() {
        failures.push(FillStatementFailure::InvalidRecords);
    }
    if !comparison.duplicate_journal_keys.is_empty() {
        failures.push(FillStatementFailure::DuplicateJournalFills);
    }
    if !comparison.duplicate_statement_keys.is_empty() {
        failures.push(FillStatementFailure::DuplicateStatementFills);
    }
    if !comparison.missing_in_journal.is_empty() {
        failures.push(FillStatementFailure::MissingInJournal);
    }
    if !comparison.missing_in_statement.is_empty() {
        failures.push(FillStatementFailure::MissingInStatement);
    }
    if !comparison.missing_exact_fees.is_empty() {
        failures.push(FillStatementFailure::MissingExactFees);
    }
    if !comparison.mismatches.is_empty() {
        failures.push(FillStatementFailure::FieldMismatches);
    }
    if comparison.counts.compared < options.minimum_fills {
        failures.push(FillStatementFailure::MinimumFillsNotMet);
    }

    let mut limitations = Vec::with_capacity(2);
    match coverage {
        FillStatementCoverage::ManualRawPages => limitations.push(
            "OKX response bodies do not echo account or request-window parameters; their coverage is an explicit operator attestation".to_string(),
        ),
        FillStatementCoverage::AuthenticatedRecentFillCollection => limitations.push(
            "coverage is limited to the recent /api/v5/trade/fills retention boundary recorded by the authenticated collection manifest".to_string(),
        ),
    }
    limitations.push(
        "this report does not reconcile balances, positions, funding, equity, liabilities, borrowing, taxes, or currency conversion".to_string(),
    );

    FillStatementReconciliationReport {
        schema_version: FILL_STATEMENT_REPORT_SCHEMA_VERSION,
        scope: FillStatementScope::FillsAndFeesOnly,
        coverage,
        java_reference_revision: PINNED_JAVA_REVISION.to_string(),
        reap_version: env!("CARGO_PKG_VERSION").to_string(),
        executable_sha256,
        account_id: options.account_id,
        journal_strategy_name,
        journal_config_fingerprint,
        window: FillStatementWindow {
            begin_ms: options.begin_ms,
            end_ms: options.end_ms,
            endpoints_inclusive: true,
        },
        minimum_fills: options.minimum_fills,
        tolerances: options.tolerances,
        statement_account_and_window_completeness_attested: operator_attested,
        statement_account_and_window_completeness_established: completeness_established,
        journal,
        journal_recovery: FillJournalRecoveryEvidence {
            records: recovered.records,
            ignored_truncated_tail: recovered.ignored_truncated_tail,
            exclusive_lease_held_while_reading: true,
        },
        statement_pages,
        collection_manifest,
        account_identity_sha256,
        comparison,
        limitations,
        passed: failures.is_empty(),
        failures,
    }
}

fn compare_fills(
    journal_fills: &[FillRecord],
    statement_fills: &[RemoteFill],
    options: &FillStatementReconciliationOptions,
) -> FillStatementComparison {
    let mut comparison = FillStatementComparison::default();
    let mut journal = BTreeMap::new();
    let mut statement = BTreeMap::new();
    let mut duplicate_journal = BTreeSet::new();
    let mut duplicate_statement = BTreeSet::new();
    let mut fee_gaps = BTreeSet::new();
    let mut missing_liquidity = BTreeSet::new();

    for (index, fill) in journal_fills.iter().enumerate() {
        comparison.counts.journal_records_total += 1;
        if !in_window(fill.ts_ms, options) {
            comparison.counts.journal_records_outside_window += 1;
            continue;
        }
        comparison.counts.journal_records_in_window += 1;
        match fill.account_id.as_deref() {
            Some(account_id) if account_id == options.account_id => {}
            Some(_) => {
                comparison.counts.journal_records_for_other_accounts += 1;
                continue;
            }
            None => {
                comparison.invalid_records.push(FillRecordIssue {
                    source: FillStatementSource::Journal,
                    record_index: index as u64,
                    key: partial_key(&fill.symbol, &fill.fill_id),
                    field: "account_id".to_string(),
                    message: "in-window journal fill is not scoped to an account".to_string(),
                });
                continue;
            }
        }
        comparison.counts.journal_records_selected += 1;
        let Some(key) = validate_journal_fill(fill, index, &mut comparison.invalid_records) else {
            continue;
        };
        if fill.fee.is_none() {
            fee_gaps.insert(FillEvidenceGap {
                source: FillStatementSource::Journal,
                key: key.clone(),
                field: "fee".to_string(),
            });
        }
        if fill.liquidity.is_none() {
            missing_liquidity.insert(key.clone());
        }
        if journal.insert(key.clone(), fill).is_some() {
            duplicate_journal.insert(key);
        }
    }

    for (index, fill) in statement_fills.iter().enumerate() {
        comparison.counts.statement_records_total += 1;
        if !in_window(fill.ts_ms, options) {
            comparison.counts.statement_records_outside_window += 1;
            continue;
        }
        comparison.counts.statement_records_selected += 1;
        let Some(key) = validate_statement_fill(fill, index, &mut comparison.invalid_records)
        else {
            continue;
        };
        if fill.fee.is_none() {
            fee_gaps.insert(FillEvidenceGap {
                source: FillStatementSource::OkxStatement,
                key: key.clone(),
                field: "fee".to_string(),
            });
        }
        if statement.insert(key.clone(), fill).is_some() {
            duplicate_statement.insert(key);
        }
    }

    comparison.invalid_records.sort();
    comparison.duplicate_journal_keys = duplicate_journal.into_iter().collect();
    comparison.duplicate_statement_keys = duplicate_statement.into_iter().collect();
    comparison.missing_in_journal = statement
        .keys()
        .filter(|key| !journal.contains_key(*key))
        .cloned()
        .collect();
    comparison.missing_in_statement = journal
        .keys()
        .filter(|key| !statement.contains_key(*key))
        .cloned()
        .collect();
    comparison.missing_exact_fees = fee_gaps.into_iter().collect();
    comparison.journal_missing_liquidity = missing_liquidity.into_iter().collect();

    for (key, journal_fill) in &journal {
        let Some(statement_fill) = statement.get(key) else {
            continue;
        };
        comparison.counts.compared += 1;
        compare_text(
            &mut comparison.mismatches,
            key,
            "order_id",
            &journal_fill.order_id,
            preferred_statement_order_id(statement_fill),
        );
        compare_text(
            &mut comparison.mismatches,
            key,
            "side",
            side_name(journal_fill.side),
            side_name(statement_fill.side),
        );
        compare_number(
            &mut comparison.mismatches,
            key,
            "price",
            journal_fill.price,
            statement_fill.price,
            options.tolerances.price_abs,
        );
        compare_number(
            &mut comparison.mismatches,
            key,
            "quantity",
            journal_fill.qty,
            statement_fill.qty,
            options.tolerances.quantity_abs,
        );
        if let Some(journal_liquidity) = journal_fill.liquidity {
            compare_text(
                &mut comparison.mismatches,
                key,
                "liquidity",
                liquidity_name(journal_liquidity),
                liquidity_name(statement_fill.liquidity),
            );
        }
        if let (Some(journal_fee), Some(statement_fee)) = (&journal_fill.fee, &statement_fill.fee) {
            compare_number(
                &mut comparison.mismatches,
                key,
                "fee_amount",
                journal_fee.amount,
                statement_fee.amount,
                options.tolerances.fee_abs,
            );
            compare_text(
                &mut comparison.mismatches,
                key,
                "fee_currency",
                &normalized_currency(&journal_fee.currency),
                &normalized_currency(&statement_fee.currency),
            );
        }
    }

    comparison
}

fn validate_journal_fill(
    fill: &FillRecord,
    index: usize,
    issues: &mut Vec<FillRecordIssue>,
) -> Option<StatementFillKey> {
    let key = validate_identity(
        FillStatementSource::Journal,
        index,
        &fill.symbol,
        &fill.fill_id,
        &fill.order_id,
        fill.price,
        fill.qty,
        issues,
    )?;
    if let Some(fee) = &fill.fee {
        validate_fee(
            FillStatementSource::Journal,
            index,
            &key,
            fee.amount,
            &fee.currency,
            issues,
        )?;
    }
    Some(key)
}

fn validate_statement_fill(
    fill: &RemoteFill,
    index: usize,
    issues: &mut Vec<FillRecordIssue>,
) -> Option<StatementFillKey> {
    let key = validate_identity(
        FillStatementSource::OkxStatement,
        index,
        &fill.symbol,
        &fill.fill_id,
        preferred_statement_order_id(fill),
        fill.price,
        fill.qty,
        issues,
    )?;
    if let Some(fee) = &fill.fee {
        validate_fee(
            FillStatementSource::OkxStatement,
            index,
            &key,
            fee.amount,
            &fee.currency,
            issues,
        )?;
    }
    Some(key)
}

#[allow(clippy::too_many_arguments)]
fn validate_identity(
    source: FillStatementSource,
    index: usize,
    symbol: &str,
    fill_id: &str,
    order_id: &str,
    price: f64,
    quantity: f64,
    issues: &mut Vec<FillRecordIssue>,
) -> Option<StatementFillKey> {
    let key = StatementFillKey {
        symbol: symbol.to_string(),
        fill_id: fill_id.to_string(),
    };
    let mut valid = true;
    for (field, value) in [
        ("symbol", symbol),
        ("fill_id", fill_id),
        ("order_id", order_id),
    ] {
        if value.is_empty() || value.trim() != value {
            valid = false;
            issues.push(FillRecordIssue {
                source,
                record_index: index as u64,
                key: Some(key.clone()),
                field: field.to_string(),
                message: "must be non-empty and contain no surrounding whitespace".to_string(),
            });
        }
    }
    for (field, value) in [("price", price), ("quantity", quantity)] {
        if !value.is_finite() || value <= 0.0 {
            valid = false;
            issues.push(FillRecordIssue {
                source,
                record_index: index as u64,
                key: Some(key.clone()),
                field: field.to_string(),
                message: "must be finite and positive".to_string(),
            });
        }
    }
    valid.then_some(key)
}

fn validate_fee(
    source: FillStatementSource,
    index: usize,
    key: &StatementFillKey,
    amount: f64,
    currency: &str,
    issues: &mut Vec<FillRecordIssue>,
) -> Option<()> {
    let mut valid = true;
    if !amount.is_finite() {
        valid = false;
        issues.push(FillRecordIssue {
            source,
            record_index: index as u64,
            key: Some(key.clone()),
            field: "fee_amount".to_string(),
            message: "must be finite".to_string(),
        });
    }
    if currency.is_empty() || currency.trim() != currency {
        valid = false;
        issues.push(FillRecordIssue {
            source,
            record_index: index as u64,
            key: Some(key.clone()),
            field: "fee_currency".to_string(),
            message: "must be non-empty and contain no surrounding whitespace".to_string(),
        });
    }
    valid.then_some(())
}

fn compare_text(
    mismatches: &mut Vec<FillFieldMismatch>,
    key: &StatementFillKey,
    field: &str,
    journal: &str,
    statement: &str,
) {
    if journal != statement {
        mismatches.push(FillFieldMismatch {
            key: key.clone(),
            field: field.to_string(),
            journal: journal.to_string(),
            statement: statement.to_string(),
            absolute_difference: None,
            tolerance: None,
        });
    }
}

fn compare_number(
    mismatches: &mut Vec<FillFieldMismatch>,
    key: &StatementFillKey,
    field: &str,
    journal: f64,
    statement: f64,
    tolerance: f64,
) {
    let difference = (journal - statement).abs();
    if difference > tolerance {
        mismatches.push(FillFieldMismatch {
            key: key.clone(),
            field: field.to_string(),
            journal: journal.to_string(),
            statement: statement.to_string(),
            absolute_difference: Some(difference.to_string()),
            tolerance: Some(tolerance.to_string()),
        });
    }
}

fn preferred_statement_order_id(fill: &RemoteFill) -> &str {
    if fill.client_order_id.is_empty() || fill.client_order_id == "0" {
        &fill.exchange_order_id
    } else {
        &fill.client_order_id
    }
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Buy => "buy",
        Side::Sell => "sell",
    }
}

fn liquidity_name(liquidity: FillLiquidity) -> &'static str {
    match liquidity {
        FillLiquidity::Maker => "maker",
        FillLiquidity::Taker => "taker",
    }
}

fn normalized_currency(currency: &str) -> String {
    currency.trim().to_ascii_uppercase()
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn partial_key(symbol: &str, fill_id: &str) -> Option<StatementFillKey> {
    (!symbol.is_empty() || !fill_id.is_empty()).then(|| StatementFillKey {
        symbol: symbol.to_string(),
        fill_id: fill_id.to_string(),
    })
}

fn in_window(ts_ms: u64, options: &FillStatementReconciliationOptions) -> bool {
    (options.begin_ms..=options.end_ms).contains(&ts_ms)
}

fn statement_file_evidence(evidence: &FillCollectionFileEvidence) -> FillStatementFileEvidence {
    FillStatementFileEvidence {
        path: evidence.path.clone(),
        bytes: evidence.bytes,
        sha256: evidence.sha256.clone(),
    }
}

fn canonical_regular_file(path: &Path, label: &'static str) -> Result<PathBuf, FillStatementError> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|error| FillStatementError::InvalidInputPath {
            label,
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(FillStatementError::InvalidInputPath {
            label,
            path: path.to_path_buf(),
            message: "must be a regular file and not a symbolic link".to_string(),
        });
    }
    std::fs::canonicalize(path).map_err(|error| FillStatementError::InvalidInputPath {
        label,
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn read_input(
    path: &Path,
    label: &'static str,
    limit: u64,
) -> Result<(FillStatementFileEvidence, Vec<u8>), FillStatementError> {
    let metadata = std::fs::metadata(path).map_err(|source| FillStatementError::ReadInput {
        label,
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(FillStatementError::InvalidInputPath {
            label,
            path: path.to_path_buf(),
            message: "must be a regular file".to_string(),
        });
    }
    if metadata.len() > limit {
        return Err(FillStatementError::InputTooLarge {
            label,
            path: path.to_path_buf(),
            actual: metadata.len(),
            limit,
        });
    }
    let bytes = std::fs::read(path).map_err(|source| FillStatementError::ReadInput {
        label,
        path: path.to_path_buf(),
        source,
    })?;
    let bytes_len = bytes.len() as u64;
    if bytes_len > limit {
        return Err(FillStatementError::InputTooLarge {
            label,
            path: path.to_path_buf(),
            actual: bytes_len,
            limit,
        });
    }
    let path_string = path
        .to_str()
        .ok_or_else(|| FillStatementError::InvalidInputPath {
            label,
            path: path.to_path_buf(),
            message: "canonical path is not valid UTF-8".to_string(),
        })?
        .to_string();
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    Ok((
        FillStatementFileEvidence {
            path: path_string,
            bytes: bytes_len,
            sha256,
        },
        bytes,
    ))
}

#[cfg(test)]
mod tests {
    use reap_core::{FillFee, FillLiquidity, Side};

    use crate::{
        FILL_COLLECTION_SCHEMA_VERSION, FillCollectionClockEvidence, FillCollectionCoverage,
        FillCollectionManifest, FillCollectionPageEvidence, FillCollectionWindow,
        MAX_FILL_COLLECTION_WINDOW_AGE_MS, MIN_FILL_COLLECTION_PAGE_INTERVAL_MS,
        OKX_RECENT_FILLS_RETENTION_MS,
    };

    use super::*;

    fn options(attested: bool) -> FillStatementReconciliationOptions {
        FillStatementReconciliationOptions {
            account_id: "main".to_string(),
            begin_ms: 1_000,
            end_ms: 2_000,
            minimum_fills: 1,
            tolerances: FillStatementTolerances {
                price_abs: 0.0,
                quantity_abs: 0.0,
                fee_abs: 0.0,
            },
            statement_account_and_window_completeness_attested: attested,
        }
    }

    fn journal_fill() -> FillRecord {
        FillRecord {
            ts_ms: 1_500,
            account_id: Some("main".to_string()),
            fill_id: "trade-1".to_string(),
            order_id: "client-1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            price: 50_000.0,
            qty: 0.01,
            liquidity: None,
            fee: Some(FillFee {
                amount: -0.00001,
                currency: "btc".to_string(),
            }),
        }
    }

    fn statement_fill() -> RemoteFill {
        RemoteFill {
            fill_id: "trade-1".to_string(),
            exchange_order_id: "exchange-1".to_string(),
            client_order_id: "client-1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            price: 50_000.0,
            qty: 0.01,
            liquidity: FillLiquidity::Maker,
            fee: Some(FillFee {
                amount: -0.00001,
                currency: "BTC".to_string(),
            }),
            ts_ms: 1_500,
        }
    }

    fn evidence(path: &str) -> FillStatementFileEvidence {
        FillStatementFileEvidence {
            path: path.to_string(),
            bytes: 1,
            sha256: "a".repeat(64),
        }
    }

    fn recovered_with_identity(fills: Vec<FillRecord>, records: u64) -> RecoveredStorage {
        let mut recovered = RecoveredStorage {
            fills,
            records,
            ..RecoveredStorage::default()
        };
        recovered
            .bootstrap_identities
            .insert("main".to_string(), ("iarb2".to_string(), "c".repeat(64)));
        recovered
    }

    #[test]
    fn exact_fill_and_fee_reconciliation_passes_with_optional_liquidity_gap() {
        let recovered = recovered_with_identity(vec![journal_fill()], 1);

        let report = build_report(
            options(true),
            "b".repeat(64),
            evidence("/journal"),
            recovered,
            vec![evidence("/statement")],
            vec![statement_fill()],
            FillEvidenceContext::OperatorPages,
        );

        assert!(report.passed);
        assert!(report.failures.is_empty());
        assert_eq!(report.comparison.counts.compared, 1);
        assert_eq!(report.comparison.journal_missing_liquidity.len(), 1);
        assert!(report.comparison.mismatches.is_empty());
    }

    #[test]
    fn report_fails_closed_on_missing_fee_duplicate_and_absent_attestation() {
        let mut journal_fill = journal_fill();
        journal_fill.fee = None;
        let recovered = recovered_with_identity(vec![journal_fill.clone(), journal_fill], 2);

        let report = build_report(
            options(false),
            "b".repeat(64),
            evidence("/journal"),
            recovered,
            vec![evidence("/statement")],
            vec![statement_fill()],
            FillEvidenceContext::OperatorPages,
        );

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&FillStatementFailure::StatementAccountAndWindowCompletenessNotAttested)
        );
        assert!(
            report
                .failures
                .contains(&FillStatementFailure::DuplicateJournalFills)
        );
        assert!(
            report
                .failures
                .contains(&FillStatementFailure::MissingExactFees)
        );
    }

    #[test]
    fn report_fails_closed_without_account_bootstrap_identity() {
        let recovered = RecoveredStorage {
            fills: vec![journal_fill()],
            records: 1,
            ..RecoveredStorage::default()
        };

        let report = build_report(
            options(true),
            "b".repeat(64),
            evidence("/journal"),
            recovered,
            vec![evidence("/statement")],
            vec![statement_fill()],
            FillEvidenceContext::OperatorPages,
        );

        assert!(!report.passed);
        assert_eq!(
            report.failures,
            vec![FillStatementFailure::JournalAccountBootstrapMissingOrInvalid]
        );
        assert!(report.journal_strategy_name.is_none());
        assert!(report.journal_config_fingerprint.is_none());
    }

    #[test]
    fn authenticated_collection_establishes_coverage_and_binds_journal_config() {
        let expected_account_identity = "d".repeat(64);
        let context = |config_fingerprint: &str| FillEvidenceContext::AuthenticatedCollection {
            manifest: evidence("/manifest"),
            account_identity_sha256: expected_account_identity.clone(),
            config_fingerprint: config_fingerprint.to_string(),
            pages: vec![evidence("/statement")],
        };
        let report = build_report(
            options(false),
            "b".repeat(64),
            evidence("/journal"),
            recovered_with_identity(vec![journal_fill()], 1),
            vec![evidence("/statement")],
            vec![statement_fill()],
            context(&"c".repeat(64)),
        );

        assert!(report.passed);
        assert_eq!(
            report.coverage,
            FillStatementCoverage::AuthenticatedRecentFillCollection
        );
        assert!(!report.statement_account_and_window_completeness_attested);
        assert!(report.statement_account_and_window_completeness_established);
        assert!(report.collection_manifest.is_some());
        assert_eq!(
            report.account_identity_sha256.as_deref(),
            Some(expected_account_identity.as_str())
        );

        let mismatched = build_report(
            options(false),
            "b".repeat(64),
            evidence("/journal"),
            recovered_with_identity(vec![journal_fill()], 1),
            vec![evidence("/statement")],
            vec![statement_fill()],
            context(&"e".repeat(64)),
        );
        assert!(!mismatched.passed);
        assert!(
            mismatched
                .failures
                .contains(&FillStatementFailure::CollectionConfigFingerprintMismatch)
        );
    }

    #[test]
    fn invalid_window_and_tolerance_are_rejected() {
        let mut invalid_window = options(true);
        invalid_window.begin_ms = 2_001;
        assert!(invalid_window.validate().is_err());

        let mut invalid_tolerance = options(true);
        invalid_tolerance.tolerances.fee_abs = f64::NAN;
        assert!(invalid_tolerance.validate().is_err());
    }

    #[test]
    fn overflowing_numeric_difference_still_serializes_failure_evidence() {
        let mut journal_fill = journal_fill();
        journal_fill.fee.as_mut().unwrap().amount = f64::MAX;
        let mut statement_fill = statement_fill();
        statement_fill.fee.as_mut().unwrap().amount = -f64::MAX;
        let recovered = recovered_with_identity(vec![journal_fill], 1);

        let report = build_report(
            options(true),
            "b".repeat(64),
            evidence("/journal"),
            recovered,
            vec![evidence("/statement")],
            vec![statement_fill],
            FillEvidenceContext::OperatorPages,
        );

        assert!(!report.passed);
        assert_eq!(
            report.comparison.mismatches[0]
                .absolute_difference
                .as_deref(),
            Some("inf")
        );
        serde_json::to_string(&report).unwrap();
    }

    #[test]
    fn path_reconciliation_hashes_the_exact_parsed_inputs() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "reap-fill-statement-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        let journal_path = root.join("journal.jsonl");
        let statement_path = root.join("statement.json");
        let journal_bytes = br#"{"schema_version":5,"record":{"kind":"bootstrap","data":{"ts_ms":1000,"account_id":"main","strategy_name":"iarb2","config_fingerprint":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","baseline_fill_ids":[]}}}
{"schema_version":5,"record":{"kind":"fill","data":{"ts_ms":1500,"account_id":"main","fill_id":"trade-1","order_id":"client-1","symbol":"BTC-USDT","side":"buy","price":50000.0,"qty":0.01,"liquidity":"maker","fee":{"amount":-0.00001,"currency":"BTC"}}}}
"#;
        let statement_bytes = br#"{"code":"0","msg":"","data":[{"billId":"bill-1","tradeId":"trade-1","ordId":"exchange-1","clOrdId":"client-1","instId":"BTC-USDT","side":"buy","fillPx":"50000","fillSz":"0.01","execType":"M","fee":"-0.00001","feeCcy":"BTC","fillTime":"1500"}]}"#;
        std::fs::write(&journal_path, journal_bytes).unwrap();
        std::fs::write(&statement_path, statement_bytes).unwrap();

        let report = reconcile_okx_fill_statement_paths(
            &journal_path,
            std::slice::from_ref(&statement_path),
            options(true),
        )
        .unwrap();

        assert!(report.passed);
        assert_eq!(report.journal.bytes, journal_bytes.len() as u64);
        assert_eq!(
            report.statement_pages[0].bytes,
            statement_bytes.len() as u64
        );
        assert_eq!(report.journal.sha256.len(), 64);
        assert_eq!(report.statement_pages[0].sha256.len(), 64);
        assert_eq!(report.executable_sha256.len(), 64);
        assert_eq!(report.journal_strategy_name.as_deref(), Some("iarb2"));
        assert_eq!(
            report.journal_config_fingerprint.as_deref(),
            Some("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc")
        );
        assert!(report.journal_recovery.exclusive_lease_held_while_reading);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn authenticated_collection_reconciles_exact_manifest_page_and_journal() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "reap-fill-collection-statement-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        let config_path = root.join("live.toml");
        let page_path = root.join("page-0001.json");
        let manifest_path = root.join("manifest.json");
        let journal_path = root.join("journal.jsonl");
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let begin_ms = now_ms - 120_000;
        let fill_ms = now_ms - 90_000;
        let end_ms = now_ms - 60_000;
        let config_bytes = include_bytes!("../../../examples/live-okx-demo.toml");
        std::fs::write(&config_path, config_bytes).unwrap();
        let config =
            crate::LiveConfig::from_toml(std::str::from_utf8(config_bytes).unwrap()).unwrap();
        let config_fingerprint = config.fingerprint().unwrap();
        let account = config.account("main").unwrap();
        let page_bytes = format!(
            r#"{{"code":"0","msg":"","data":[{{"billId":"bill-1","tradeId":"trade-1","ordId":"exchange-1","clOrdId":"client-1","instId":"BTC-USDT","side":"buy","fillPx":"50000","fillSz":"0.01","execType":"M","fee":"-0.00001","feeCcy":"BTC","fillTime":"{fill_ms}"}}]}}"#
        );
        std::fs::write(&page_path, page_bytes.as_bytes()).unwrap();
        let collection_evidence = |path: &Path| {
            let canonical = std::fs::canonicalize(path).unwrap();
            let bytes = std::fs::read(&canonical).unwrap();
            FillCollectionFileEvidence {
                path: canonical.to_str().unwrap().to_string(),
                bytes: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&bytes)),
            }
        };
        let config_file = collection_evidence(&config_path);
        let response = collection_evidence(&page_path);
        let manifest = FillCollectionManifest {
            schema_version: FILL_COLLECTION_SCHEMA_VERSION,
            coverage: FillCollectionCoverage::CompleteOkxRecentFills,
            java_reference_revision: PINNED_JAVA_REVISION.to_string(),
            reap_version: env!("CARGO_PKG_VERSION").to_string(),
            executable_sha256: "a".repeat(64),
            host_identity_sha256: "b".repeat(64),
            config_file,
            config_fingerprint: config_fingerprint.clone(),
            environment: config.venue.environment,
            account_id: "main".to_string(),
            account_identity_sha256: "d".repeat(64),
            account_level: account.expected_account_level,
            position_mode: account.expected_position_mode,
            endpoint: "/api/v5/trade/fills".to_string(),
            retention_ms: OKX_RECENT_FILLS_RETENTION_MS,
            maximum_window_age_ms: MAX_FILL_COLLECTION_WINDOW_AGE_MS,
            window: FillCollectionWindow {
                begin_ms,
                end_ms,
                endpoints_inclusive: true,
                minimum_close_delay_ms: 30_000,
            },
            max_pages: 3,
            page_interval_ms: MIN_FILL_COLLECTION_PAGE_INTERVAL_MS,
            start_clock: FillCollectionClockEvidence {
                local_midpoint_ms: now_ms,
                server_ms: now_ms,
                absolute_skew_ms: 0,
            },
            finish_clock: FillCollectionClockEvidence {
                local_midpoint_ms: now_ms + 1,
                server_ms: now_ms + 1,
                absolute_skew_ms: 0,
            },
            pages: vec![FillCollectionPageEvidence {
                page_index: 1,
                request_path: "/api/v5/trade/fills?limit=100".to_string(),
                requested_after: None,
                next_after: None,
                rows: 1,
                minimum_fill_time_ms: Some(fill_ms),
                maximum_fill_time_ms: Some(fill_ms),
                response,
            }],
            total_rows: 1,
            window_rows: 1,
            total_response_bytes: page_bytes.len() as u64,
            account_identity_sampled_before_and_after: true,
            complete: true,
        };
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let journal_bytes = format!(
            "{{\"schema_version\":5,\"record\":{{\"kind\":\"bootstrap\",\"data\":{{\"ts_ms\":{begin_ms},\"account_id\":\"main\",\"strategy_name\":\"iarb2\",\"config_fingerprint\":\"{config_fingerprint}\",\"baseline_fill_ids\":[]}}}}}}\n{{\"schema_version\":5,\"record\":{{\"kind\":\"fill\",\"data\":{{\"ts_ms\":{fill_ms},\"account_id\":\"main\",\"fill_id\":\"trade-1\",\"order_id\":\"client-1\",\"symbol\":\"BTC-USDT\",\"side\":\"buy\",\"price\":50000.0,\"qty\":0.01,\"liquidity\":\"maker\",\"fee\":{{\"amount\":-0.00001,\"currency\":\"BTC\"}}}}}}}}\n"
        );
        std::fs::write(&journal_path, journal_bytes).unwrap();
        let report = reconcile_okx_fill_collection_paths(
            &journal_path,
            &manifest_path,
            FillStatementReconciliationOptions {
                account_id: "main".to_string(),
                begin_ms,
                end_ms,
                minimum_fills: 1,
                tolerances: FillStatementTolerances {
                    price_abs: 0.0,
                    quantity_abs: 0.0,
                    fee_abs: 0.0,
                },
                statement_account_and_window_completeness_attested: false,
            },
        )
        .unwrap();

        assert!(report.passed);
        assert_eq!(
            report.coverage,
            FillStatementCoverage::AuthenticatedRecentFillCollection
        );
        assert_eq!(report.comparison.counts.compared, 1);
        assert!(report.collection_manifest.is_some());
        std::fs::remove_dir_all(root).unwrap();
    }
}
