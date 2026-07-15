use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use reap_core::{FillLiquidity, MarketEvent, NormalizedEvent, PINNED_JAVA_REVISION, Side};
use reap_storage::{
    RecoveredStorage, StorageError, StorageRecord, acquire_storage_lease,
    recover_jsonl_bytes_with_visitor,
};
use reap_strategy::{InstrumentConfig, InstrumentKindConfig};
use reap_venue::RemoteFill;
use reap_venue::okx::{
    OkxBill, OkxBillExecutionType, OkxBillMarginMode, OkxInstrumentType, OkxTradeMode,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::provenance::current_executable_sha256;
use crate::{
    BillCollectionError, BillCollectionWindow, FillCollectionError, FillCollectionFileEvidence,
    LiveConfig, LiveConfigError, TradingEnvironment, verify_bill_collection_manifest_path,
    verify_fill_collection_manifest_path,
};

pub const ECONOMIC_RECONCILIATION_SCHEMA_VERSION: u32 = 3;
pub const MAX_ECONOMIC_JOURNAL_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_ECONOMIC_CONFIG_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_ECONOMIC_REPORTED_ISSUES: usize = 1_024;
pub const MAX_ECONOMIC_FUNDING_SAMPLES: usize = 1_024;
pub const MAX_TRADE_BILL_DELAY_MS: u64 = 10 * 60 * 1_000;
pub const MAX_FUNDING_BILL_DELAY_MS: u64 = 10 * 60 * 1_000;
pub const MAX_FUNDING_MARK_BRACKET_DISTANCE_MS: u64 = 10_000;

#[derive(Debug, Clone)]
pub struct EconomicReconciliationOptions {
    pub account_id: String,
    pub begin_ms: u64,
    pub end_ms: u64,
    pub minimum_trade_bills: u64,
    pub minimum_funding_bills: u64,
    pub maximum_trade_bill_delay_ms: u64,
    pub maximum_funding_bill_delay_ms: u64,
    pub maximum_funding_mark_bracket_distance_ms: u64,
    pub tolerances: EconomicReconciliationTolerances,
}

impl EconomicReconciliationOptions {
    fn validate(&self) -> Result<(), EconomicReconciliationError> {
        if self.account_id.is_empty() || self.account_id.trim() != self.account_id {
            return Err(EconomicReconciliationError::InvalidOptions(
                "account id must be non-empty and contain no surrounding whitespace".to_string(),
            ));
        }
        if self.account_id.len() > 128 {
            return Err(EconomicReconciliationError::InvalidOptions(
                "account id exceeds 128 bytes".to_string(),
            ));
        }
        if self.begin_ms == 0 || self.end_ms == 0 || self.begin_ms > self.end_ms {
            return Err(EconomicReconciliationError::InvalidOptions(
                "begin-ms and end-ms must form a positive inclusive window".to_string(),
            ));
        }
        if self.minimum_trade_bills == 0 || self.minimum_funding_bills == 0 {
            return Err(EconomicReconciliationError::InvalidOptions(
                "minimum trade and funding bill counts must both be positive".to_string(),
            ));
        }
        if self.maximum_trade_bill_delay_ms == 0
            || self.maximum_trade_bill_delay_ms > MAX_TRADE_BILL_DELAY_MS
        {
            return Err(EconomicReconciliationError::InvalidOptions(format!(
                "maximum-trade-bill-delay-ms must be in 1..={MAX_TRADE_BILL_DELAY_MS}"
            )));
        }
        if self.maximum_funding_bill_delay_ms == 0
            || self.maximum_funding_bill_delay_ms > MAX_FUNDING_BILL_DELAY_MS
        {
            return Err(EconomicReconciliationError::InvalidOptions(format!(
                "maximum-funding-bill-delay-ms must be in 1..={MAX_FUNDING_BILL_DELAY_MS}"
            )));
        }
        if self.maximum_funding_mark_bracket_distance_ms == 0
            || self.maximum_funding_mark_bracket_distance_ms > MAX_FUNDING_MARK_BRACKET_DISTANCE_MS
        {
            return Err(EconomicReconciliationError::InvalidOptions(format!(
                "maximum-funding-mark-bracket-distance-ms must be in 1..={MAX_FUNDING_MARK_BRACKET_DISTANCE_MS}"
            )));
        }
        for (name, value) in [
            ("price-abs", self.tolerances.price_abs),
            ("quantity-abs", self.tolerances.quantity_abs),
            ("fee-abs", self.tolerances.fee_abs),
            ("balance-abs", self.tolerances.balance_abs),
            ("funding-pnl-abs", self.tolerances.funding_pnl_abs),
            ("funding-pnl-relative", self.tolerances.funding_pnl_relative),
            ("funding-mark-abs", self.tolerances.funding_mark_abs),
            (
                "funding-mark-relative",
                self.tolerances.funding_mark_relative,
            ),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(EconomicReconciliationError::InvalidOptions(format!(
                    "{name} tolerance must be finite and non-negative"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EconomicReconciliationTolerances {
    pub price_abs: f64,
    pub quantity_abs: f64,
    pub fee_abs: f64,
    pub balance_abs: f64,
    pub funding_pnl_abs: f64,
    pub funding_pnl_relative: f64,
    pub funding_mark_abs: f64,
    pub funding_mark_relative: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EconomicReconciliationScope {
    NormalTradeAndFundingBills,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EconomicIssueSource {
    Config,
    Journal,
    FillCollection,
    BillCollection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EconomicJournalRecoveryEvidence {
    pub records: u64,
    pub ignored_truncated_tail: bool,
    pub account_bootstrap_records: u64,
    pub runtime_session_records: u64,
    pub funding_settlement_records: u64,
    pub position_observation_records: u64,
    pub mark_price_observation_records: u64,
    pub exclusive_lease_held_while_reading: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EconomicReconciliationCounts {
    pub bills_total: u64,
    pub trade_bills: u64,
    pub funding_bills: u64,
    pub unsupported_bills: u64,
    pub fills_total: u64,
    pub fills_in_required_collection_window: u64,
    pub fills_eligible_for_completeness: u64,
    pub fills_in_end_guard: u64,
    pub trade_bills_matched: u64,
    pub trade_bills_validated: u64,
    pub eligible_fills_missing_bill: u64,
    pub funding_settlements_total: u64,
    pub funding_settlements_relevant: u64,
    pub funding_bills_matched: u64,
    pub funding_mark_brackets_validated: u64,
    pub funding_bills_validated: u64,
    pub issues_total: u64,
    pub issues_reported: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EconomicIssue {
    pub source: EconomicIssueSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bill_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trade_id: Option<String>,
    pub field: String,
    pub expected: String,
    pub observed: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FundingFormulaSample {
    pub bill_id: String,
    pub symbol: String,
    pub runtime_session_id: String,
    pub runtime_session_start_line: u64,
    pub runtime_session_started_at_ms: u64,
    pub bill_timestamp_ms: u64,
    pub settlement_time_ms: u64,
    pub settlement_delay_ms: u64,
    pub assessment_time_ms: u64,
    pub assessment_delay_ms: u64,
    pub rate: f64,
    pub inverse: bool,
    pub currency: String,
    pub quantity: f64,
    pub journal_position_quantity: f64,
    pub position_observation_line: u64,
    pub position_observation_time_ms: u64,
    pub contract_value: f64,
    pub bill_mark_price: f64,
    pub mark_before_line: u64,
    pub mark_before_time_ms: u64,
    pub mark_before_price: f64,
    pub mark_after_line: u64,
    pub mark_after_time_ms: u64,
    pub mark_after_price: f64,
    pub mark_lower_bound: f64,
    pub mark_upper_bound: f64,
    pub mark_effective_tolerance: f64,
    pub mark_validated: bool,
    pub expected_pnl_at_bill_mark: f64,
    pub expected_pnl_lower_bound: f64,
    pub expected_pnl_upper_bound: f64,
    pub expected_pnl_absolute: f64,
    pub observed_pnl: f64,
    pub absolute_difference: f64,
    pub relative_difference: f64,
    pub effective_tolerance: f64,
    pub validated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EconomicReconciliationFailure {
    JournalAccountBootstrapMissingOrInvalid,
    JournalConfigFingerprintMismatch,
    JournalStrategyMismatch,
    JournalTruncatedTail,
    InvalidOrDuplicateRuntimeSessions,
    InvalidOrDuplicateFundingSettlements,
    InvalidOrDuplicateFundingMarks,
    DuplicateFills,
    DuplicateTradeBills,
    UnsupportedBills,
    InvalidTradeBills,
    TradeBillsMissingFills,
    EligibleFillsMissingBills,
    InvalidFundingBills,
    FundingBillsMissingSettlements,
    FundingSessionBoundaryMissing,
    FundingPositionMismatches,
    FundingMarkBracketsMissing,
    FundingMarkMismatches,
    FundingFormulaMismatches,
    MinimumTradeBillsNotMet,
    MinimumFundingBillsNotMet,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EconomicReconciliationReport {
    pub schema_version: u32,
    pub scope: EconomicReconciliationScope,
    pub java_reference_revision: String,
    pub reap_version: String,
    pub executable_sha256: String,
    pub account_id: String,
    pub environment: TradingEnvironment,
    pub account_identity_sha256: String,
    pub strategy_name: String,
    pub config_fingerprint: String,
    pub window: BillCollectionWindow,
    pub minimum_trade_bills: u64,
    pub minimum_funding_bills: u64,
    pub maximum_trade_bill_delay_ms: u64,
    pub maximum_funding_bill_delay_ms: u64,
    pub maximum_funding_mark_bracket_distance_ms: u64,
    pub tolerances: EconomicReconciliationTolerances,
    pub config_file: FillCollectionFileEvidence,
    pub journal: FillCollectionFileEvidence,
    pub journal_recovery: EconomicJournalRecoveryEvidence,
    pub fill_collection_manifest: FillCollectionFileEvidence,
    pub bill_collection_manifest: FillCollectionFileEvidence,
    pub counts: EconomicReconciliationCounts,
    pub funding_formula_samples: Vec<FundingFormulaSample>,
    pub funding_formula_samples_omitted: u64,
    pub issues: Vec<EconomicIssue>,
    pub issues_truncated: bool,
    pub limitations: Vec<String>,
    pub failures: Vec<EconomicReconciliationFailure>,
    pub passed: bool,
}

#[derive(Debug, Error)]
pub enum EconomicReconciliationError {
    #[error("invalid economic-reconciliation options: {0}")]
    InvalidOptions(String),
    #[error("fill collection failed verification: {0}")]
    FillCollection(#[from] FillCollectionError),
    #[error("bill collection failed verification: {0}")]
    BillCollection(#[from] BillCollectionError),
    #[error("verified economic sources do not bind: {0}")]
    SourceMismatch(String),
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
    #[error("live config failed validation: {0}")]
    Config(#[from] LiveConfigError),
    #[error("journal recovery failed: {0}")]
    Journal(#[source] StorageError),
    #[error("failed to fingerprint the running executable: {0}")]
    ExecutableHash(String),
}

#[derive(Debug, Clone)]
struct JournalFundingSettlement {
    line: u64,
    event_ts_ms: u64,
    symbol: String,
    funding_time_ms: u64,
    rate: f64,
}

#[derive(Debug, Clone)]
struct JournalPositionObservation {
    line: u64,
    event_ts_ms: u64,
    symbol: String,
    quantity: f64,
}

#[derive(Debug, Clone)]
struct JournalMarkPriceObservation {
    line: u64,
    event_ts_ms: u64,
    symbol: String,
    price: f64,
}

#[derive(Debug, Clone)]
struct JournalRuntimeSession {
    line: u64,
    started_at_ms: u64,
    session_id: String,
    account_id: String,
    strategy_name: String,
    config_fingerprint: String,
    account_identity_sha256: String,
}

#[derive(Debug, Default)]
struct IssueSink {
    total: u64,
    issues: Vec<EconomicIssue>,
}

impl IssueSink {
    fn push(
        &mut self,
        failure: EconomicReconciliationFailure,
        issue: EconomicIssue,
        failures: &mut BTreeSet<EconomicReconciliationFailure>,
    ) {
        failures.insert(failure);
        self.total = self.total.saturating_add(1);
        if self.issues.len() < MAX_ECONOMIC_REPORTED_ISSUES {
            self.issues.push(issue);
        }
    }
}

struct BoundEconomicSources {
    account_id: String,
    config: LiveConfig,
    config_file: FillCollectionFileEvidence,
    journal: FillCollectionFileEvidence,
    recovered: RecoveredStorage,
    account_bootstrap_records: u64,
    runtime_sessions: Vec<JournalRuntimeSession>,
    settlements: Vec<JournalFundingSettlement>,
    position_observations: Vec<JournalPositionObservation>,
    mark_price_observations: Vec<JournalMarkPriceObservation>,
    fill_manifest_file: FillCollectionFileEvidence,
    bill_manifest_file: FillCollectionFileEvidence,
    fills: Vec<RemoteFill>,
    bills: Vec<OkxBill>,
    environment: TradingEnvironment,
    account_identity_sha256: String,
    config_fingerprint: String,
    window: BillCollectionWindow,
}

/// Rebuilds normal-trade and funding economics from exact verified collections
/// plus a stopped runtime journal. No credentials or network access are used.
pub fn reconcile_okx_economics_paths(
    journal_path: impl AsRef<Path>,
    fill_collection_manifest_path: impl AsRef<Path>,
    bill_collection_manifest_path: impl AsRef<Path>,
    options: EconomicReconciliationOptions,
) -> Result<EconomicReconciliationReport, EconomicReconciliationError> {
    options.validate()?;
    let fills = verify_fill_collection_manifest_path(fill_collection_manifest_path)?;
    let bills = verify_bill_collection_manifest_path(bill_collection_manifest_path)?;
    bind_collection_manifests(&fills.manifest, &bills.manifest, &options)?;

    let config_path = PathBuf::from(&bills.manifest.config_file.path);
    let (config_file, config_bytes) = read_input(
        &config_path,
        "referenced live config",
        MAX_ECONOMIC_CONFIG_BYTES,
    )?;
    if config_file != bills.manifest.config_file {
        return Err(EconomicReconciliationError::SourceMismatch(
            "referenced live config changed after collection verification".to_string(),
        ));
    }
    let config_text = std::str::from_utf8(&config_bytes).map_err(|error| {
        EconomicReconciliationError::SourceMismatch(format!(
            "referenced live config is not valid UTF-8: {error}"
        ))
    })?;
    let config = LiveConfig::from_toml(config_text)?;
    if config.fingerprint()? != bills.manifest.config_fingerprint {
        return Err(EconomicReconciliationError::SourceMismatch(
            "referenced live config fingerprint changed after collection verification".to_string(),
        ));
    }

    let lease =
        acquire_storage_lease(journal_path).map_err(EconomicReconciliationError::Journal)?;
    let (journal, journal_bytes) =
        read_input(lease.journal_path(), "journal", MAX_ECONOMIC_JOURNAL_BYTES)?;
    let mut account_bootstrap_records = 0_u64;
    let mut runtime_sessions = Vec::new();
    let mut settlements = Vec::new();
    let mut position_observations = Vec::new();
    let mut mark_price_observations = Vec::new();
    let recovered = recover_jsonl_bytes_with_visitor(&journal_bytes, |line, record| match record {
        StorageRecord::Bootstrap(_) => {
            account_bootstrap_records = account_bootstrap_records.saturating_add(1);
        }
        StorageRecord::SessionStart(session) => runtime_sessions.push(JournalRuntimeSession {
            line,
            started_at_ms: session.ts_ms,
            session_id: session.session_id.clone(),
            account_id: session.account_id.clone(),
            strategy_name: session.strategy_name.clone(),
            config_fingerprint: session.config_fingerprint.clone(),
            account_identity_sha256: session.account_identity_sha256.clone(),
        }),
        StorageRecord::Normalized(NormalizedEvent::Market(MarketEvent::FundingRate {
            ts_ms,
            symbol,
            settlement: Some(settlement),
            ..
        })) => settlements.push(JournalFundingSettlement {
            line,
            event_ts_ms: *ts_ms,
            symbol: symbol.clone(),
            funding_time_ms: settlement.funding_time_ms,
            rate: settlement.rate,
        }),
        StorageRecord::Normalized(NormalizedEvent::Market(MarketEvent::PriceLimits {
            ts_ms,
            symbol,
            mark_price,
            ..
        })) if *mark_price != 0.0 => {
            mark_price_observations.push(JournalMarkPriceObservation {
                line,
                event_ts_ms: *ts_ms,
                symbol: symbol.clone(),
                price: *mark_price,
            });
        }
        StorageRecord::Normalized(NormalizedEvent::Account(update)) => {
            position_observations.extend(update.positions.iter().map(|position| {
                JournalPositionObservation {
                    line,
                    event_ts_ms: update.ts_ms,
                    symbol: position.symbol.clone(),
                    quantity: position.qty,
                }
            }));
        }
        _ => {}
    })
    .map_err(EconomicReconciliationError::Journal)?;

    let sources = BoundEconomicSources {
        account_id: options.account_id.clone(),
        config,
        config_file,
        journal,
        recovered,
        account_bootstrap_records,
        runtime_sessions,
        settlements,
        position_observations,
        mark_price_observations,
        fill_manifest_file: fills.manifest_file,
        bill_manifest_file: bills.manifest_file,
        fills: fills.fills,
        bills: bills.bills,
        environment: bills.manifest.environment,
        account_identity_sha256: bills.manifest.account_identity_sha256,
        config_fingerprint: bills.manifest.config_fingerprint,
        window: bills.manifest.window,
    };
    let executable_sha256 =
        current_executable_sha256().map_err(EconomicReconciliationError::ExecutableHash)?;
    Ok(build_report(sources, options, executable_sha256))
}

fn bind_collection_manifests(
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

fn build_report(
    sources: BoundEconomicSources,
    options: EconomicReconciliationOptions,
    executable_sha256: String,
) -> EconomicReconciliationReport {
    let mut counts = EconomicReconciliationCounts {
        bills_total: sources.bills.len() as u64,
        fills_total: sources.fills.len() as u64,
        funding_settlements_total: sources.settlements.len() as u64,
        ..EconomicReconciliationCounts::default()
    };
    let mut failures = BTreeSet::new();
    let mut issues = IssueSink::default();
    validate_journal_identity(&sources, &mut failures, &mut issues);

    let required_fill_begin = options
        .begin_ms
        .saturating_sub(options.maximum_trade_bill_delay_ms);
    let completeness_end = options
        .end_ms
        .saturating_sub(options.maximum_trade_bill_delay_ms);
    for fill in &sources.fills {
        if (required_fill_begin..=options.end_ms).contains(&fill.ts_ms) {
            counts.fills_in_required_collection_window += 1;
        }
        if (options.begin_ms..=completeness_end).contains(&fill.ts_ms) {
            counts.fills_eligible_for_completeness += 1;
        } else if fill.ts_ms > completeness_end && fill.ts_ms <= options.end_ms {
            counts.fills_in_end_guard += 1;
        }
    }

    let mut fill_by_key = BTreeMap::new();
    for fill in &sources.fills {
        let key = (fill.symbol.clone(), fill.fill_id.clone());
        if fill_by_key.insert(key.clone(), fill).is_some() {
            issues.push(
                EconomicReconciliationFailure::DuplicateFills,
                issue(
                    EconomicIssueSource::FillCollection,
                    None,
                    Some(&key.0),
                    Some(&key.1),
                    "trade_identity",
                    "unique (symbol, tradeId)",
                    "duplicate",
                    "verified fill pages contain a duplicate trade identity",
                ),
                &mut failures,
            );
        }
    }

    let valid_runtime_sessions = validate_runtime_sessions(&sources, &mut failures, &mut issues);
    let valid_settlements = validate_funding_settlements(
        &sources,
        &valid_runtime_sessions,
        &options,
        &mut counts,
        &mut failures,
        &mut issues,
    );
    let valid_mark_prices = validate_funding_mark_prices(
        &sources,
        &valid_runtime_sessions,
        &mut failures,
        &mut issues,
    );
    let mut trade_bill_keys = BTreeSet::new();
    let mut matched_fill_keys = BTreeSet::new();
    let mut funding_samples = Vec::new();
    let mut funding_samples_omitted = 0_u64;

    for bill in &sources.bills {
        match bill.bill_type.as_str() {
            "2" => {
                counts.trade_bills += 1;
                let key = (bill.symbol.clone(), bill.trade_id.clone());
                if !trade_bill_keys.insert(key.clone()) {
                    issues.push(
                        EconomicReconciliationFailure::DuplicateTradeBills,
                        issue_for_bill(
                            EconomicIssueSource::BillCollection,
                            bill,
                            "trade_identity",
                            "unique (symbol, tradeId)",
                            "duplicate",
                            "multiple trade bills have the same exchange trade identity",
                        ),
                        &mut failures,
                    );
                    continue;
                }
                let Some(fill) = fill_by_key.get(&key).copied() else {
                    issues.push(
                        EconomicReconciliationFailure::TradeBillsMissingFills,
                        issue_for_bill(
                            EconomicIssueSource::BillCollection,
                            bill,
                            "trade_identity",
                            "matching verified fill",
                            "missing",
                            "trade bill has no matching fill in the guarded fill collection",
                        ),
                        &mut failures,
                    );
                    continue;
                };
                counts.trade_bills_matched += 1;
                matched_fill_keys.insert(key);
                if validate_trade_bill(
                    bill,
                    fill,
                    &sources.config,
                    &sources.account_id,
                    &options,
                    &mut failures,
                    &mut issues,
                ) {
                    counts.trade_bills_validated += 1;
                }
            }
            "8" => {
                counts.funding_bills += 1;
                let sample = validate_funding_bill(
                    bill,
                    &valid_settlements,
                    &valid_runtime_sessions,
                    &sources.position_observations,
                    &valid_mark_prices,
                    &sources.config,
                    &sources.config_fingerprint,
                    &sources.account_identity_sha256,
                    &sources.account_id,
                    &options,
                    &mut counts,
                    &mut failures,
                    &mut issues,
                );
                if let Some(sample) = sample {
                    if funding_samples.len() < MAX_ECONOMIC_FUNDING_SAMPLES {
                        funding_samples.push(sample);
                    } else {
                        funding_samples_omitted = funding_samples_omitted.saturating_add(1);
                    }
                }
            }
            _ => {
                counts.unsupported_bills += 1;
                issues.push(
                    EconomicReconciliationFailure::UnsupportedBills,
                    issue_for_bill(
                        EconomicIssueSource::BillCollection,
                        bill,
                        "type",
                        "2 (trade) or 8 (funding)",
                        &bill.bill_type,
                        "controlled strategy window contains an unexplained balance-changing bill",
                    ),
                    &mut failures,
                );
            }
        }
    }

    for fill in &sources.fills {
        if !(options.begin_ms..=completeness_end).contains(&fill.ts_ms) {
            continue;
        }
        let key = (fill.symbol.clone(), fill.fill_id.clone());
        if !matched_fill_keys.contains(&key) {
            counts.eligible_fills_missing_bill += 1;
            issues.push(
                EconomicReconciliationFailure::EligibleFillsMissingBills,
                issue(
                    EconomicIssueSource::FillCollection,
                    None,
                    Some(&fill.symbol),
                    Some(&fill.fill_id),
                    "trade_bill",
                    "matching account bill inside the closed window",
                    "missing",
                    "interior fill has no matching account trade bill",
                ),
                &mut failures,
            );
        }
    }

    if counts.trade_bills_validated < options.minimum_trade_bills {
        failures.insert(EconomicReconciliationFailure::MinimumTradeBillsNotMet);
    }
    if counts.funding_bills_validated < options.minimum_funding_bills {
        failures.insert(EconomicReconciliationFailure::MinimumFundingBillsNotMet);
    }
    counts.issues_total = issues.total;
    counts.issues_reported = issues.issues.len() as u64;
    let issues_truncated = issues.total > issues.issues.len() as u64;
    let failures = failures.into_iter().collect::<Vec<_>>();
    let passed = failures.is_empty();

    EconomicReconciliationReport {
        schema_version: ECONOMIC_RECONCILIATION_SCHEMA_VERSION,
        scope: EconomicReconciliationScope::NormalTradeAndFundingBills,
        java_reference_revision: PINNED_JAVA_REVISION.to_string(),
        reap_version: env!("CARGO_PKG_VERSION").to_string(),
        executable_sha256,
        account_id: options.account_id,
        environment: sources.environment,
        account_identity_sha256: sources.account_identity_sha256,
        strategy_name: sources.config.strategy.strategy_name.clone(),
        config_fingerprint: sources.config_fingerprint,
        window: sources.window,
        minimum_trade_bills: options.minimum_trade_bills,
        minimum_funding_bills: options.minimum_funding_bills,
        maximum_trade_bill_delay_ms: options.maximum_trade_bill_delay_ms,
        maximum_funding_bill_delay_ms: options.maximum_funding_bill_delay_ms,
        maximum_funding_mark_bracket_distance_ms: options
            .maximum_funding_mark_bracket_distance_ms,
        tolerances: options.tolerances,
        config_file: sources.config_file,
        journal: sources.journal,
        journal_recovery: EconomicJournalRecoveryEvidence {
            records: sources.recovered.records,
            ignored_truncated_tail: sources.recovered.ignored_truncated_tail,
            account_bootstrap_records: sources.account_bootstrap_records,
            runtime_session_records: sources.runtime_sessions.len() as u64,
            funding_settlement_records: sources.settlements.len() as u64,
            position_observation_records: sources.position_observations.len() as u64,
            mark_price_observation_records: sources.mark_price_observations.len() as u64,
            exclusive_lease_held_while_reading: true,
        },
        fill_collection_manifest: sources.fill_manifest_file,
        bill_collection_manifest: sources.bill_manifest_file,
        counts,
        funding_formula_samples: funding_samples,
        funding_formula_samples_omitted: funding_samples_omitted,
        issues: issues.issues,
        issues_truncated,
        limitations: vec![
            "realized trade PnL is checked against each derivative bill's balance equation but is not independently recomputed because the journal does not yet retain an attested opening cost basis".to_string(),
            "funding checks the bill-reported mark against journaled observations bracketing the exchange-reported assessment time; the exact internal venue assessment tick is not reproduced".to_string(),
            "runtime-session boundaries are locally journaled provenance that prevents cross-restart evidence composition; they are not remote process attestation".to_string(),
            "settlements with no funding bill are not failures because a zero position legitimately produces no balance change; minimum matched funding evidence is required instead".to_string(),
            "the final trade-delay guard is excluded from fill-to-bill completeness because its bills may fall after the closed account-bill window".to_string(),
        ],
        failures,
        passed,
    }
}

fn validate_journal_identity(
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

fn validate_runtime_sessions<'a>(
    sources: &'a BoundEconomicSources,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> Vec<&'a JournalRuntimeSession> {
    let mut seen = BTreeSet::new();
    let mut valid = Vec::new();
    for session in &sources.runtime_sessions {
        let identity_valid = session.started_at_ms > 0
            && is_runtime_session_id(&session.session_id)
            && !session.account_id.is_empty()
            && session.account_id.trim() == session.account_id
            && sources.config.account(&session.account_id).is_some()
            && !session.strategy_name.is_empty()
            && session.strategy_name == sources.config.strategy.strategy_name
            && session.config_fingerprint == sources.config_fingerprint
            && is_lower_sha256(&session.config_fingerprint)
            && is_lower_sha256(&session.account_identity_sha256);
        if !identity_valid {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateRuntimeSessions,
                issue(
                    EconomicIssueSource::Journal,
                    None,
                    None,
                    None,
                    "runtime_session",
                    "configured account/strategy and valid session/config/account identities",
                    &format!(
                        "line {}, started_at={}, session_id={}, account={}",
                        session.line, session.started_at_ms, session.session_id, session.account_id
                    ),
                    "journal contains a malformed or foreign runtime-session boundary",
                ),
                failures,
            );
            continue;
        }
        if !seen.insert((session.account_id.clone(), session.session_id.clone())) {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateRuntimeSessions,
                issue(
                    EconomicIssueSource::Journal,
                    None,
                    None,
                    None,
                    "runtime_session",
                    "one session-start record per account/session id",
                    &format!("duplicate at line {}", session.line),
                    "journal contains a duplicate runtime-session boundary",
                ),
                failures,
            );
            continue;
        }
        valid.push(session);
    }
    valid
}

fn is_runtime_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn runtime_session_for_line<'a>(
    sessions: &[&'a JournalRuntimeSession],
    account_id: &str,
    line: u64,
) -> Option<&'a JournalRuntimeSession> {
    sessions
        .iter()
        .copied()
        .filter(|session| session.account_id == account_id && session.line < line)
        .max_by_key(|session| session.line)
}

fn validate_funding_settlements<'a>(
    sources: &'a BoundEconomicSources,
    runtime_sessions: &[&JournalRuntimeSession],
    options: &EconomicReconciliationOptions,
    counts: &mut EconomicReconciliationCounts,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> Vec<&'a JournalFundingSettlement> {
    let mut seen = BTreeSet::new();
    let mut valid = Vec::new();
    let relevant_begin = options
        .begin_ms
        .saturating_sub(options.maximum_funding_bill_delay_ms);
    for settlement in &sources.settlements {
        let configured_swap = instrument(&sources.config, &settlement.symbol)
            .is_some_and(|instrument| instrument.kind.is_swap());
        if configured_swap
            && (relevant_begin..=options.end_ms).contains(&settlement.funding_time_ms)
        {
            counts.funding_settlements_relevant += 1;
        }
        if settlement.symbol.is_empty()
            || settlement.funding_time_ms == 0
            || !settlement.rate.is_finite()
            || settlement.event_ts_ms == 0
            || settlement.event_ts_ms < settlement.funding_time_ms
            || settlement.event_ts_ms - settlement.funding_time_ms
                > options.maximum_funding_bill_delay_ms
        {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateFundingSettlements,
                issue(
                    EconomicIssueSource::Journal,
                    None,
                    Some(&settlement.symbol),
                    None,
                    "funding_settlement",
                    "non-empty symbol, finite rate, and observation inside the post-settlement delay",
                    &format!(
                        "line {}, event_ts={}, funding_time={}, rate={}",
                        settlement.line,
                        settlement.event_ts_ms,
                        settlement.funding_time_ms,
                        settlement.rate
                    ),
                    "journal contains an invalid settled funding observation",
                ),
                failures,
            );
            continue;
        }
        let session_id =
            runtime_session_for_line(runtime_sessions, &sources.account_id, settlement.line)
                .map_or("legacy", |session| session.session_id.as_str());
        let key = (
            session_id.to_string(),
            settlement.symbol.clone(),
            settlement.funding_time_ms,
        );
        if !seen.insert(key) {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateFundingSettlements,
                issue(
                    EconomicIssueSource::Journal,
                    None,
                    Some(&settlement.symbol),
                    None,
                    "funding_settlement",
                    "one normalized settlement per runtime session/symbol/time",
                    &format!("duplicate at line {}", settlement.line),
                    "journal funding deduplication did not produce a unique settlement",
                ),
                failures,
            );
            continue;
        }
        valid.push(settlement);
    }
    valid
}

fn validate_funding_mark_prices<'a>(
    sources: &'a BoundEconomicSources,
    runtime_sessions: &[&JournalRuntimeSession],
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> Vec<&'a JournalMarkPriceObservation> {
    let mut seen = BTreeSet::new();
    let mut valid = Vec::new();
    for observation in &sources.mark_price_observations {
        if observation.symbol.is_empty()
            || observation.event_ts_ms == 0
            || !observation.price.is_finite()
            || observation.price <= 0.0
        {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateFundingMarks,
                issue(
                    EconomicIssueSource::Journal,
                    None,
                    Some(&observation.symbol),
                    None,
                    "mark_price",
                    "non-empty symbol and positive finite exchange-time mark",
                    &format!(
                        "line {}, event_ts={}, price={}",
                        observation.line, observation.event_ts_ms, observation.price
                    ),
                    "journal contains an invalid mark-price observation",
                ),
                failures,
            );
            continue;
        }
        let session_id =
            runtime_session_for_line(runtime_sessions, &sources.account_id, observation.line)
                .map_or("legacy", |session| session.session_id.as_str());
        let key = (
            session_id.to_string(),
            observation.symbol.clone(),
            observation.event_ts_ms,
        );
        if !seen.insert(key) {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateFundingMarks,
                issue(
                    EconomicIssueSource::Journal,
                    None,
                    Some(&observation.symbol),
                    None,
                    "mark_price",
                    "one normalized mark per runtime session/symbol/exchange timestamp",
                    &format!("duplicate at line {}", observation.line),
                    "journal mark-price deduplication did not produce a unique observation",
                ),
                failures,
            );
            continue;
        }
        valid.push(observation);
    }
    valid
}

#[allow(clippy::too_many_arguments)]
fn validate_trade_bill(
    bill: &OkxBill,
    fill: &RemoteFill,
    config: &LiveConfig,
    account_id: &str,
    options: &EconomicReconciliationOptions,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> bool {
    let before = issues.total;
    let Some(instrument) = instrument(config, &bill.symbol) else {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "instId",
            "configured strategy instrument",
            &bill.symbol,
            "trade bill references an instrument outside the exact live config",
        );
        return false;
    };
    let expected_side = trade_subtype_side(&bill.sub_type);
    if expected_side.is_none()
        || (instrument.kind.is_spot() && !matches!(bill.sub_type.as_str(), "1" | "2"))
    {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "subType",
            if instrument.kind.is_spot() {
                "1 or 2"
            } else {
                "1 through 6"
            },
            &bill.sub_type,
            "trade bill subtype is not a supported normal strategy trade",
        );
    }
    if let Some(expected_side) = expected_side
        && fill.side != expected_side
    {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "side",
            side_name(expected_side),
            side_name(fill.side),
            "bill subtype side does not match the verified fill",
        );
    }
    compare_text(
        bill,
        "ordId",
        &fill.exchange_order_id,
        &bill.order_id,
        failures,
        issues,
    );
    if fill.client_order_id.is_empty() || bill.client_order_id.is_empty() {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "clOrdId",
            "non-empty Reap client order id",
            &bill.client_order_id,
            "normal strategy trade must retain its client order identity",
        );
    } else {
        compare_text(
            bill,
            "clOrdId",
            &fill.client_order_id,
            &bill.client_order_id,
            failures,
            issues,
        );
        if let Some(account) = config.account(account_id)
            && !bill.client_order_id.starts_with(&account.id_prefix)
        {
            push_bill_issue(
                failures,
                issues,
                EconomicReconciliationFailure::InvalidTradeBills,
                bill,
                "clOrdId",
                &format!("prefix {}", account.id_prefix),
                &bill.client_order_id,
                "trade bill is not attributable to the configured Reap client-id namespace",
            );
        }
    }
    let expected_instrument_type = instrument_type(instrument.kind);
    if bill.instrument_type != Some(expected_instrument_type) {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "instType",
            expected_instrument_type.as_str(),
            bill.instrument_type
                .map_or("missing", OkxInstrumentType::as_str),
            "bill instrument type does not match the configured contract model",
        );
    }
    match expected_bill_margin_mode(config, account_id, &bill.symbol) {
        Some(expected_margin) if bill.margin_mode != Some(expected_margin) => push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "mgnMode",
            margin_mode_name(expected_margin),
            bill.margin_mode.map_or("missing", margin_mode_name),
            "trade bill margin mode does not match the configured account trade mode",
        ),
        None => push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "mgnMode",
            "configured account trade mode",
            bill.margin_mode.map_or("missing", margin_mode_name),
            "trade bill cannot be bound to an account trade-mode configuration",
        ),
        Some(_) => {}
    }
    let expected_execution = match fill.liquidity {
        FillLiquidity::Maker => OkxBillExecutionType::Maker,
        FillLiquidity::Taker => OkxBillExecutionType::Taker,
    };
    if bill.execution_type != Some(expected_execution) {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "execType",
            execution_name(expected_execution),
            bill.execution_type.map_or("missing", execution_name),
            "bill liquidity does not match the verified fill",
        );
    }
    let fill_time_ms = bill.fill_time_ms;
    match fill_time_ms {
        Some(fill_time_ms) => {
            if fill_time_ms != fill.ts_ms {
                push_bill_issue(
                    failures,
                    issues,
                    EconomicReconciliationFailure::InvalidTradeBills,
                    bill,
                    "fillTime",
                    &fill.ts_ms.to_string(),
                    &fill_time_ms.to_string(),
                    "bill fill timestamp does not match the verified fill",
                );
            }
            if bill.timestamp_ms < fill_time_ms
                || bill.timestamp_ms - fill_time_ms > options.maximum_trade_bill_delay_ms
            {
                push_bill_issue(
                    failures,
                    issues,
                    EconomicReconciliationFailure::InvalidTradeBills,
                    bill,
                    "ts",
                    &format!(
                        "fillTime..=fillTime+{}",
                        options.maximum_trade_bill_delay_ms
                    ),
                    &bill.timestamp_ms.to_string(),
                    "trade bill completion time is outside the bounded causal delay",
                );
            }
        }
        None => push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "fillTime",
            &fill.ts_ms.to_string(),
            "missing",
            "trade bill does not retain an exact fill timestamp",
        ),
    }
    compare_number(
        bill,
        "px",
        fill.price,
        bill.price,
        options.tolerances.price_abs,
        failures,
        issues,
    );

    let expected_currency = if instrument.kind.is_spot() {
        match expected_side {
            Some(Side::Buy) => instrument.base_currency.as_str(),
            Some(Side::Sell) => instrument.quote_currency.as_str(),
            None => "",
        }
    } else {
        instrument.settle_currency.as_str()
    }
    .to_ascii_uppercase();
    if expected_currency.is_empty() || bill.currency != expected_currency {
        let expected = if expected_currency.is_empty() {
            "configured accounting currency"
        } else {
            &expected_currency
        };
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "ccy",
            expected,
            &bill.currency,
            "trade bill currency does not match the configured received/settlement currency",
        );
    }
    let expected_quantity = if instrument.kind.is_spot() {
        match expected_side {
            Some(Side::Buy) => Some(fill.qty),
            Some(Side::Sell) => Some(fill.qty * fill.price),
            None => None,
        }
    } else {
        Some(fill.qty)
    };
    if let Some(expected_quantity) = expected_quantity {
        compare_number(
            bill,
            "sz",
            expected_quantity,
            bill.quantity,
            options.tolerances.quantity_abs,
            failures,
            issues,
        );
    }

    match (&fill.fee, bill.fee) {
        (Some(fill_fee), Some(bill_fee)) => {
            if fill_fee.currency.trim().to_ascii_uppercase() != bill.currency {
                push_bill_issue(
                    failures,
                    issues,
                    EconomicReconciliationFailure::InvalidTradeBills,
                    bill,
                    "feeCcy",
                    &fill_fee.currency.trim().to_ascii_uppercase(),
                    &bill.currency,
                    "bill currency does not match the verified fill fee currency",
                );
            }
            compare_number_value(
                bill,
                "fee",
                fill_fee.amount,
                bill_fee,
                options.tolerances.fee_abs,
                EconomicReconciliationFailure::InvalidTradeBills,
                failures,
                issues,
            );
        }
        _ => push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "fee",
            "exact fee on both bill and verified fill",
            "missing",
            "trade economics cannot be accepted without exact signed fee evidence",
        ),
    }

    if let Some(interest) = bill.interest
        && !close_abs(interest, 0.0, options.tolerances.balance_abs)
    {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "interest",
            "0",
            &interest.to_string(),
            "normal controlled strategy trade unexpectedly accrued interest",
        );
    }
    if bill.from_account.is_some() || bill.to_account.is_some() {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "from/to",
            "empty for non-transfer trade",
            "populated",
            "trade bill unexpectedly identifies an account transfer",
        );
    }
    let expected_balance_change = match (bill.fee, instrument.kind.is_spot()) {
        (Some(fee), true) => bill.quantity.map(|quantity| quantity + fee),
        (Some(fee), false) => bill.pnl.map(|pnl| pnl + fee),
        _ => None,
    };
    if let Some(expected_balance_change) = expected_balance_change.filter(|value| value.is_finite())
    {
        compare_number_value(
            bill,
            "balChg",
            expected_balance_change,
            bill.balance_change,
            options.tolerances.balance_abs,
            EconomicReconciliationFailure::InvalidTradeBills,
            failures,
            issues,
        );
    } else {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            if instrument.kind.is_spot() {
                "sz/fee"
            } else {
                "pnl/fee"
            },
            "complete finite balance equation inputs",
            "missing",
            "trade bill balance change cannot be checked for internal consistency",
        );
    }
    before == issues.total
}

#[allow(clippy::too_many_arguments)]
fn validate_funding_bill(
    bill: &OkxBill,
    settlements: &[&JournalFundingSettlement],
    runtime_sessions: &[&JournalRuntimeSession],
    position_observations: &[JournalPositionObservation],
    mark_price_observations: &[&JournalMarkPriceObservation],
    config: &LiveConfig,
    config_fingerprint: &str,
    account_identity_sha256: &str,
    account_id: &str,
    options: &EconomicReconciliationOptions,
    counts: &mut EconomicReconciliationCounts,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> Option<FundingFormulaSample> {
    let before = issues.total;
    let Some(instrument) = instrument(config, &bill.symbol) else {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "instId",
            "configured swap instrument",
            &bill.symbol,
            "funding bill references an instrument outside the exact live config",
        );
        return None;
    };
    if !instrument.kind.is_swap() || bill.instrument_type != Some(OkxInstrumentType::Swap) {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "instType",
            "configured SWAP",
            bill.instrument_type
                .map_or("missing", OkxInstrumentType::as_str),
            "funding bill is not for a configured swap contract",
        );
    }
    if !matches!(bill.sub_type.as_str(), "173" | "174") {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "subType",
            "173 (expense) or 174 (income)",
            &bill.sub_type,
            "funding bill subtype does not match the pinned Java mapping",
        );
    }
    let expected_currency = instrument.settle_currency.trim().to_ascii_uppercase();
    if expected_currency.is_empty() || bill.currency != expected_currency {
        let expected = if expected_currency.is_empty() {
            "configured settlement currency"
        } else {
            &expected_currency
        };
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "ccy",
            expected,
            &bill.currency,
            "funding bill currency does not match the configured settlement currency",
        );
    }
    if let Some(expected_margin) = expected_bill_margin_mode(config, account_id, &bill.symbol)
        && bill.margin_mode != Some(expected_margin)
    {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "mgnMode",
            margin_mode_name(expected_margin),
            bill.margin_mode.map_or("missing", margin_mode_name),
            "funding bill margin mode does not match the configured account trade mode",
        );
    }
    if !bill.trade_id.is_empty()
        || !bill.order_id.is_empty()
        || !bill.client_order_id.is_empty()
        || bill.execution_type.is_some()
    {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "trade_identity",
            "empty for funding",
            "populated",
            "funding bill unexpectedly carries a normal trade identity",
        );
    }
    if bill.from_account.is_some() || bill.to_account.is_some() {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "from/to",
            "empty for funding",
            "populated",
            "funding bill unexpectedly identifies an account transfer",
        );
    }
    let Some(assessment_time_ms) = bill.fill_time_ms else {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "fillTime",
            "funding assessment timestamp",
            "missing",
            "funding bill omits the timestamp needed to bind position and mark evidence",
        );
        return None;
    };
    if assessment_time_ms
        < bill
            .timestamp_ms
            .saturating_sub(options.maximum_funding_bill_delay_ms)
        || assessment_time_ms
            > bill
                .timestamp_ms
                .saturating_add(options.maximum_funding_bill_delay_ms)
    {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "fillTime",
            &format!(
                "within {} ms of bill ts",
                options.maximum_funding_bill_delay_ms
            ),
            &assessment_time_ms.to_string(),
            "funding assessment and balance-update timestamps are not causally close",
        );
    }
    if bill
        .fee
        .is_some_and(|fee| !close_abs(fee, 0.0, options.tolerances.fee_abs))
        || bill
            .interest
            .is_some_and(|interest| !close_abs(interest, 0.0, options.tolerances.balance_abs))
    {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "fee/interest",
            "0",
            &format!("fee={:?}, interest={:?}", bill.fee, bill.interest),
            "funding settlement unexpectedly contains a separate fee or interest charge",
        );
    }

    let candidates = settlements
        .iter()
        .copied()
        .filter(|settlement| {
            settlement.symbol == bill.symbol
                && settlement.funding_time_ms <= bill.timestamp_ms
                && bill.timestamp_ms - settlement.funding_time_ms
                    <= options.maximum_funding_bill_delay_ms
        })
        .collect::<Vec<_>>();
    let session_bound_candidates = candidates
        .iter()
        .copied()
        .filter(|settlement| {
            runtime_session_for_line(runtime_sessions, account_id, settlement.line).is_some_and(
                |session| {
                    session.config_fingerprint == config_fingerprint
                        && session.account_identity_sha256 == account_identity_sha256
                        && session.started_at_ms <= assessment_time_ms
                },
            )
        })
        .collect::<Vec<_>>();
    let settlement = match (candidates.as_slice(), session_bound_candidates.as_slice()) {
        ([settlement], _) | (_, [settlement]) => *settlement,
        _ => {
            issues.push(
                EconomicReconciliationFailure::FundingBillsMissingSettlements,
                issue_for_bill(
                    EconomicIssueSource::Journal,
                    bill,
                    "funding_settlement",
                    "exactly one session-bound journaled settled rate within the causal delay",
                    &format!(
                        "causal={}, session_bound={}",
                        candidates.len(),
                        session_bound_candidates.len()
                    ),
                    "funding bill cannot be bound to one normalized settled-rate source",
                ),
                failures,
            );
            return None;
        }
    };
    counts.funding_bills_matched += 1;
    if assessment_time_ms < settlement.funding_time_ms
        || assessment_time_ms - settlement.funding_time_ms > options.maximum_funding_bill_delay_ms
    {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "fillTime",
            &format!(
                "settlement..=settlement+{}",
                options.maximum_funding_bill_delay_ms
            ),
            &assessment_time_ms.to_string(),
            "funding assessment timestamp is outside the scheduled settlement delay",
        );
    }
    let runtime_session = runtime_session_for_line(runtime_sessions, account_id, settlement.line);
    let Some(runtime_session) = runtime_session.filter(|session| {
        session.config_fingerprint == config_fingerprint
            && session.account_identity_sha256 == account_identity_sha256
            && session.started_at_ms <= assessment_time_ms
    }) else {
        issues.push(
            EconomicReconciliationFailure::FundingSessionBoundaryMissing,
            issue_for_bill(
                EconomicIssueSource::Journal,
                bill,
                "runtime_session",
                "matching account/config/account-identity session start before assessment",
                "missing",
                "funding evidence cannot be tied to one explicitly journaled runtime session",
            ),
            failures,
        );
        return None;
    };
    let next_session_line = runtime_sessions
        .iter()
        .copied()
        .filter(|session| session.account_id == account_id && session.line > runtime_session.line)
        .map(|session| session.line)
        .min()
        .unwrap_or(u64::MAX);
    let position = position_observations
        .iter()
        .filter(|position| {
            position.symbol == bill.symbol
                && position.line > runtime_session.line
                && position.line < next_session_line
                && position.event_ts_ms <= assessment_time_ms
        })
        .max_by_key(|position| (position.event_ts_ms, position.line));
    let Some(position) = position else {
        issues.push(
            EconomicReconciliationFailure::FundingPositionMismatches,
            issue_for_bill(
                EconomicIssueSource::Journal,
                bill,
                "position",
                "latest same-session journaled position at or before funding assessment",
                "missing",
                "funding payment cannot be bound to an independently journaled position",
            ),
            failures,
        );
        return None;
    };
    if !position.quantity.is_finite() || position.quantity == 0.0 {
        issues.push(
            EconomicReconciliationFailure::FundingPositionMismatches,
            issue_for_bill(
                EconomicIssueSource::Journal,
                bill,
                "position_quantity",
                "finite non-zero signed position",
                &position.quantity.to_string(),
                "journaled position cannot explain a non-zero funding bill",
            ),
            failures,
        );
    }
    let Some(quantity) = bill
        .quantity
        .filter(|value| value.is_finite() && *value > 0.0)
    else {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "sz",
            "positive position quantity in contracts",
            &format!("{:?}", bill.quantity),
            "funding formula requires a positive contract quantity",
        );
        return None;
    };
    if !close_abs(
        position.quantity.abs(),
        quantity,
        options.tolerances.quantity_abs,
    ) {
        issues.push(
            EconomicReconciliationFailure::FundingPositionMismatches,
            issue_for_bill(
                EconomicIssueSource::Journal,
                bill,
                "position_quantity",
                &position.quantity.abs().to_string(),
                &quantity.to_string(),
                "funding bill quantity does not match the latest journaled position",
            ),
            failures,
        );
    }
    let Some(bill_mark_price) = bill.price.filter(|value| value.is_finite() && *value > 0.0) else {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "px",
            "positive settlement mark price",
            &format!("{:?}", bill.price),
            "funding formula requires the exchange-reported assessment mark for independent comparison",
        );
        return None;
    };
    let mark_before = mark_price_observations
        .iter()
        .copied()
        .filter(|observation| {
            observation.symbol == bill.symbol
                && observation.line > runtime_session.line
                && observation.line < next_session_line
                && observation.event_ts_ms <= assessment_time_ms
                && assessment_time_ms - observation.event_ts_ms
                    <= options.maximum_funding_mark_bracket_distance_ms
        })
        .max_by_key(|observation| (observation.event_ts_ms, observation.line));
    let mark_after = mark_price_observations
        .iter()
        .copied()
        .filter(|observation| {
            observation.symbol == bill.symbol
                && observation.line > runtime_session.line
                && observation.line < next_session_line
                && observation.event_ts_ms >= assessment_time_ms
                && observation.event_ts_ms - assessment_time_ms
                    <= options.maximum_funding_mark_bracket_distance_ms
        })
        .min_by_key(|observation| (observation.event_ts_ms, observation.line));
    let (Some(mark_before), Some(mark_after)) = (mark_before, mark_after) else {
        issues.push(
            EconomicReconciliationFailure::FundingMarkBracketsMissing,
            issue_for_bill(
                EconomicIssueSource::Journal,
                bill,
                "mark_price_bracket",
                &format!(
                    "same-session marks on both sides of fillTime within {} ms",
                    options.maximum_funding_mark_bracket_distance_ms
                ),
                &format!(
                    "before={}, after={}",
                    mark_before.is_some(),
                    mark_after.is_some()
                ),
                "funding assessment cannot be compared with a two-sided journaled mark bracket",
            ),
            failures,
        );
        return None;
    };
    let mark_lower_bound = mark_before.price.min(mark_after.price);
    let mark_upper_bound = mark_before.price.max(mark_after.price);
    let mark_scale = bill_mark_price
        .abs()
        .max(mark_lower_bound.abs())
        .max(mark_upper_bound.abs());
    let mark_effective_tolerance = options
        .tolerances
        .funding_mark_abs
        .max(options.tolerances.funding_mark_relative * mark_scale);
    let mark_valid = mark_effective_tolerance.is_finite()
        && bill_mark_price >= mark_lower_bound - mark_effective_tolerance
        && bill_mark_price <= mark_upper_bound + mark_effective_tolerance;
    if mark_valid {
        counts.funding_mark_brackets_validated += 1;
    } else {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::FundingMarkMismatches,
            bill,
            "px",
            &format!(
                "{}..={} +/- {} from journaled mark bracket",
                mark_lower_bound, mark_upper_bound, mark_effective_tolerance
            ),
            &bill_mark_price.to_string(),
            "funding bill mark lies outside the independently journaled assessment bracket",
        );
    }
    let Some(pnl) = bill.pnl.filter(|value| value.is_finite() && *value != 0.0) else {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "pnl",
            "non-zero signed funding payment",
            &format!("{:?}", bill.pnl),
            "funding bill does not contain a signed payment",
        );
        return None;
    };
    if (bill.sub_type == "173" && pnl >= 0.0) || (bill.sub_type == "174" && pnl <= 0.0) {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "pnl_sign",
            if bill.sub_type == "173" {
                "negative expense"
            } else {
                "positive income"
            },
            &pnl.to_string(),
            "funding payment sign contradicts the pinned Java/OKX subtype",
        );
    }
    if !close_abs(bill.balance_change, pnl, options.tolerances.balance_abs) {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "balChg",
            &pnl.to_string(),
            &bill.balance_change.to_string(),
            "funding balance change does not equal the reported funding PnL",
        );
    }

    let expected_pnl_at_bill_mark = funding_pnl_at_mark(
        position.quantity,
        instrument.contract_value,
        settlement.rate,
        bill_mark_price,
        instrument.kind.is_inverse(),
    );
    let expected_pnl_at_mark_before = funding_pnl_at_mark(
        position.quantity,
        instrument.contract_value,
        settlement.rate,
        mark_before.price,
        instrument.kind.is_inverse(),
    );
    let expected_pnl_at_mark_after = funding_pnl_at_mark(
        position.quantity,
        instrument.contract_value,
        settlement.rate,
        mark_after.price,
        instrument.kind.is_inverse(),
    );
    let expected_pnl_lower_bound = expected_pnl_at_mark_before.min(expected_pnl_at_mark_after);
    let expected_pnl_upper_bound = expected_pnl_at_mark_before.max(expected_pnl_at_mark_after);
    let expected_pnl_absolute = expected_pnl_lower_bound
        .abs()
        .max(expected_pnl_upper_bound.abs());
    let absolute_difference = if pnl < expected_pnl_lower_bound {
        expected_pnl_lower_bound - pnl
    } else if pnl > expected_pnl_upper_bound {
        pnl - expected_pnl_upper_bound
    } else {
        0.0
    };
    let relative_difference =
        absolute_difference / expected_pnl_absolute.max(pnl.abs()).max(f64::MIN_POSITIVE);
    let effective_tolerance = options
        .tolerances
        .funding_pnl_abs
        .max(options.tolerances.funding_pnl_relative * expected_pnl_absolute);
    let formula_valid = expected_pnl_at_bill_mark.is_finite()
        && expected_pnl_lower_bound.is_finite()
        && expected_pnl_upper_bound.is_finite()
        && absolute_difference.is_finite()
        && absolute_difference <= effective_tolerance;
    if !formula_valid {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::FundingFormulaMismatches,
            bill,
            "pnl_formula",
            &format!(
                "{}..={} +/- {}",
                expected_pnl_lower_bound, expected_pnl_upper_bound, effective_tolerance
            ),
            &pnl.to_string(),
            "funding payment does not match the configured contract formula, journaled signed position/rate, and independent mark bracket",
        );
    }
    let validated = before == issues.total && mark_valid && formula_valid;
    if validated {
        counts.funding_bills_validated += 1;
    }
    Some(FundingFormulaSample {
        bill_id: bill.bill_id.clone(),
        symbol: bill.symbol.clone(),
        runtime_session_id: runtime_session.session_id.clone(),
        runtime_session_start_line: runtime_session.line,
        runtime_session_started_at_ms: runtime_session.started_at_ms,
        bill_timestamp_ms: bill.timestamp_ms,
        settlement_time_ms: settlement.funding_time_ms,
        settlement_delay_ms: bill.timestamp_ms - settlement.funding_time_ms,
        assessment_time_ms,
        assessment_delay_ms: assessment_time_ms.saturating_sub(settlement.funding_time_ms),
        rate: settlement.rate,
        inverse: instrument.kind.is_inverse(),
        currency: bill.currency.clone(),
        quantity,
        journal_position_quantity: position.quantity,
        position_observation_line: position.line,
        position_observation_time_ms: position.event_ts_ms,
        contract_value: instrument.contract_value,
        bill_mark_price,
        mark_before_line: mark_before.line,
        mark_before_time_ms: mark_before.event_ts_ms,
        mark_before_price: mark_before.price,
        mark_after_line: mark_after.line,
        mark_after_time_ms: mark_after.event_ts_ms,
        mark_after_price: mark_after.price,
        mark_lower_bound,
        mark_upper_bound,
        mark_effective_tolerance,
        mark_validated: mark_valid,
        expected_pnl_at_bill_mark,
        expected_pnl_lower_bound,
        expected_pnl_upper_bound,
        expected_pnl_absolute,
        observed_pnl: pnl,
        absolute_difference,
        relative_difference,
        effective_tolerance,
        validated,
    })
}

fn funding_pnl_at_mark(
    signed_position: f64,
    contract_value: f64,
    rate: f64,
    mark_price: f64,
    inverse: bool,
) -> f64 {
    if inverse {
        -(signed_position * contract_value * rate / mark_price)
    } else {
        -(signed_position * contract_value * rate * mark_price)
    }
}

fn instrument<'a>(config: &'a LiveConfig, symbol: &str) -> Option<&'a InstrumentConfig> {
    config
        .strategy
        .instruments
        .iter()
        .find(|instrument| instrument.symbol == symbol)
}

fn instrument_type(kind: InstrumentKindConfig) -> OkxInstrumentType {
    match kind {
        InstrumentKindConfig::Spot => OkxInstrumentType::Spot,
        InstrumentKindConfig::Future
        | InstrumentKindConfig::LinearFuture
        | InstrumentKindConfig::InverseFuture => OkxInstrumentType::Futures,
        InstrumentKindConfig::LinearSwap | InstrumentKindConfig::InverseSwap => {
            OkxInstrumentType::Swap
        }
    }
}

fn expected_bill_margin_mode(
    config: &LiveConfig,
    account_id: &str,
    symbol: &str,
) -> Option<OkxBillMarginMode> {
    let account = config.account(account_id)?;
    match account.trade_mode(symbol)? {
        OkxTradeMode::Cash => Some(OkxBillMarginMode::Cash),
        OkxTradeMode::Cross => Some(OkxBillMarginMode::Cross),
        OkxTradeMode::Isolated => Some(OkxBillMarginMode::Isolated),
    }
}

fn trade_subtype_side(sub_type: &str) -> Option<Side> {
    match sub_type {
        "1" | "3" | "6" => Some(Side::Buy),
        "2" | "4" | "5" => Some(Side::Sell),
        _ => None,
    }
}

fn compare_text(
    bill: &OkxBill,
    field: &str,
    expected: &str,
    observed: &str,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) {
    if expected != observed {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            field,
            expected,
            observed,
            "trade bill field does not match the verified fill",
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn compare_number(
    bill: &OkxBill,
    field: &str,
    expected: f64,
    observed: Option<f64>,
    tolerance: f64,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) {
    let Some(observed) = observed else {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            field,
            &expected.to_string(),
            "missing",
            "trade bill omits a required numeric field",
        );
        return;
    };
    compare_number_value(
        bill,
        field,
        expected,
        observed,
        tolerance,
        EconomicReconciliationFailure::InvalidTradeBills,
        failures,
        issues,
    );
}

#[allow(clippy::too_many_arguments)]
fn compare_number_value(
    bill: &OkxBill,
    field: &str,
    expected: f64,
    observed: f64,
    tolerance: f64,
    failure: EconomicReconciliationFailure,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) {
    if !close_abs(expected, observed, tolerance) {
        push_bill_issue(
            failures,
            issues,
            failure,
            bill,
            field,
            &expected.to_string(),
            &observed.to_string(),
            &format!("absolute difference exceeds {tolerance}"),
        );
    }
}

fn close_abs(left: f64, right: f64, tolerance: f64) -> bool {
    left.is_finite() && right.is_finite() && (left - right).abs() <= tolerance
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[allow(clippy::too_many_arguments)]
fn push_bill_issue(
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
    failure: EconomicReconciliationFailure,
    bill: &OkxBill,
    field: &str,
    expected: &str,
    observed: &str,
    message: &str,
) {
    issues.push(
        failure,
        issue_for_bill(
            EconomicIssueSource::BillCollection,
            bill,
            field,
            expected,
            observed,
            message,
        ),
        failures,
    );
}

fn issue_for_bill(
    source: EconomicIssueSource,
    bill: &OkxBill,
    field: &str,
    expected: &str,
    observed: &str,
    message: &str,
) -> EconomicIssue {
    issue(
        source,
        Some(&bill.bill_id),
        (!bill.symbol.is_empty()).then_some(bill.symbol.as_str()),
        (!bill.trade_id.is_empty()).then_some(bill.trade_id.as_str()),
        field,
        expected,
        observed,
        message,
    )
}

#[allow(clippy::too_many_arguments)]
fn issue(
    source: EconomicIssueSource,
    bill_id: Option<&str>,
    symbol: Option<&str>,
    trade_id: Option<&str>,
    field: &str,
    expected: &str,
    observed: &str,
    message: &str,
) -> EconomicIssue {
    EconomicIssue {
        source,
        bill_id: bill_id.map(str::to_string),
        symbol: symbol.map(str::to_string),
        trade_id: trade_id.map(str::to_string),
        field: field.to_string(),
        expected: expected.to_string(),
        observed: observed.to_string(),
        message: message.to_string(),
    }
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Buy => "buy",
        Side::Sell => "sell",
    }
}

fn execution_name(execution: OkxBillExecutionType) -> &'static str {
    match execution {
        OkxBillExecutionType::Maker => "maker",
        OkxBillExecutionType::Taker => "taker",
    }
}

fn margin_mode_name(mode: OkxBillMarginMode) -> &'static str {
    match mode {
        OkxBillMarginMode::Cash => "cash",
        OkxBillMarginMode::Cross => "cross",
        OkxBillMarginMode::Isolated => "isolated",
    }
}

fn read_input(
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

#[cfg(test)]
mod tests {
    use reap_core::FillFee;

    use super::*;

    const BEGIN_MS: u64 = 1_000_000;
    const END_MS: u64 = 1_200_000;
    const TRADE_MS: u64 = 1_050_000;
    const FUNDING_MS: u64 = 1_100_000;

    fn config() -> LiveConfig {
        LiveConfig::from_toml(include_str!("../../../examples/live-okx-demo.toml")).unwrap()
    }

    fn options() -> EconomicReconciliationOptions {
        EconomicReconciliationOptions {
            account_id: "main".to_string(),
            begin_ms: BEGIN_MS,
            end_ms: END_MS,
            minimum_trade_bills: 1,
            minimum_funding_bills: 1,
            maximum_trade_bill_delay_ms: 10_000,
            maximum_funding_bill_delay_ms: 10_000,
            maximum_funding_mark_bracket_distance_ms: 1_000,
            tolerances: EconomicReconciliationTolerances {
                price_abs: 0.0,
                quantity_abs: 1e-9,
                fee_abs: 1e-12,
                balance_abs: 1e-12,
                funding_pnl_abs: 1e-12,
                funding_pnl_relative: 1e-12,
                funding_mark_abs: 0.0,
                funding_mark_relative: 0.0,
            },
        }
    }

    fn evidence(path: &str) -> FillCollectionFileEvidence {
        FillCollectionFileEvidence {
            path: path.to_string(),
            bytes: 1,
            sha256: "a".repeat(64),
        }
    }

    fn swap_fill() -> RemoteFill {
        RemoteFill {
            fill_id: "trade-1".to_string(),
            exchange_order_id: "exchange-1".to_string(),
            client_order_id: "reap-1".to_string(),
            symbol: "BTC-USDT-SWAP".to_string(),
            side: Side::Buy,
            price: 50_000.0,
            qty: 10.0,
            liquidity: FillLiquidity::Taker,
            fee: Some(FillFee {
                amount: -2.5,
                currency: "USDT".to_string(),
            }),
            ts_ms: TRADE_MS,
        }
    }

    fn trade_bill() -> OkxBill {
        OkxBill {
            bill_id: "bill-trade".to_string(),
            bill_type: "2".to_string(),
            sub_type: "3".to_string(),
            timestamp_ms: TRADE_MS + 1,
            currency: "USDT".to_string(),
            balance_change: -2.5,
            balance: Some(1_000.0),
            position_balance_change: Some(0.0),
            position_balance: Some(0.0),
            quantity: Some(10.0),
            price: Some(50_000.0),
            pnl: Some(0.0),
            fee: Some(-2.5),
            interest: Some(0.0),
            instrument_type: Some(OkxInstrumentType::Swap),
            symbol: "BTC-USDT-SWAP".to_string(),
            margin_mode: Some(OkxBillMarginMode::Cross),
            order_id: "exchange-1".to_string(),
            client_order_id: "reap-1".to_string(),
            trade_id: "trade-1".to_string(),
            fill_time_ms: Some(TRADE_MS),
            execution_type: Some(OkxBillExecutionType::Taker),
            from_account: None,
            to_account: None,
            notes: String::new(),
        }
    }

    fn funding_bill() -> OkxBill {
        OkxBill {
            bill_id: "bill-funding".to_string(),
            bill_type: "8".to_string(),
            sub_type: "173".to_string(),
            timestamp_ms: FUNDING_MS + 100,
            currency: "USDT".to_string(),
            balance_change: -5.0,
            balance: Some(995.0),
            position_balance_change: Some(0.0),
            position_balance: Some(0.0),
            quantity: Some(10.0),
            price: Some(50_000.0),
            pnl: Some(-5.0),
            fee: Some(0.0),
            interest: Some(0.0),
            instrument_type: Some(OkxInstrumentType::Swap),
            symbol: "BTC-USDT-SWAP".to_string(),
            margin_mode: Some(OkxBillMarginMode::Cross),
            order_id: String::new(),
            client_order_id: String::new(),
            trade_id: String::new(),
            fill_time_ms: Some(FUNDING_MS + 100),
            execution_type: None,
            from_account: None,
            to_account: None,
            notes: String::new(),
        }
    }

    fn sources() -> BoundEconomicSources {
        let config = config();
        let fingerprint = config.fingerprint().unwrap();
        let strategy_name = config.strategy.strategy_name.clone();
        let mut recovered = RecoveredStorage {
            records: 7,
            ..RecoveredStorage::default()
        };
        recovered.bootstrap_identities.insert(
            "main".to_string(),
            (config.strategy.strategy_name.clone(), fingerprint.clone()),
        );
        BoundEconomicSources {
            account_id: "main".to_string(),
            config,
            config_file: evidence("/config"),
            journal: evidence("/journal"),
            recovered,
            account_bootstrap_records: 1,
            runtime_sessions: vec![JournalRuntimeSession {
                line: 2,
                started_at_ms: BEGIN_MS - 1_000,
                session_id: "1a2b3c".to_string(),
                account_id: "main".to_string(),
                strategy_name,
                config_fingerprint: fingerprint.clone(),
                account_identity_sha256: "b".repeat(64),
            }],
            settlements: vec![JournalFundingSettlement {
                line: 4,
                event_ts_ms: FUNDING_MS + 50,
                symbol: "BTC-USDT-SWAP".to_string(),
                funding_time_ms: FUNDING_MS,
                rate: 0.001,
            }],
            position_observations: vec![
                JournalPositionObservation {
                    line: 3,
                    event_ts_ms: FUNDING_MS - 100,
                    symbol: "BTC-USDT-SWAP".to_string(),
                    quantity: 9.0,
                },
                JournalPositionObservation {
                    line: 5,
                    event_ts_ms: FUNDING_MS + 75,
                    symbol: "BTC-USDT-SWAP".to_string(),
                    quantity: 10.0,
                },
            ],
            mark_price_observations: vec![
                JournalMarkPriceObservation {
                    line: 6,
                    event_ts_ms: FUNDING_MS + 90,
                    symbol: "BTC-USDT-SWAP".to_string(),
                    price: 50_000.0,
                },
                JournalMarkPriceObservation {
                    line: 7,
                    event_ts_ms: FUNDING_MS + 110,
                    symbol: "BTC-USDT-SWAP".to_string(),
                    price: 50_000.0,
                },
            ],
            fill_manifest_file: evidence("/fills"),
            bill_manifest_file: evidence("/bills"),
            fills: vec![swap_fill()],
            bills: vec![trade_bill(), funding_bill()],
            environment: TradingEnvironment::Demo,
            account_identity_sha256: "b".repeat(64),
            config_fingerprint: fingerprint,
            window: BillCollectionWindow {
                begin_ms: BEGIN_MS,
                end_ms: END_MS,
                endpoints_inclusive: true,
                minimum_close_delay_ms: 1,
            },
        }
    }

    #[test]
    fn validates_normal_trade_and_linear_funding_from_exact_sources() {
        let report = build_report(sources(), options(), "c".repeat(64));

        assert!(report.passed, "{:?}", report.issues);
        assert!(report.failures.is_empty());
        assert_eq!(report.counts.trade_bills_validated, 1);
        assert_eq!(report.counts.funding_bills_validated, 1);
        assert_eq!(report.counts.eligible_fills_missing_bill, 0);
        assert_eq!(report.funding_formula_samples.len(), 1);
        assert_eq!(report.journal_recovery.position_observation_records, 2);
        assert_eq!(report.journal_recovery.mark_price_observation_records, 2);
        assert_eq!(report.journal_recovery.runtime_session_records, 1);
        assert_eq!(report.counts.funding_mark_brackets_validated, 1);
        assert_eq!(
            report.funding_formula_samples[0].expected_pnl_at_bill_mark,
            -5.0
        );
        assert_eq!(report.funding_formula_samples[0].expected_pnl_absolute, 5.0);
        assert_eq!(
            report.funding_formula_samples[0].journal_position_quantity,
            10.0
        );
        assert_eq!(
            report.funding_formula_samples[0].position_observation_line,
            5
        );
        assert_eq!(
            report.funding_formula_samples[0].runtime_session_id,
            "1a2b3c"
        );
        assert_eq!(
            report.funding_formula_samples[0].runtime_session_start_line,
            2
        );
        assert!(report.funding_formula_samples[0].mark_validated);
        assert!(report.funding_formula_samples[0].validated);
    }

    #[test]
    fn funding_formula_tamper_fails_even_when_balance_equation_is_self_consistent() {
        let mut sources = sources();
        sources.bills[1].pnl = Some(-4.0);
        sources.bills[1].balance_change = -4.0;

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::FundingFormulaMismatches)
        );
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::MinimumFundingBillsNotMet)
        );
        assert_eq!(report.funding_formula_samples[0].expected_pnl_absolute, 5.0);
        assert_eq!(report.funding_formula_samples[0].observed_pnl, -4.0);
    }

    #[test]
    fn trade_bill_margin_mode_must_match_the_account_configuration() {
        let mut sources = sources();
        sources.bills[0].margin_mode = Some(OkxBillMarginMode::Isolated);

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::InvalidTradeBills)
        );
        assert!(report.issues.iter().any(|issue| issue.field == "mgnMode"));
    }

    #[test]
    fn unexplained_balance_changing_bill_fails_closed() {
        let mut sources = sources();
        let mut transfer = funding_bill();
        transfer.bill_id = "bill-transfer".to_string();
        transfer.bill_type = "1".to_string();
        transfer.sub_type = "11".to_string();
        sources.bills.push(transfer);

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert_eq!(report.counts.unsupported_bills, 1);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::UnsupportedBills)
        );
    }

    #[test]
    fn spot_sell_uses_quote_currency_quantity_and_balance_change() {
        let mut sources = sources();
        sources.fills[0] = RemoteFill {
            fill_id: "trade-spot".to_string(),
            exchange_order_id: "exchange-spot".to_string(),
            client_order_id: "reap-spot".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Sell,
            price: 50_000.0,
            qty: 0.01,
            liquidity: FillLiquidity::Maker,
            fee: Some(FillFee {
                amount: -0.05,
                currency: "USDT".to_string(),
            }),
            ts_ms: TRADE_MS,
        };
        sources.bills[0] = OkxBill {
            bill_id: "bill-spot".to_string(),
            bill_type: "2".to_string(),
            sub_type: "2".to_string(),
            timestamp_ms: TRADE_MS + 1,
            currency: "USDT".to_string(),
            balance_change: 499.95,
            balance: Some(2_000.0),
            position_balance_change: Some(0.0),
            position_balance: Some(0.0),
            quantity: Some(500.0),
            price: Some(50_000.0),
            pnl: Some(0.0),
            fee: Some(-0.05),
            interest: Some(0.0),
            instrument_type: Some(OkxInstrumentType::Spot),
            symbol: "BTC-USDT".to_string(),
            margin_mode: Some(OkxBillMarginMode::Cash),
            order_id: "exchange-spot".to_string(),
            client_order_id: "reap-spot".to_string(),
            trade_id: "trade-spot".to_string(),
            fill_time_ms: Some(TRADE_MS),
            execution_type: Some(OkxBillExecutionType::Maker),
            from_account: None,
            to_account: None,
            notes: String::new(),
        };

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(report.passed, "{:?}", report.issues);
        assert_eq!(report.counts.trade_bills_validated, 1);
    }

    #[test]
    fn duplicate_journal_settlement_is_rejected_before_formula_acceptance() {
        let mut sources = sources();
        sources.settlements.push(sources.settlements[0].clone());

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::InvalidOrDuplicateFundingSettlements)
        );
    }

    #[test]
    fn settled_rate_replay_after_restart_does_not_duplicate_prior_session() {
        let mut sources = sources();
        sources.runtime_sessions.push(JournalRuntimeSession {
            line: 8,
            started_at_ms: FUNDING_MS + 110,
            session_id: "4d5e6f".to_string(),
            account_id: "main".to_string(),
            strategy_name: sources.config.strategy.strategy_name.clone(),
            config_fingerprint: sources.config_fingerprint.clone(),
            account_identity_sha256: sources.account_identity_sha256.clone(),
        });
        let mut replay = sources.settlements[0].clone();
        replay.line = 9;
        replay.event_ts_ms = FUNDING_MS + 150;
        sources.settlements.push(replay);

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(report.passed, "{:?}", report.issues);
        assert_eq!(report.counts.funding_bills_validated, 1);
        assert_eq!(
            report.funding_formula_samples[0].runtime_session_id,
            "1a2b3c"
        );
    }

    #[test]
    fn funding_sign_is_recomputed_from_the_journaled_position() {
        let mut sources = sources();
        sources.position_observations[1].quantity = -10.0;

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::FundingFormulaMismatches)
        );
        assert_eq!(
            report.funding_formula_samples[0].expected_pnl_at_bill_mark,
            5.0
        );
        assert_eq!(report.funding_formula_samples[0].observed_pnl, -5.0);
    }

    #[test]
    fn funding_requires_a_matching_pre_assessment_journal_position() {
        let mut sources = sources();
        sources.position_observations.clear();

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::FundingPositionMismatches)
        );
        assert!(report.funding_formula_samples.is_empty());
    }

    #[test]
    fn funding_requires_marks_on_both_sides_of_the_assessment() {
        let mut sources = sources();
        sources.mark_price_observations.pop();

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::FundingMarkBracketsMissing)
        );
        assert!(report.funding_formula_samples.is_empty());
    }

    #[test]
    fn funding_bill_mark_must_lie_inside_the_journaled_bracket() {
        let mut sources = sources();
        sources.bills[1].price = Some(51_000.0);

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::FundingMarkMismatches)
        );
        assert!(!report.funding_formula_samples[0].mark_validated);
        assert_eq!(report.funding_formula_samples[0].absolute_difference, 0.0);
    }

    #[test]
    fn duplicate_journal_mark_timestamp_fails_closed() {
        let mut sources = sources();
        let mut duplicate = sources.mark_price_observations[0].clone();
        duplicate.line = 8;
        sources.mark_price_observations.push(duplicate);

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::InvalidOrDuplicateFundingMarks)
        );
    }

    #[test]
    fn mark_replay_after_restart_does_not_duplicate_prior_session() {
        let mut sources = sources();
        sources.runtime_sessions.push(JournalRuntimeSession {
            line: 8,
            started_at_ms: FUNDING_MS + 110,
            session_id: "4d5e6f".to_string(),
            account_id: "main".to_string(),
            strategy_name: sources.config.strategy.strategy_name.clone(),
            config_fingerprint: sources.config_fingerprint.clone(),
            account_identity_sha256: sources.account_identity_sha256.clone(),
        });
        let mut replay = sources.mark_price_observations[1].clone();
        replay.line = 9;
        sources.mark_price_observations.push(replay);

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(report.passed, "{:?}", report.issues);
        assert_eq!(report.counts.funding_bills_validated, 1);
        assert_eq!(
            report.funding_formula_samples[0].runtime_session_id,
            "1a2b3c"
        );
    }

    #[test]
    fn funding_requires_an_explicit_matching_runtime_session() {
        let mut sources = sources();
        sources.runtime_sessions.clear();

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::FundingSessionBoundaryMissing)
        );
        assert!(report.funding_formula_samples.is_empty());
    }

    #[test]
    fn funding_mark_bracket_cannot_cross_a_runtime_restart() {
        let mut sources = sources();
        sources.mark_price_observations.pop();
        sources.runtime_sessions.push(JournalRuntimeSession {
            line: 7,
            started_at_ms: FUNDING_MS + 105,
            session_id: "4d5e6f".to_string(),
            account_id: "main".to_string(),
            strategy_name: sources.config.strategy.strategy_name.clone(),
            config_fingerprint: sources.config_fingerprint.clone(),
            account_identity_sha256: sources.account_identity_sha256.clone(),
        });
        sources
            .mark_price_observations
            .push(JournalMarkPriceObservation {
                line: 8,
                event_ts_ms: FUNDING_MS + 110,
                symbol: "BTC-USDT-SWAP".to_string(),
                price: 50_000.0,
            });

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::FundingMarkBracketsMissing)
        );
        assert!(report.funding_formula_samples.is_empty());
    }
}
