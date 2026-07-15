use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use reap_core::{
    FillLiquidity, MarketEvent, NormalizedEvent, PINNED_JAVA_REVISION, Position,
    PositionMarginMode, Side,
};
use reap_storage::{
    FillRecord, RecoveredStorage, StorageError, StorageRecord, acquire_storage_lease,
    recover_jsonl_bytes_with_visitor,
};
use reap_strategy::{InstrumentConfig, InstrumentKindConfig};
use reap_venue::RemoteFill;
use reap_venue::okx::{
    OkxAccountBalanceSnapshot, OkxBalanceDetail, OkxBill, OkxBillExecutionType, OkxBillMarginMode,
    OkxInstrumentType, OkxTradeMode, parse_okx_account_balance_response_json,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::provenance::current_executable_sha256;
use crate::{
    AccountCertificationError, BillCollectionError, BillCollectionWindow, FillCollectionError,
    FillCollectionFileEvidence, LiveConfig, LiveConfigError,
    MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES, TradingEnvironment,
    verify_account_certification_artifact_path, verify_bill_collection_manifest_path,
    verify_fill_collection_manifest_path,
};

pub const ECONOMIC_RECONCILIATION_SCHEMA_VERSION: u32 = 5;
pub const MAX_ECONOMIC_JOURNAL_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_ECONOMIC_CONFIG_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_ECONOMIC_REPORTED_ISSUES: usize = 1_024;
pub const MAX_ECONOMIC_FUNDING_SAMPLES: usize = 1_024;
pub const MAX_ECONOMIC_DERIVATIVE_PNL_SAMPLES: usize = 4_096;
pub const MAX_TRADE_BILL_DELAY_MS: u64 = 10 * 60 * 1_000;
pub const MAX_FUNDING_BILL_DELAY_MS: u64 = 10 * 60 * 1_000;
pub const MAX_FUNDING_MARK_BRACKET_DISTANCE_MS: u64 = 10_000;
pub const MAX_ACCOUNT_BOUNDARY_GAP_MS: u64 = 10 * 60 * 1_000;

#[derive(Debug, Clone)]
pub struct EconomicReconciliationOptions {
    pub account_id: String,
    pub begin_ms: u64,
    pub end_ms: u64,
    pub minimum_trade_bills: u64,
    pub minimum_derivative_close_bills: u64,
    pub minimum_funding_bills: u64,
    pub maximum_trade_bill_delay_ms: u64,
    pub maximum_funding_bill_delay_ms: u64,
    pub maximum_funding_mark_bracket_distance_ms: u64,
    pub maximum_account_boundary_gap_ms: u64,
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
        if self.minimum_trade_bills == 0
            || self.minimum_derivative_close_bills == 0
            || self.minimum_funding_bills == 0
        {
            return Err(EconomicReconciliationError::InvalidOptions(
                "minimum trade, derivative-close, and funding bill counts must all be positive"
                    .to_string(),
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
        if self.maximum_account_boundary_gap_ms == 0
            || self.maximum_account_boundary_gap_ms > MAX_ACCOUNT_BOUNDARY_GAP_MS
        {
            return Err(EconomicReconciliationError::InvalidOptions(format!(
                "maximum-account-boundary-gap-ms must be in 1..={MAX_ACCOUNT_BOUNDARY_GAP_MS}"
            )));
        }
        for (name, value) in [
            ("price-abs", self.tolerances.price_abs),
            ("quantity-abs", self.tolerances.quantity_abs),
            ("fee-abs", self.tolerances.fee_abs),
            ("balance-abs", self.tolerances.balance_abs),
            ("trade-pnl-abs", self.tolerances.trade_pnl_abs),
            ("trade-pnl-relative", self.tolerances.trade_pnl_relative),
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
    pub trade_pnl_abs: f64,
    pub trade_pnl_relative: f64,
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
    AccountBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EconomicJournalRecoveryEvidence {
    pub records: u64,
    pub ignored_truncated_tail: bool,
    pub account_bootstrap_records: u64,
    pub runtime_session_records: u64,
    pub authoritative_account_snapshot_records: u64,
    pub journal_fill_records: u64,
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
    pub derivative_close_bills: u64,
    pub derivative_close_bills_recomputed: u64,
    pub eligible_fills_missing_bill: u64,
    pub funding_settlements_total: u64,
    pub funding_settlements_relevant: u64,
    pub funding_bills_matched: u64,
    pub funding_mark_brackets_validated: u64,
    pub funding_bills_validated: u64,
    pub cash_balance_currencies: u64,
    pub cash_balance_currencies_validated: u64,
    pub cash_balance_chain_links: u64,
    pub cash_balance_chain_links_validated: u64,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DerivativePnlFormulaSample {
    pub bill_id: String,
    pub symbol: String,
    pub trade_id: String,
    pub runtime_session_id: String,
    pub runtime_session_start_line: u64,
    pub snapshot_line: u64,
    pub snapshot_time_ms: u64,
    pub fill_line: u64,
    pub fill_time_ms: u64,
    pub inverse: bool,
    pub currency: String,
    pub pre_quantity: f64,
    pub pre_avg_price: f64,
    pub fill_side: Side,
    pub fill_price: f64,
    pub fill_quantity: f64,
    pub close_quantity: f64,
    pub contract_value: f64,
    pub post_quantity: f64,
    pub post_avg_price: f64,
    pub expected_sub_type: String,
    pub observed_sub_type: String,
    pub expected_pnl: f64,
    pub observed_pnl: f64,
    pub absolute_difference: f64,
    pub relative_difference: f64,
    pub effective_tolerance: f64,
    pub validated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EconomicAccountBoundaryEvidence {
    pub certification_file: FillCollectionFileEvidence,
    pub certification_schema_version: u32,
    pub collector_reap_version: String,
    pub collector_executable_sha256: String,
    pub collector_host_identity_sha256: String,
    pub start_server_ms: u64,
    pub finish_server_ms: u64,
    pub window_gap_ms: u64,
    pub total_equity_usd: f64,
    pub balance_currencies: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrencyBalanceContinuitySample {
    pub currency: String,
    pub opening_cash_balance: f64,
    pub closing_cash_balance: f64,
    pub opening_equity: f64,
    pub closing_equity: f64,
    pub opening_equity_usd: f64,
    pub closing_equity_usd: f64,
    pub bill_count: u64,
    pub first_bill_id: Option<String>,
    pub last_bill_id: Option<String>,
    pub summed_balance_change: f64,
    pub expected_closing_cash_balance: f64,
    pub aggregate_absolute_difference: f64,
    pub effective_tolerance: f64,
    pub balance_chain_links: u64,
    pub balance_chain_links_validated: u64,
    pub validated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EconomicReconciliationFailure {
    InvalidAccountBoundaries,
    InvalidOrDuplicateBalanceCurrencies,
    InvalidBillBalanceChain,
    CashBalanceContinuityMismatches,
    JournalAccountBootstrapMissingOrInvalid,
    JournalConfigFingerprintMismatch,
    JournalStrategyMismatch,
    JournalTruncatedTail,
    InvalidOrDuplicateRuntimeSessions,
    InvalidAuthoritativeAccountSnapshots,
    TradeJournalFillMismatches,
    DerivativeOpeningBasisMissingOrInvalid,
    DerivativePnlFormulaMismatches,
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
    MinimumDerivativeCloseBillsNotMet,
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
    pub minimum_derivative_close_bills: u64,
    pub minimum_funding_bills: u64,
    pub maximum_trade_bill_delay_ms: u64,
    pub maximum_funding_bill_delay_ms: u64,
    pub maximum_funding_mark_bracket_distance_ms: u64,
    pub maximum_account_boundary_gap_ms: u64,
    pub tolerances: EconomicReconciliationTolerances,
    pub config_file: FillCollectionFileEvidence,
    pub journal: FillCollectionFileEvidence,
    pub journal_recovery: EconomicJournalRecoveryEvidence,
    pub fill_collection_manifest: FillCollectionFileEvidence,
    pub bill_collection_manifest: FillCollectionFileEvidence,
    pub opening_account_boundary: EconomicAccountBoundaryEvidence,
    pub closing_account_boundary: EconomicAccountBoundaryEvidence,
    pub total_equity_change_usd: f64,
    pub currency_balance_continuity: Vec<CurrencyBalanceContinuitySample>,
    pub counts: EconomicReconciliationCounts,
    pub derivative_pnl_formula_samples: Vec<DerivativePnlFormulaSample>,
    pub derivative_pnl_formula_samples_omitted: u64,
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
    #[error("account certification failed verification: {0}")]
    AccountCertification(#[from] AccountCertificationError),
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

#[derive(Debug, Clone)]
struct JournalAuthoritativeAccountSnapshot {
    line: u64,
    event_ts_ms: u64,
    update_ts_ms: u64,
    account_id: String,
    positions: Vec<Position>,
}

#[derive(Debug, Clone)]
struct JournalFillObservation {
    line: u64,
    fill: FillRecord,
}

#[derive(Debug, Clone, Copy)]
struct PositionBasis {
    quantity: f64,
    avg_price: f64,
    snapshot_line: u64,
    snapshot_time_ms: u64,
}

#[derive(Debug, Clone)]
struct JournalDerivativePnlEvidence {
    fill_line: u64,
    runtime_session_id: String,
    runtime_session_start_line: u64,
    basis: PositionBasis,
    close_quantity: f64,
    post_quantity: f64,
    post_avg_price: f64,
    expected_sub_type: String,
    expected_pnl: f64,
}

#[derive(Debug, Clone)]
struct JournalTradeEvidence {
    observation: JournalFillObservation,
    derivative: Option<JournalDerivativePnlEvidence>,
}

#[derive(Debug)]
struct TradeBillValidation {
    valid: bool,
    derivative_sample: Option<DerivativePnlFormulaSample>,
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
    authoritative_account_snapshots: Vec<JournalAuthoritativeAccountSnapshot>,
    journal_fills: Vec<JournalFillObservation>,
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
    opening_account_boundary: BoundAccountBoundary,
    closing_account_boundary: BoundAccountBoundary,
}

#[derive(Debug, Clone)]
struct BoundAccountBoundary {
    evidence: EconomicAccountBoundaryEvidence,
    account_id: String,
    environment: TradingEnvironment,
    account_identity_sha256: String,
    config_fingerprint: String,
    config_source_path: String,
    config_sha256: String,
    passed: bool,
    balance: OkxAccountBalanceSnapshot,
}

/// Rebuilds normal-trade and funding economics from exact verified collections
/// plus a stopped runtime journal. No credentials or network access are used.
pub fn reconcile_okx_economics_paths(
    journal_path: impl AsRef<Path>,
    fill_collection_manifest_path: impl AsRef<Path>,
    bill_collection_manifest_path: impl AsRef<Path>,
    opening_account_certification_path: impl AsRef<Path>,
    closing_account_certification_path: impl AsRef<Path>,
    options: EconomicReconciliationOptions,
) -> Result<EconomicReconciliationReport, EconomicReconciliationError> {
    options.validate()?;
    let fills = verify_fill_collection_manifest_path(fill_collection_manifest_path)?;
    let bills = verify_bill_collection_manifest_path(bill_collection_manifest_path)?;
    bind_collection_manifests(&fills.manifest, &bills.manifest, &options)?;
    let mut opening_account_boundary = read_account_boundary(
        opening_account_certification_path.as_ref(),
        "opening account certification",
    )?;
    let mut closing_account_boundary = read_account_boundary(
        closing_account_certification_path.as_ref(),
        "closing account certification",
    )?;
    bind_account_boundaries(
        &mut opening_account_boundary,
        &mut closing_account_boundary,
        &bills.manifest,
        &options,
    )?;

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
    let mut authoritative_account_snapshots = Vec::new();
    let mut journal_fills = Vec::new();
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
        StorageRecord::AccountSnapshot(snapshot) => {
            authoritative_account_snapshots.push(JournalAuthoritativeAccountSnapshot {
                line,
                event_ts_ms: snapshot.ts_ms,
                update_ts_ms: snapshot.update.ts_ms,
                account_id: snapshot.account_id.clone(),
                positions: snapshot.update.positions.clone(),
            });
        }
        StorageRecord::Fill(fill) => journal_fills.push(JournalFillObservation {
            line,
            fill: fill.clone(),
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
        authoritative_account_snapshots,
        journal_fills,
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
        opening_account_boundary,
        closing_account_boundary,
    };
    let executable_sha256 =
        current_executable_sha256().map_err(EconomicReconciliationError::ExecutableHash)?;
    Ok(build_report(sources, options, executable_sha256))
}

fn read_account_boundary(
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

fn bind_account_boundaries(
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
    let currency_balance_continuity = validate_account_balance_continuity(
        &sources,
        &options,
        &mut counts,
        &mut failures,
        &mut issues,
    );
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
    let journal_trade_evidence = build_journal_trade_evidence(
        &sources,
        &valid_runtime_sessions,
        &options,
        &mut failures,
        &mut issues,
    );
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
    let mut derivative_pnl_samples = Vec::new();
    let mut derivative_pnl_samples_omitted = 0_u64;
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
                matched_fill_keys.insert(key.clone());
                if instrument(&sources.config, &bill.symbol).is_some_and(|instrument| {
                    instrument.kind.is_derivative() && matches!(bill.sub_type.as_str(), "5" | "6")
                }) {
                    counts.derivative_close_bills += 1;
                }
                let validation = validate_trade_bill(
                    bill,
                    fill,
                    journal_trade_evidence
                        .get(&key)
                        .map(Vec::as_slice)
                        .unwrap_or_default(),
                    &sources.config,
                    &sources.account_id,
                    &options,
                    &mut failures,
                    &mut issues,
                );
                if validation.valid {
                    counts.trade_bills_validated += 1;
                }
                if let Some(sample) = validation.derivative_sample {
                    if validation.valid
                        && sample.close_quantity > options.tolerances.quantity_abs
                        && sample.validated
                    {
                        counts.derivative_close_bills_recomputed += 1;
                    }
                    if derivative_pnl_samples.len() < MAX_ECONOMIC_DERIVATIVE_PNL_SAMPLES {
                        derivative_pnl_samples.push(sample);
                    } else {
                        derivative_pnl_samples_omitted =
                            derivative_pnl_samples_omitted.saturating_add(1);
                    }
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
    if counts.derivative_close_bills_recomputed < options.minimum_derivative_close_bills {
        failures.insert(EconomicReconciliationFailure::MinimumDerivativeCloseBillsNotMet);
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
        minimum_derivative_close_bills: options.minimum_derivative_close_bills,
        minimum_funding_bills: options.minimum_funding_bills,
        maximum_trade_bill_delay_ms: options.maximum_trade_bill_delay_ms,
        maximum_funding_bill_delay_ms: options.maximum_funding_bill_delay_ms,
        maximum_funding_mark_bracket_distance_ms: options
            .maximum_funding_mark_bracket_distance_ms,
        maximum_account_boundary_gap_ms: options.maximum_account_boundary_gap_ms,
        tolerances: options.tolerances,
        config_file: sources.config_file,
        journal: sources.journal,
        journal_recovery: EconomicJournalRecoveryEvidence {
            records: sources.recovered.records,
            ignored_truncated_tail: sources.recovered.ignored_truncated_tail,
            account_bootstrap_records: sources.account_bootstrap_records,
            runtime_session_records: sources.runtime_sessions.len() as u64,
            authoritative_account_snapshot_records: sources
                .authoritative_account_snapshots
                .len() as u64,
            journal_fill_records: sources.journal_fills.len() as u64,
            funding_settlement_records: sources.settlements.len() as u64,
            position_observation_records: sources.position_observations.len() as u64,
            mark_price_observation_records: sources.mark_price_observations.len() as u64,
            exclusive_lease_held_while_reading: true,
        },
        fill_collection_manifest: sources.fill_manifest_file,
        bill_collection_manifest: sources.bill_manifest_file,
        total_equity_change_usd: sources
            .closing_account_boundary
            .evidence
            .total_equity_usd
            - sources
                .opening_account_boundary
                .evidence
                .total_equity_usd,
        opening_account_boundary: sources.opening_account_boundary.evidence,
        closing_account_boundary: sources.closing_account_boundary.evidence,
        currency_balance_continuity,
        counts,
        derivative_pnl_formula_samples: derivative_pnl_samples,
        derivative_pnl_formula_samples_omitted: derivative_pnl_samples_omitted,
        funding_formula_samples: funding_samples,
        funding_formula_samples_omitted: funding_samples_omitted,
        issues: issues.issues,
        issues_truncated,
        limitations: vec![
            "derivative close PnL is reconstructed from same-session authoritative REST avgPx snapshots and every intervening critical journal fill; the snapshot exchange timestamp must strictly precede every replayed fill".to_string(),
            "expiry-futures avgPx can reset at settlement; controlled evidence windows containing unsupported settlement bills fail, but dedicated settlement-PnL reconstruction remains out of scope".to_string(),
            "funding checks the bill-reported mark against journaled observations bracketing the exchange-reported assessment time; the exact internal venue assessment tick is not reproduced".to_string(),
            "runtime-session boundaries are locally journaled provenance that prevents cross-restart evidence composition; they are not remote process attestation".to_string(),
            "settlements with no funding bill are not failures because a zero position legitimately produces no balance change; minimum matched funding evidence is required instead".to_string(),
            "the final trade-delay guard is excluded from fill-to-bill completeness because its bills may fall after the closed account-bill window".to_string(),
            "opening and closing account snapshots are sequential authenticated/public REST certifications rather than atomic venue valuation ticks".to_string(),
            "a currency absent from an unfiltered OKX balance response is treated as zero at that boundary; every intervening balance-changing bill must still be present".to_string(),
            "total-equity delta is reported but is not equated to cash bill changes because mark-to-market unrealized PnL can change between boundaries".to_string(),
        ],
        failures,
        passed,
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct BoundaryCurrencyValue {
    cash_balance: f64,
    equity: f64,
    equity_usd: f64,
}

fn validate_account_balance_continuity(
    sources: &BoundEconomicSources,
    options: &EconomicReconciliationOptions,
    counts: &mut EconomicReconciliationCounts,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> Vec<CurrencyBalanceContinuitySample> {
    validate_bound_account_identity(
        "opening",
        &sources.opening_account_boundary,
        sources,
        options,
        failures,
        issues,
    );
    validate_bound_account_identity(
        "closing",
        &sources.closing_account_boundary,
        sources,
        options,
        failures,
        issues,
    );
    let opening = boundary_currency_values(
        "opening",
        &sources.opening_account_boundary.balance,
        failures,
        issues,
    );
    let closing = boundary_currency_values(
        "closing",
        &sources.closing_account_boundary.balance,
        failures,
        issues,
    );
    let mut bills_by_currency = BTreeMap::<String, Vec<(&OkxBill, u128)>>::new();
    let mut seen_bill_ids = BTreeSet::new();
    for bill in &sources.bills {
        if !valid_currency(&bill.currency) {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateBalanceCurrencies,
                issue_for_bill(
                    EconomicIssueSource::BillCollection,
                    bill,
                    "ccy",
                    "1-32 uppercase ASCII letters or digits",
                    &bill.currency,
                    "bill currency cannot be joined to certified account balances",
                ),
                failures,
            );
            continue;
        }
        let numeric_id = match parse_numeric_bill_id(&bill.bill_id) {
            Some(value) => value,
            None => {
                issues.push(
                    EconomicReconciliationFailure::InvalidBillBalanceChain,
                    issue_for_bill(
                        EconomicIssueSource::BillCollection,
                        bill,
                        "billId",
                        "positive base-10 integer",
                        &bill.bill_id,
                        "bill balance ordering requires a numeric OKX bill id",
                    ),
                    failures,
                );
                0
            }
        };
        if !seen_bill_ids.insert(bill.bill_id.clone()) {
            issues.push(
                EconomicReconciliationFailure::InvalidBillBalanceChain,
                issue_for_bill(
                    EconomicIssueSource::BillCollection,
                    bill,
                    "billId",
                    "globally unique bill id",
                    &bill.bill_id,
                    "bill balance chain contains a duplicate bill id",
                ),
                failures,
            );
        }
        bills_by_currency
            .entry(bill.currency.clone())
            .or_default()
            .push((bill, numeric_id));
    }
    for bills in bills_by_currency.values_mut() {
        bills.sort_by(|(left, left_id), (right, right_id)| {
            (left.timestamp_ms, *left_id, left.bill_id.as_str()).cmp(&(
                right.timestamp_ms,
                *right_id,
                right.bill_id.as_str(),
            ))
        });
    }

    let currencies = opening
        .keys()
        .chain(closing.keys())
        .chain(bills_by_currency.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut samples = Vec::new();
    for currency in currencies {
        let opening_value = opening.get(&currency).copied().unwrap_or_default();
        let closing_value = closing.get(&currency).copied().unwrap_or_default();
        let bills = bills_by_currency
            .get(&currency)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut summed_balance_change = 0.0;
        let links = (bills.len() as u64).saturating_add(1);
        let mut links_validated = 0_u64;
        let mut valid = true;
        let mut previous_post_balance = None::<f64>;

        if bills.is_empty() {
            if close_abs(
                opening_value.cash_balance,
                closing_value.cash_balance,
                options.tolerances.balance_abs,
            ) {
                links_validated = 1;
            } else {
                valid = false;
                push_cash_continuity_issue(
                    &currency,
                    "boundary_cash_balance",
                    opening_value.cash_balance,
                    closing_value.cash_balance,
                    "currency cash balance changed without an account bill",
                    failures,
                    issues,
                );
            }
        } else {
            for (offset, (bill, numeric_id)) in bills.iter().enumerate() {
                if *numeric_id == 0 {
                    valid = false;
                }
                if !bill.balance_change.is_finite() {
                    valid = false;
                    issues.push(
                        EconomicReconciliationFailure::InvalidBillBalanceChain,
                        issue_for_bill(
                            EconomicIssueSource::BillCollection,
                            bill,
                            "balChg",
                            "finite balance change",
                            &bill.balance_change.to_string(),
                            "bill balance change is not finite",
                        ),
                        failures,
                    );
                    continue;
                }
                summed_balance_change += bill.balance_change;
                let Some(post_balance) = bill.balance.filter(|value| value.is_finite()) else {
                    valid = false;
                    issues.push(
                        EconomicReconciliationFailure::InvalidBillBalanceChain,
                        issue_for_bill(
                            EconomicIssueSource::BillCollection,
                            bill,
                            "bal",
                            "finite post-bill cash balance",
                            "missing or non-finite",
                            "bill does not expose the post-change balance needed for continuity",
                        ),
                        failures,
                    );
                    previous_post_balance = None;
                    continue;
                };
                let pre_balance = post_balance - bill.balance_change;
                let (expected, field, message, failure) = if offset == 0 {
                    (
                        opening_value.cash_balance,
                        "opening_cash_balance",
                        "first bill pre-balance does not match the opening account snapshot",
                        EconomicReconciliationFailure::CashBalanceContinuityMismatches,
                    )
                } else if let Some(previous) = previous_post_balance {
                    (
                        previous,
                        "bill_balance_chain",
                        "adjacent bill post/pre balances are discontinuous",
                        EconomicReconciliationFailure::InvalidBillBalanceChain,
                    )
                } else {
                    valid = false;
                    previous_post_balance = Some(post_balance);
                    continue;
                };
                if close_abs(expected, pre_balance, options.tolerances.balance_abs) {
                    links_validated = links_validated.saturating_add(1);
                } else {
                    valid = false;
                    issues.push(
                        failure,
                        issue_for_bill(
                            EconomicIssueSource::BillCollection,
                            bill,
                            field,
                            &expected.to_string(),
                            &pre_balance.to_string(),
                            message,
                        ),
                        failures,
                    );
                }
                previous_post_balance = Some(post_balance);
            }
            if let Some(last_post_balance) = previous_post_balance {
                if close_abs(
                    last_post_balance,
                    closing_value.cash_balance,
                    options.tolerances.balance_abs,
                ) {
                    links_validated = links_validated.saturating_add(1);
                } else {
                    valid = false;
                    push_cash_continuity_issue(
                        &currency,
                        "closing_cash_balance",
                        last_post_balance,
                        closing_value.cash_balance,
                        "last bill post-balance does not match the closing account snapshot",
                        failures,
                        issues,
                    );
                }
            } else {
                valid = false;
            }
        }

        let expected_closing_cash_balance = opening_value.cash_balance + summed_balance_change;
        let aggregate_absolute_difference =
            (expected_closing_cash_balance - closing_value.cash_balance).abs();
        if !expected_closing_cash_balance.is_finite()
            || aggregate_absolute_difference > options.tolerances.balance_abs
        {
            valid = false;
            push_cash_continuity_issue(
                &currency,
                "aggregate_cash_balance",
                expected_closing_cash_balance,
                closing_value.cash_balance,
                "opening cash plus all bill balance changes does not equal closing cash",
                failures,
                issues,
            );
        }
        if links_validated != links {
            valid = false;
        }
        counts.cash_balance_chain_links = counts.cash_balance_chain_links.saturating_add(links);
        counts.cash_balance_chain_links_validated = counts
            .cash_balance_chain_links_validated
            .saturating_add(links_validated);
        counts.cash_balance_currencies = counts.cash_balance_currencies.saturating_add(1);
        if valid {
            counts.cash_balance_currencies_validated =
                counts.cash_balance_currencies_validated.saturating_add(1);
        }
        samples.push(CurrencyBalanceContinuitySample {
            currency,
            opening_cash_balance: opening_value.cash_balance,
            closing_cash_balance: closing_value.cash_balance,
            opening_equity: opening_value.equity,
            closing_equity: closing_value.equity,
            opening_equity_usd: opening_value.equity_usd,
            closing_equity_usd: closing_value.equity_usd,
            bill_count: bills.len() as u64,
            first_bill_id: bills.first().map(|(bill, _)| bill.bill_id.clone()),
            last_bill_id: bills.last().map(|(bill, _)| bill.bill_id.clone()),
            summed_balance_change,
            expected_closing_cash_balance,
            aggregate_absolute_difference,
            effective_tolerance: options.tolerances.balance_abs,
            balance_chain_links: links,
            balance_chain_links_validated: links_validated,
            validated: valid,
        });
    }
    samples
}

fn validate_bound_account_identity(
    label: &str,
    boundary: &BoundAccountBoundary,
    sources: &BoundEconomicSources,
    options: &EconomicReconciliationOptions,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) {
    let identity_valid = boundary.passed
        && boundary.account_id == sources.account_id
        && boundary.environment == sources.environment
        && boundary.account_identity_sha256 == sources.account_identity_sha256
        && boundary.config_fingerprint == sources.config_fingerprint
        && boundary.config_source_path == sources.config_file.path
        && boundary.config_sha256 == sources.config_file.sha256
        && boundary.evidence.start_server_ms <= boundary.evidence.finish_server_ms
        && boundary.evidence.balance_currencies == boundary.balance.details.len() as u64
        && boundary.evidence.total_equity_usd.is_finite()
        && boundary.balance.total_equity_usd.is_some_and(|value| {
            close_abs(
                value,
                boundary.evidence.total_equity_usd,
                options.tolerances.balance_abs,
            )
        });
    let expected_gap = if label == "opening" {
        options
            .begin_ms
            .checked_sub(boundary.evidence.finish_server_ms)
    } else {
        boundary
            .evidence
            .start_server_ms
            .checked_sub(options.end_ms)
    };
    let timing_valid = expected_gap.is_some_and(|gap| {
        gap == boundary.evidence.window_gap_ms && gap <= options.maximum_account_boundary_gap_ms
    });
    if !identity_valid || !timing_valid {
        issues.push(
            EconomicReconciliationFailure::InvalidAccountBoundaries,
            issue(
                EconomicIssueSource::AccountBoundary,
                None,
                None,
                None,
                &format!("{label}_account_boundary"),
                "passing, bound certification on the correct side of the window within the configured gap",
                &format!(
                    "account={}, environment={:?}, passed={}, start={}, finish={}, gap={}",
                    boundary.account_id,
                    boundary.environment,
                    boundary.passed,
                    boundary.evidence.start_server_ms,
                    boundary.evidence.finish_server_ms,
                    boundary.evidence.window_gap_ms
                ),
                "account boundary identity, timing, or certified total equity is invalid",
            ),
            failures,
        );
    }
}

fn boundary_currency_values(
    label: &str,
    balance: &OkxAccountBalanceSnapshot,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> BTreeMap<String, BoundaryCurrencyValue> {
    let mut values = BTreeMap::new();
    if balance.details.is_empty() {
        issues.push(
            EconomicReconciliationFailure::InvalidOrDuplicateBalanceCurrencies,
            issue(
                EconomicIssueSource::AccountBoundary,
                None,
                None,
                None,
                &format!("{label}_balance_details"),
                "at least one certified currency",
                "empty",
                "account boundary has no currency balances",
            ),
            failures,
        );
    }
    for detail in &balance.details {
        if !valid_currency(&detail.currency) {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateBalanceCurrencies,
                boundary_currency_issue(
                    label,
                    detail,
                    "ccy",
                    "1-32 uppercase ASCII letters or digits",
                    &detail.currency,
                    "account boundary contains an invalid currency",
                ),
                failures,
            );
            continue;
        }
        let Some(cash_balance) = finite_optional(detail.cash_balance) else {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateBalanceCurrencies,
                boundary_currency_issue(
                    label,
                    detail,
                    "cashBal",
                    "finite value",
                    "missing or non-finite",
                    "account boundary cash balance is unavailable",
                ),
                failures,
            );
            continue;
        };
        let Some(equity) = finite_optional(detail.equity) else {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateBalanceCurrencies,
                boundary_currency_issue(
                    label,
                    detail,
                    "eq",
                    "finite value",
                    "missing or non-finite",
                    "account boundary native equity is unavailable",
                ),
                failures,
            );
            continue;
        };
        let Some(equity_usd) = finite_optional(detail.equity_usd) else {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateBalanceCurrencies,
                boundary_currency_issue(
                    label,
                    detail,
                    "eqUsd",
                    "finite value",
                    "missing or non-finite",
                    "account boundary converted equity is unavailable",
                ),
                failures,
            );
            continue;
        };
        if values
            .insert(
                detail.currency.clone(),
                BoundaryCurrencyValue {
                    cash_balance,
                    equity,
                    equity_usd,
                },
            )
            .is_some()
        {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateBalanceCurrencies,
                boundary_currency_issue(
                    label,
                    detail,
                    "ccy",
                    "unique currency",
                    &detail.currency,
                    "account boundary contains duplicate currency balances",
                ),
                failures,
            );
        }
    }
    values
}

fn boundary_currency_issue(
    label: &str,
    detail: &OkxBalanceDetail,
    field: &str,
    expected: &str,
    observed: &str,
    message: &str,
) -> EconomicIssue {
    issue(
        EconomicIssueSource::AccountBoundary,
        None,
        None,
        None,
        &format!("{label}.{}.{}", detail.currency, field),
        expected,
        observed,
        message,
    )
}

fn push_cash_continuity_issue(
    currency: &str,
    field: &str,
    expected: f64,
    observed: f64,
    message: &str,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) {
    issues.push(
        EconomicReconciliationFailure::CashBalanceContinuityMismatches,
        issue(
            EconomicIssueSource::AccountBoundary,
            None,
            None,
            None,
            &format!("{currency}.{field}"),
            &expected.to_string(),
            &observed.to_string(),
            message,
        ),
        failures,
    );
}

fn valid_currency(currency: &str) -> bool {
    !currency.is_empty()
        && currency.len() <= 32
        && currency
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

fn parse_numeric_bill_id(value: &str) -> Option<u128> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse::<u128>().ok().filter(|value| *value > 0)
}

fn finite_optional(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite())
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

fn build_journal_trade_evidence(
    sources: &BoundEconomicSources,
    runtime_sessions: &[&JournalRuntimeSession],
    options: &EconomicReconciliationOptions,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> BTreeMap<(String, String), Vec<JournalTradeEvidence>> {
    enum TimelineEvent<'a> {
        Snapshot(&'a JournalAuthoritativeAccountSnapshot),
        Fill(&'a JournalFillObservation),
    }

    impl TimelineEvent<'_> {
        fn line(&self) -> u64 {
            match self {
                Self::Snapshot(snapshot) => snapshot.line,
                Self::Fill(fill) => fill.line,
            }
        }
    }

    let mut timeline = sources
        .authoritative_account_snapshots
        .iter()
        .map(TimelineEvent::Snapshot)
        .chain(sources.journal_fills.iter().map(TimelineEvent::Fill))
        .collect::<Vec<_>>();
    timeline.sort_by_key(TimelineEvent::line);
    let mut verified_fills = BTreeMap::<(String, String), Vec<&RemoteFill>>::new();
    for fill in &sources.fills {
        verified_fills
            .entry((fill.symbol.clone(), fill.fill_id.clone()))
            .or_default()
            .push(fill);
    }

    let mut state_session_id = None::<String>;
    let mut positions = BTreeMap::<String, PositionBasis>::new();
    let mut by_key = BTreeMap::<(String, String), Vec<JournalTradeEvidence>>::new();
    let mut seen_fill_keys = BTreeSet::new();

    for event in timeline {
        match event {
            TimelineEvent::Snapshot(snapshot) => {
                if snapshot.account_id != sources.account_id {
                    continue;
                }
                let Some(session) =
                    runtime_session_for_line(runtime_sessions, &sources.account_id, snapshot.line)
                else {
                    push_invalid_snapshot_issue(
                        snapshot,
                        "same-account runtime session before the snapshot",
                        "missing",
                        failures,
                        issues,
                    );
                    state_session_id = None;
                    positions.clear();
                    continue;
                };
                if session.account_identity_sha256 != sources.account_identity_sha256 {
                    push_invalid_snapshot_issue(
                        snapshot,
                        "runtime session with the exact collected account identity",
                        &session.account_identity_sha256,
                        failures,
                        issues,
                    );
                    state_session_id = None;
                    positions.clear();
                    continue;
                }
                if snapshot.event_ts_ms == 0 || snapshot.event_ts_ms != snapshot.update_ts_ms {
                    push_invalid_snapshot_issue(
                        snapshot,
                        "matching positive record/update timestamps",
                        &format!(
                            "record={}, update={}",
                            snapshot.event_ts_ms, snapshot.update_ts_ms
                        ),
                        failures,
                        issues,
                    );
                    state_session_id = None;
                    positions.clear();
                    continue;
                }

                let mut snapshot_positions = BTreeMap::new();
                let mut invalid = None;
                for position in &snapshot.positions {
                    let configured = instrument(&sources.config, &position.symbol);
                    let owned = sources
                        .config
                        .account_for_symbol(&position.symbol)
                        .is_some_and(|account| account.id == sources.account_id);
                    let margin_mode_valid = if position.qty == 0.0 {
                        true
                    } else {
                        matches!(
                            (
                                expected_bill_margin_mode(
                                    &sources.config,
                                    &sources.account_id,
                                    &position.symbol,
                                ),
                                position.margin_mode,
                            ),
                            (
                                Some(OkxBillMarginMode::Cross),
                                Some(PositionMarginMode::Cross)
                            ) | (
                                Some(OkxBillMarginMode::Isolated),
                                Some(PositionMarginMode::Isolated)
                            )
                        )
                    };
                    if position.symbol.is_empty()
                        || !position.qty.is_finite()
                        || !position.avg_price.is_finite()
                        || (position.qty != 0.0
                            && (!owned
                                || !margin_mode_valid
                                || !configured.is_some_and(|instrument| {
                                    instrument.kind.is_derivative() && position.avg_price > 0.0
                                })))
                    {
                        invalid = Some(format!(
                            "invalid position {} qty={} avgPx={}",
                            position.symbol, position.qty, position.avg_price
                        ));
                        break;
                    }
                    if snapshot_positions
                        .insert(position.symbol.clone(), position)
                        .is_some()
                    {
                        invalid = Some(format!("duplicate position {}", position.symbol));
                        break;
                    }
                }
                if let Some(invalid) = invalid {
                    push_invalid_snapshot_issue(
                        snapshot,
                        "unique finite configured derivative positions with positive avgPx when non-zero",
                        &invalid,
                        failures,
                        issues,
                    );
                    state_session_id = None;
                    positions.clear();
                    continue;
                }

                positions.clear();
                for configured in sources.config.instruments_for_account(&sources.account_id) {
                    if !configured.kind.is_derivative() {
                        continue;
                    }
                    let (quantity, avg_price) =
                        snapshot_positions
                            .get(&configured.symbol)
                            .map_or((0.0, 0.0), |position| {
                                if position.qty == 0.0 {
                                    (0.0, 0.0)
                                } else {
                                    (position.qty, position.avg_price)
                                }
                            });
                    positions.insert(
                        configured.symbol.clone(),
                        PositionBasis {
                            quantity,
                            avg_price,
                            snapshot_line: snapshot.line,
                            snapshot_time_ms: snapshot.event_ts_ms,
                        },
                    );
                }
                state_session_id = Some(session.session_id.clone());
            }
            TimelineEvent::Fill(observation) => {
                if observation.fill.account_id.as_deref() != Some(sources.account_id.as_str()) {
                    continue;
                }
                let fill_key = (
                    observation.fill.symbol.clone(),
                    observation.fill.fill_id.clone(),
                );
                if observation.fill.fill_id.is_empty() || !seen_fill_keys.insert(fill_key.clone()) {
                    issues.push(
                        EconomicReconciliationFailure::TradeJournalFillMismatches,
                        issue(
                            EconomicIssueSource::Journal,
                            None,
                            Some(&observation.fill.symbol),
                            (!observation.fill.fill_id.is_empty())
                                .then_some(observation.fill.fill_id.as_str()),
                            "journal_fill_identity",
                            "unique non-empty (symbol, fill_id) for the account",
                            &format!("duplicate or empty at line {}", observation.line),
                            "critical fill journal contains an ambiguous trade identity",
                        ),
                        failures,
                    );
                    positions.remove(&observation.fill.symbol);
                    by_key
                        .entry(fill_key)
                        .or_default()
                        .push(JournalTradeEvidence {
                            observation: observation.clone(),
                            derivative: None,
                        });
                    continue;
                }
                let session = runtime_session_for_line(
                    runtime_sessions,
                    &sources.account_id,
                    observation.line,
                );
                if state_session_id.as_deref() != session.map(|session| session.session_id.as_str())
                {
                    state_session_id = None;
                    positions.clear();
                }
                let derivative_instrument = instrument(&sources.config, &observation.fill.symbol)
                    .filter(|instrument| instrument.kind.is_derivative());
                let derivative = derivative_instrument.and_then(|instrument| {
                    let session = session?;
                    let basis = *positions.get(&observation.fill.symbol)?;
                    let [exchange_fill] = verified_fills.get(&fill_key)?.as_slice() else {
                        return None;
                    };
                    if basis.snapshot_time_ms >= observation.fill.ts_ms
                        || exchange_fill.ts_ms != observation.fill.ts_ms
                        || exchange_fill.side != observation.fill.side
                        || !close_abs(
                            exchange_fill.price,
                            observation.fill.price,
                            options.tolerances.price_abs,
                        )
                        || !close_abs(
                            exchange_fill.qty,
                            observation.fill.qty,
                            options.tolerances.quantity_abs,
                        )
                    {
                        return None;
                    }
                    let calculation = apply_derivative_fill(
                        basis,
                        &observation.fill,
                        instrument,
                        options.tolerances.quantity_abs,
                    )?;
                    positions.insert(
                        observation.fill.symbol.clone(),
                        PositionBasis {
                            quantity: calculation.post_quantity,
                            avg_price: calculation.post_avg_price,
                            ..basis
                        },
                    );
                    Some(JournalDerivativePnlEvidence {
                        fill_line: observation.line,
                        runtime_session_id: session.session_id.clone(),
                        runtime_session_start_line: session.line,
                        basis,
                        close_quantity: calculation.close_quantity,
                        post_quantity: calculation.post_quantity,
                        post_avg_price: calculation.post_avg_price,
                        expected_sub_type: calculation.expected_sub_type,
                        expected_pnl: calculation.expected_pnl,
                    })
                });
                if derivative_instrument.is_some() && derivative.is_none() {
                    positions.remove(&observation.fill.symbol);
                }
                by_key
                    .entry(fill_key)
                    .or_default()
                    .push(JournalTradeEvidence {
                        observation: observation.clone(),
                        derivative,
                    });
            }
        }
    }
    by_key
}

struct DerivativeFillCalculation {
    close_quantity: f64,
    post_quantity: f64,
    post_avg_price: f64,
    expected_sub_type: String,
    expected_pnl: f64,
}

fn apply_derivative_fill(
    basis: PositionBasis,
    fill: &FillRecord,
    instrument: &InstrumentConfig,
    quantity_tolerance: f64,
) -> Option<DerivativeFillCalculation> {
    if !basis.quantity.is_finite()
        || !basis.avg_price.is_finite()
        || !fill.price.is_finite()
        || fill.price <= 0.0
        || !fill.qty.is_finite()
        || fill.qty <= 0.0
        || !instrument.contract_value.is_finite()
        || instrument.contract_value <= 0.0
    {
        return None;
    }
    let pre_quantity = if basis.quantity.abs() <= quantity_tolerance {
        0.0
    } else {
        basis.quantity
    };
    if pre_quantity != 0.0 && basis.avg_price <= 0.0 {
        return None;
    }
    let delta = match fill.side {
        Side::Buy => fill.qty,
        Side::Sell => -fill.qty,
    };
    let closes_position = pre_quantity != 0.0 && pre_quantity.signum() != delta.signum();
    let close_quantity = if closes_position {
        pre_quantity.abs().min(fill.qty)
    } else {
        0.0
    };
    let expected_sub_type = match (closes_position, pre_quantity.is_sign_positive(), fill.side) {
        (true, true, Side::Sell) => "5",
        (true, false, Side::Buy) => "6",
        (false, _, Side::Buy) => "3",
        (false, _, Side::Sell) => "4",
        _ => return None,
    }
    .to_string();
    let expected_pnl = if close_quantity == 0.0 {
        0.0
    } else if instrument.kind.is_inverse() {
        pre_quantity.signum()
            * (1.0 / basis.avg_price - 1.0 / fill.price)
            * close_quantity
            * instrument.contract_value
    } else {
        pre_quantity.signum()
            * (fill.price - basis.avg_price)
            * close_quantity
            * instrument.contract_value
    };

    let raw_post_quantity = pre_quantity + delta;
    let post_quantity = if raw_post_quantity.abs() <= quantity_tolerance {
        0.0
    } else {
        raw_post_quantity
    };
    let post_avg_price = if post_quantity == 0.0 {
        0.0
    } else if pre_quantity == 0.0 || pre_quantity.signum() != post_quantity.signum() {
        fill.price
    } else if post_quantity.abs() < pre_quantity.abs() {
        basis.avg_price
    } else if instrument.kind.is_inverse() {
        let base_value = pre_quantity.abs() / basis.avg_price + fill.qty / fill.price;
        if base_value <= 0.0 || !base_value.is_finite() {
            return None;
        }
        post_quantity.abs() / base_value
    } else {
        (basis.avg_price * pre_quantity.abs() + fill.price * fill.qty) / post_quantity.abs()
    };
    if !expected_pnl.is_finite() || !post_avg_price.is_finite() || post_avg_price < 0.0 {
        return None;
    }
    Some(DerivativeFillCalculation {
        close_quantity,
        post_quantity,
        post_avg_price,
        expected_sub_type,
        expected_pnl,
    })
}

fn push_invalid_snapshot_issue(
    snapshot: &JournalAuthoritativeAccountSnapshot,
    expected: &str,
    observed: &str,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) {
    issues.push(
        EconomicReconciliationFailure::InvalidAuthoritativeAccountSnapshots,
        issue(
            EconomicIssueSource::Journal,
            None,
            None,
            None,
            "authoritative_account_snapshot",
            expected,
            &format!("line {}: {observed}", snapshot.line),
            "journaled REST account snapshot cannot establish an opening-cost basis",
        ),
        failures,
    );
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
    journal_candidates: &[JournalTradeEvidence],
    config: &LiveConfig,
    account_id: &str,
    options: &EconomicReconciliationOptions,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> TradeBillValidation {
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
        return TradeBillValidation {
            valid: false,
            derivative_sample: None,
        };
    };
    let journal_trade =
        validate_journal_trade_fill(bill, fill, journal_candidates, options, failures, issues);
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
    let derivative_sample = if instrument.kind.is_derivative() {
        validate_derivative_trade_pnl(
            bill,
            fill,
            journal_trade,
            instrument,
            options,
            failures,
            issues,
        )
    } else {
        None
    };
    TradeBillValidation {
        valid: before == issues.total,
        derivative_sample,
    }
}

fn validate_journal_trade_fill<'a>(
    bill: &OkxBill,
    fill: &RemoteFill,
    candidates: &'a [JournalTradeEvidence],
    options: &EconomicReconciliationOptions,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> Option<&'a JournalTradeEvidence> {
    let [candidate] = candidates else {
        issues.push(
            EconomicReconciliationFailure::TradeJournalFillMismatches,
            issue_for_bill(
                EconomicIssueSource::Journal,
                bill,
                "journal_fill",
                "exactly one same-account critical fill record",
                &format!("{} candidates", candidates.len()),
                "verified exchange fill cannot be bound to one durable runtime fill",
            ),
            failures,
        );
        return None;
    };
    let journal = &candidate.observation.fill;
    let mut valid = true;
    let mut compare = |field: &str, matches: bool, expected: String, observed: String| {
        if !matches {
            valid = false;
            issues.push(
                EconomicReconciliationFailure::TradeJournalFillMismatches,
                issue_for_bill(
                    EconomicIssueSource::Journal,
                    bill,
                    field,
                    &expected,
                    &observed,
                    "critical journal fill does not match the independently collected exchange fill",
                ),
                failures,
            );
        }
    };
    compare(
        "journal_fill_time",
        journal.ts_ms == fill.ts_ms,
        fill.ts_ms.to_string(),
        journal.ts_ms.to_string(),
    );
    compare(
        "journal_order_id",
        journal.order_id == fill.exchange_order_id
            || (!fill.client_order_id.is_empty() && journal.order_id == fill.client_order_id),
        format!("{} or {}", fill.exchange_order_id, fill.client_order_id),
        journal.order_id.clone(),
    );
    compare(
        "journal_side",
        journal.side == fill.side,
        side_name(fill.side).to_string(),
        side_name(journal.side).to_string(),
    );
    compare(
        "journal_price",
        close_abs(journal.price, fill.price, options.tolerances.price_abs),
        fill.price.to_string(),
        journal.price.to_string(),
    );
    compare(
        "journal_quantity",
        close_abs(journal.qty, fill.qty, options.tolerances.quantity_abs),
        fill.qty.to_string(),
        journal.qty.to_string(),
    );
    compare(
        "journal_liquidity",
        journal.liquidity == Some(fill.liquidity),
        execution_name(match fill.liquidity {
            FillLiquidity::Maker => OkxBillExecutionType::Maker,
            FillLiquidity::Taker => OkxBillExecutionType::Taker,
        })
        .to_string(),
        journal
            .liquidity
            .map(|liquidity| match liquidity {
                FillLiquidity::Maker => "maker",
                FillLiquidity::Taker => "taker",
            })
            .unwrap_or("missing")
            .to_string(),
    );
    match (&fill.fee, &journal.fee) {
        (Some(expected), Some(observed)) => {
            compare(
                "journal_fee_currency",
                expected
                    .currency
                    .trim()
                    .eq_ignore_ascii_case(observed.currency.trim()),
                expected.currency.trim().to_ascii_uppercase(),
                observed.currency.trim().to_ascii_uppercase(),
            );
            compare(
                "journal_fee",
                close_abs(expected.amount, observed.amount, options.tolerances.fee_abs),
                expected.amount.to_string(),
                observed.amount.to_string(),
            );
        }
        _ => compare(
            "journal_fee",
            false,
            "exact signed fee on collection and journal".to_string(),
            "missing".to_string(),
        ),
    }
    valid.then_some(candidate)
}

#[allow(clippy::too_many_arguments)]
fn validate_derivative_trade_pnl(
    bill: &OkxBill,
    fill: &RemoteFill,
    journal_trade: Option<&JournalTradeEvidence>,
    instrument: &InstrumentConfig,
    options: &EconomicReconciliationOptions,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> Option<DerivativePnlFormulaSample> {
    let journal_trade = journal_trade?;
    let Some(evidence) = journal_trade.derivative.as_ref() else {
        issues.push(
            EconomicReconciliationFailure::DerivativeOpeningBasisMissingOrInvalid,
            issue_for_bill(
                EconomicIssueSource::Journal,
                bill,
                "opening_basis",
                "same-session authoritative REST avgPx snapshot before the critical fill",
                "missing or invalid",
                "derivative PnL cannot be independently reconstructed from the stopped journal",
            ),
            failures,
        );
        return None;
    };
    if evidence.basis.snapshot_line <= evidence.runtime_session_start_line
        || evidence.basis.snapshot_line >= evidence.fill_line
        || evidence.basis.snapshot_time_ms >= fill.ts_ms
    {
        issues.push(
            EconomicReconciliationFailure::DerivativeOpeningBasisMissingOrInvalid,
            issue_for_bill(
                EconomicIssueSource::Journal,
                bill,
                "opening_basis_order",
                "same-session snapshot line/time before fill line/time",
                &format!(
                    "session_line={}, snapshot_line={}, snapshot_ts={}, fill_line={}, fill_ts={}",
                    evidence.runtime_session_start_line,
                    evidence.basis.snapshot_line,
                    evidence.basis.snapshot_time_ms,
                    evidence.fill_line,
                    fill.ts_ms
                ),
                "authoritative position basis is not causally ordered before the target fill",
            ),
            failures,
        );
        return None;
    }
    let Some(observed_pnl) = bill.pnl.filter(|pnl| pnl.is_finite()) else {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "pnl",
            "finite derivative trade PnL",
            &format!("{:?}", bill.pnl),
            "derivative trade bill does not expose a finite realized-PnL value",
        );
        return None;
    };
    let subtype_valid = bill.sub_type == evidence.expected_sub_type;
    if !subtype_valid {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::DerivativePnlFormulaMismatches,
            bill,
            "subType",
            &evidence.expected_sub_type,
            &bill.sub_type,
            "bill open/close direction contradicts the journal-reconstructed pre-fill position",
        );
    }
    let absolute_difference = (evidence.expected_pnl - observed_pnl).abs();
    let scale = evidence.expected_pnl.abs().max(observed_pnl.abs());
    let relative_difference = absolute_difference / scale.max(f64::MIN_POSITIVE);
    let effective_tolerance = options
        .tolerances
        .trade_pnl_abs
        .max(options.tolerances.trade_pnl_relative * evidence.expected_pnl.abs());
    let formula_valid = evidence.expected_pnl.is_finite()
        && absolute_difference.is_finite()
        && effective_tolerance.is_finite()
        && absolute_difference <= effective_tolerance;
    if !formula_valid {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::DerivativePnlFormulaMismatches,
            bill,
            "pnl_formula",
            &format!("{} +/- {}", evidence.expected_pnl, effective_tolerance),
            &observed_pnl.to_string(),
            "derivative trade PnL does not match the attested opening basis and configured contract formula",
        );
    }
    Some(DerivativePnlFormulaSample {
        bill_id: bill.bill_id.clone(),
        symbol: bill.symbol.clone(),
        trade_id: bill.trade_id.clone(),
        runtime_session_id: evidence.runtime_session_id.clone(),
        runtime_session_start_line: evidence.runtime_session_start_line,
        snapshot_line: evidence.basis.snapshot_line,
        snapshot_time_ms: evidence.basis.snapshot_time_ms,
        fill_line: evidence.fill_line,
        fill_time_ms: fill.ts_ms,
        inverse: instrument.kind.is_inverse(),
        currency: instrument.settle_currency.trim().to_ascii_uppercase(),
        pre_quantity: evidence.basis.quantity,
        pre_avg_price: evidence.basis.avg_price,
        fill_side: fill.side,
        fill_price: fill.price,
        fill_quantity: fill.qty,
        close_quantity: evidence.close_quantity,
        contract_value: instrument.contract_value,
        post_quantity: evidence.post_quantity,
        post_avg_price: evidence.post_avg_price,
        expected_sub_type: evidence.expected_sub_type.clone(),
        observed_sub_type: bill.sub_type.clone(),
        expected_pnl: evidence.expected_pnl,
        observed_pnl,
        absolute_difference,
        relative_difference,
        effective_tolerance,
        validated: subtype_valid && formula_valid,
    })
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
            minimum_derivative_close_bills: 1,
            minimum_funding_bills: 1,
            maximum_trade_bill_delay_ms: 10_000,
            maximum_funding_bill_delay_ms: 10_000,
            maximum_funding_mark_bracket_distance_ms: 1_000,
            maximum_account_boundary_gap_ms: 10_000,
            tolerances: EconomicReconciliationTolerances {
                price_abs: 0.0,
                quantity_abs: 1e-9,
                fee_abs: 1e-12,
                balance_abs: 1e-12,
                trade_pnl_abs: 1e-12,
                trade_pnl_relative: 1e-12,
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

    fn account_boundary(
        path: &str,
        fingerprint: &str,
        start_server_ms: u64,
        finish_server_ms: u64,
        window_gap_ms: u64,
        cash_balance: f64,
    ) -> BoundAccountBoundary {
        let detail = OkxBalanceDetail {
            currency: "USDT".to_string(),
            update_time_ms: finish_server_ms,
            cash_balance: Some(cash_balance),
            available_balance: Some(cash_balance),
            equity: Some(cash_balance),
            equity_usd: Some(cash_balance),
            discounted_equity_usd: Some(cash_balance),
            unrealized_pnl: Some(0.0),
            liability: Some(0.0),
            cross_liability: Some(0.0),
            isolated_liability: None,
            unrealized_loss_liability: None,
            accrued_interest: Some(0.0),
            borrow_frozen_usd: Some(0.0),
            max_loan: Some(0.0),
            forced_repayment_indicator: Some(0),
        };
        BoundAccountBoundary {
            evidence: EconomicAccountBoundaryEvidence {
                certification_file: evidence(path),
                certification_schema_version: crate::ACCOUNT_CERTIFICATION_SCHEMA_VERSION,
                collector_reap_version: env!("CARGO_PKG_VERSION").to_string(),
                collector_executable_sha256: "c".repeat(64),
                collector_host_identity_sha256: "d".repeat(64),
                start_server_ms,
                finish_server_ms,
                window_gap_ms,
                total_equity_usd: cash_balance,
                balance_currencies: 1,
            },
            account_id: "main".to_string(),
            environment: TradingEnvironment::Demo,
            account_identity_sha256: "b".repeat(64),
            config_fingerprint: fingerprint.to_string(),
            config_source_path: "/config".to_string(),
            config_sha256: "a".repeat(64),
            passed: true,
            balance: OkxAccountBalanceSnapshot {
                update_time_ms: finish_server_ms,
                total_equity_usd: Some(cash_balance),
                adjusted_equity_usd: Some(cash_balance),
                borrow_frozen_usd: Some(0.0),
                notional_usd_for_borrow: Some(0.0),
                margin_ratio: None,
                notional_usd: Some(0.0),
                details: vec![detail],
            },
        }
    }

    fn set_boundary_cash(boundary: &mut BoundAccountBoundary, value: f64) {
        boundary.evidence.total_equity_usd = value;
        boundary.balance.total_equity_usd = Some(value);
        boundary.balance.adjusted_equity_usd = Some(value);
        boundary.balance.details[0].cash_balance = Some(value);
        boundary.balance.details[0].available_balance = Some(value);
        boundary.balance.details[0].equity = Some(value);
        boundary.balance.details[0].equity_usd = Some(value);
        boundary.balance.details[0].discounted_equity_usd = Some(value);
    }

    fn swap_fill() -> RemoteFill {
        RemoteFill {
            fill_id: "trade-1".to_string(),
            exchange_order_id: "exchange-1".to_string(),
            client_order_id: "reap-1".to_string(),
            symbol: "BTC-USDT-SWAP".to_string(),
            side: Side::Sell,
            price: 50_000.0,
            qty: 2.0,
            liquidity: FillLiquidity::Taker,
            fee: Some(FillFee {
                amount: -0.5,
                currency: "USDT".to_string(),
            }),
            ts_ms: TRADE_MS,
        }
    }

    fn trade_bill() -> OkxBill {
        OkxBill {
            bill_id: "100".to_string(),
            bill_type: "2".to_string(),
            sub_type: "5".to_string(),
            timestamp_ms: TRADE_MS + 1,
            currency: "USDT".to_string(),
            balance_change: 19.5,
            balance: Some(1_000.0),
            position_balance_change: Some(0.0),
            position_balance: Some(0.0),
            quantity: Some(2.0),
            price: Some(50_000.0),
            pnl: Some(20.0),
            fee: Some(-0.5),
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
            bill_id: "200".to_string(),
            bill_type: "8".to_string(),
            sub_type: "173".to_string(),
            timestamp_ms: FUNDING_MS + 100,
            currency: "USDT".to_string(),
            balance_change: -4.0,
            balance: Some(996.0),
            position_balance_change: Some(0.0),
            position_balance: Some(0.0),
            quantity: Some(8.0),
            price: Some(50_000.0),
            pnl: Some(-4.0),
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
            records: 9,
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
            authoritative_account_snapshots: vec![JournalAuthoritativeAccountSnapshot {
                line: 3,
                event_ts_ms: TRADE_MS - 100,
                update_ts_ms: TRADE_MS - 100,
                account_id: "main".to_string(),
                positions: vec![Position {
                    symbol: "BTC-USDT-SWAP".to_string(),
                    qty: 10.0,
                    avg_price: 49_000.0,
                    margin_mode: Some(reap_core::PositionMarginMode::Cross),
                }],
            }],
            journal_fills: vec![JournalFillObservation {
                line: 4,
                fill: FillRecord {
                    ts_ms: TRADE_MS,
                    account_id: Some("main".to_string()),
                    fill_id: "trade-1".to_string(),
                    order_id: "reap-1".to_string(),
                    symbol: "BTC-USDT-SWAP".to_string(),
                    side: Side::Sell,
                    price: 50_000.0,
                    qty: 2.0,
                    liquidity: Some(FillLiquidity::Taker),
                    fee: Some(FillFee {
                        amount: -0.5,
                        currency: "USDT".to_string(),
                    }),
                },
            }],
            settlements: vec![JournalFundingSettlement {
                line: 6,
                event_ts_ms: FUNDING_MS + 50,
                symbol: "BTC-USDT-SWAP".to_string(),
                funding_time_ms: FUNDING_MS,
                rate: 0.001,
            }],
            position_observations: vec![
                JournalPositionObservation {
                    line: 5,
                    event_ts_ms: FUNDING_MS - 100,
                    symbol: "BTC-USDT-SWAP".to_string(),
                    quantity: 9.0,
                },
                JournalPositionObservation {
                    line: 7,
                    event_ts_ms: FUNDING_MS + 75,
                    symbol: "BTC-USDT-SWAP".to_string(),
                    quantity: 8.0,
                },
            ],
            mark_price_observations: vec![
                JournalMarkPriceObservation {
                    line: 8,
                    event_ts_ms: FUNDING_MS + 90,
                    symbol: "BTC-USDT-SWAP".to_string(),
                    price: 50_000.0,
                },
                JournalMarkPriceObservation {
                    line: 9,
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
            config_fingerprint: fingerprint.clone(),
            window: BillCollectionWindow {
                begin_ms: BEGIN_MS,
                end_ms: END_MS,
                endpoints_inclusive: true,
                minimum_close_delay_ms: 1,
            },
            opening_account_boundary: account_boundary(
                "/opening-account",
                &fingerprint,
                BEGIN_MS - 2_000,
                BEGIN_MS - 1_000,
                1_000,
                980.5,
            ),
            closing_account_boundary: account_boundary(
                "/closing-account",
                &fingerprint,
                END_MS + 1_000,
                END_MS + 2_000,
                1_000,
                996.0,
            ),
        }
    }

    #[test]
    fn validates_normal_trade_and_linear_funding_from_exact_sources() {
        let report = build_report(sources(), options(), "c".repeat(64));

        assert!(report.passed, "{:?}", report.issues);
        assert!(report.failures.is_empty());
        assert_eq!(report.counts.trade_bills_validated, 1);
        assert_eq!(report.counts.derivative_close_bills_recomputed, 1);
        assert_eq!(report.counts.funding_bills_validated, 1);
        assert_eq!(report.counts.eligible_fills_missing_bill, 0);
        assert_eq!(report.funding_formula_samples.len(), 1);
        assert_eq!(report.derivative_pnl_formula_samples.len(), 1);
        assert_eq!(
            report
                .journal_recovery
                .authoritative_account_snapshot_records,
            1
        );
        assert_eq!(report.journal_recovery.journal_fill_records, 1);
        assert_eq!(report.journal_recovery.position_observation_records, 2);
        assert_eq!(report.journal_recovery.mark_price_observation_records, 2);
        assert_eq!(report.journal_recovery.runtime_session_records, 1);
        assert_eq!(report.counts.funding_mark_brackets_validated, 1);
        assert_eq!(report.counts.cash_balance_currencies, 1);
        assert_eq!(report.counts.cash_balance_currencies_validated, 1);
        assert_eq!(report.counts.cash_balance_chain_links, 3);
        assert_eq!(report.counts.cash_balance_chain_links_validated, 3);
        assert_eq!(report.currency_balance_continuity.len(), 1);
        assert!(report.currency_balance_continuity[0].validated);
        assert_eq!(report.currency_balance_continuity[0].bill_count, 2);
        assert_eq!(
            report.currency_balance_continuity[0].summed_balance_change,
            15.5
        );
        assert_eq!(report.total_equity_change_usd, 15.5);
        assert_eq!(
            report.funding_formula_samples[0].expected_pnl_at_bill_mark,
            -4.0
        );
        assert_eq!(report.funding_formula_samples[0].expected_pnl_absolute, 4.0);
        assert_eq!(
            report.funding_formula_samples[0].journal_position_quantity,
            8.0
        );
        assert_eq!(
            report.funding_formula_samples[0].position_observation_line,
            7
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
        assert_eq!(report.derivative_pnl_formula_samples[0].pre_quantity, 10.0);
        assert_eq!(
            report.derivative_pnl_formula_samples[0].pre_avg_price,
            49_000.0
        );
        assert_eq!(report.derivative_pnl_formula_samples[0].close_quantity, 2.0);
        assert_eq!(report.derivative_pnl_formula_samples[0].expected_pnl, 20.0);
        assert_eq!(report.derivative_pnl_formula_samples[0].post_quantity, 8.0);
        assert!(report.derivative_pnl_formula_samples[0].validated);
    }

    #[test]
    fn cash_continuity_requires_every_bill_post_balance() {
        let mut sources = sources();
        sources.bills[0].balance = None;

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::InvalidBillBalanceChain)
        );
        assert!(report.issues.iter().any(|issue| issue.field == "bal"));
        assert_eq!(report.counts.cash_balance_chain_links, 3);
        assert!(report.counts.cash_balance_chain_links_validated < 3);
    }

    #[test]
    fn cash_continuity_rejects_a_broken_intermediate_bill_link() {
        let mut sources = sources();
        sources.bills[1].balance = Some(995.0);

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::InvalidBillBalanceChain)
        );
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.field == "bill_balance_chain")
        );
    }

    #[test]
    fn cash_continuity_rejects_a_certified_endpoint_delta_mismatch() {
        let mut sources = sources();
        set_boundary_cash(&mut sources.closing_account_boundary, 997.0);

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::CashBalanceContinuityMismatches)
        );
        assert!(!report.currency_balance_continuity[0].validated);
        assert_eq!(
            report.currency_balance_continuity[0].expected_closing_cash_balance,
            996.0
        );
    }

    #[test]
    fn account_boundary_timing_and_numeric_bill_ids_fail_closed() {
        let mut timing = sources();
        timing.opening_account_boundary.evidence.finish_server_ms = BEGIN_MS + 1;
        timing.opening_account_boundary.evidence.window_gap_ms = 0;
        let report = build_report(timing, options(), "c".repeat(64));
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::InvalidAccountBoundaries)
        );

        let mut nonnumeric = sources();
        nonnumeric.bills[0].bill_id = "not-numeric".to_string();
        let report = build_report(nonnumeric, options(), "c".repeat(64));
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::InvalidBillBalanceChain)
        );
    }

    #[test]
    fn derivative_pnl_tamper_fails_even_when_balance_equation_is_self_consistent() {
        let mut sources = sources();
        sources.bills[0].pnl = Some(19.0);
        sources.bills[0].balance_change = 18.5;

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::DerivativePnlFormulaMismatches)
        );
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::MinimumDerivativeCloseBillsNotMet)
        );
        assert_eq!(report.derivative_pnl_formula_samples[0].expected_pnl, 20.0);
        assert_eq!(report.derivative_pnl_formula_samples[0].observed_pnl, 19.0);
        assert!(!report.derivative_pnl_formula_samples[0].validated);
    }

    #[test]
    fn derivative_close_requires_a_same_session_authoritative_basis() {
        let mut sources = sources();
        sources.authoritative_account_snapshots.clear();

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::DerivativeOpeningBasisMissingOrInvalid)
        );
        assert!(report.derivative_pnl_formula_samples.is_empty());
        assert_eq!(report.counts.derivative_close_bills_recomputed, 0);
    }

    #[test]
    fn derivative_basis_must_strictly_precede_the_fill_exchange_time() {
        let mut sources = sources();
        sources.authoritative_account_snapshots[0].event_ts_ms = TRADE_MS;
        sources.authoritative_account_snapshots[0].update_ts_ms = TRADE_MS;

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::DerivativeOpeningBasisMissingOrInvalid)
        );
        assert!(report.derivative_pnl_formula_samples.is_empty());
        assert_eq!(report.counts.derivative_close_bills_recomputed, 0);
    }

    #[test]
    fn derivative_basis_margin_mode_must_match_the_account_configuration() {
        let mut sources = sources();
        sources.authoritative_account_snapshots[0].positions[0].margin_mode =
            Some(PositionMarginMode::Isolated);

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::InvalidAuthoritativeAccountSnapshots)
        );
        assert!(report.derivative_pnl_formula_samples.is_empty());
    }

    #[test]
    fn derivative_basis_session_must_match_the_collected_account_identity() {
        let mut sources = sources();
        sources.runtime_sessions[0].account_identity_sha256 = "d".repeat(64);

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::InvalidAuthoritativeAccountSnapshots)
        );
        assert!(report.derivative_pnl_formula_samples.is_empty());
    }

    #[test]
    fn derivative_close_requires_the_exact_critical_journal_fill() {
        let mut sources = sources();
        sources.journal_fills[0].fill.qty = 1.0;

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::TradeJournalFillMismatches)
        );
        assert!(report.derivative_pnl_formula_samples.is_empty());
    }

    #[test]
    fn derivative_basis_rejects_an_uncollected_intervening_fill() {
        let mut sources = sources();
        sources.journal_fills[0].line = 5;
        sources.journal_fills.insert(
            0,
            JournalFillObservation {
                line: 4,
                fill: FillRecord {
                    ts_ms: TRADE_MS - 50,
                    account_id: Some("main".to_string()),
                    fill_id: "uncollected".to_string(),
                    order_id: "reap-uncollected".to_string(),
                    symbol: "BTC-USDT-SWAP".to_string(),
                    side: Side::Buy,
                    price: 49_500.0,
                    qty: 1.0,
                    liquidity: Some(FillLiquidity::Maker),
                    fee: None,
                },
            },
        );

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::DerivativeOpeningBasisMissingOrInvalid)
        );
        assert!(report.derivative_pnl_formula_samples.is_empty());
        assert_eq!(report.counts.derivative_close_bills_recomputed, 0);
    }

    #[test]
    fn duplicate_critical_journal_fill_identity_fails_closed() {
        let mut sources = sources();
        let mut duplicate = sources.journal_fills[0].clone();
        duplicate.line = 5;
        sources.journal_fills.push(duplicate);

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::TradeJournalFillMismatches)
        );
        assert!(report.derivative_pnl_formula_samples.is_empty());
    }

    #[test]
    fn inverse_position_basis_uses_pinned_java_harmonic_average() {
        let mut inverse = config()
            .strategy
            .instruments
            .into_iter()
            .find(|instrument| instrument.symbol == "BTC-USDT-SWAP")
            .unwrap();
        inverse.kind = InstrumentKindConfig::InverseSwap;
        inverse.contract_value = 100.0;
        let increase = FillRecord {
            ts_ms: 2,
            account_id: Some("main".to_string()),
            fill_id: "increase".to_string(),
            order_id: "reap-increase".to_string(),
            symbol: inverse.symbol.clone(),
            side: Side::Buy,
            price: 20_000.0,
            qty: 1.0,
            liquidity: Some(FillLiquidity::Maker),
            fee: None,
        };
        let increased = apply_derivative_fill(
            PositionBasis {
                quantity: 2.0,
                avg_price: 10_000.0,
                snapshot_line: 1,
                snapshot_time_ms: 1,
            },
            &increase,
            &inverse,
            1e-12,
        )
        .unwrap();
        assert!((increased.post_avg_price - 12_000.0).abs() < 1e-9);

        let close = FillRecord {
            ts_ms: 3,
            fill_id: "close".to_string(),
            order_id: "reap-close".to_string(),
            side: Side::Sell,
            price: 15_000.0,
            qty: 1.0,
            ..increase
        };
        let closed = apply_derivative_fill(
            PositionBasis {
                quantity: increased.post_quantity,
                avg_price: increased.post_avg_price,
                snapshot_line: 1,
                snapshot_time_ms: 1,
            },
            &close,
            &inverse,
            1e-12,
        )
        .unwrap();
        let expected = 100.0 * (1.0 / 12_000.0 - 1.0 / 15_000.0);
        assert!((closed.expected_pnl - expected).abs() < 1e-15);
        assert_eq!(closed.expected_sub_type, "5");
        assert_eq!(closed.post_quantity, 2.0);
        assert!((closed.post_avg_price - 12_000.0).abs() < 1e-9);
    }

    #[test]
    fn funding_formula_tamper_fails_even_when_balance_equation_is_self_consistent() {
        let mut sources = sources();
        sources.bills[1].pnl = Some(-3.0);
        sources.bills[1].balance_change = -3.0;

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
        assert_eq!(report.funding_formula_samples[0].expected_pnl_absolute, 4.0);
        assert_eq!(report.funding_formula_samples[0].observed_pnl, -3.0);
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
        transfer.bill_id = "300".to_string();
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
            bill_id: "100".to_string(),
            bill_type: "2".to_string(),
            sub_type: "2".to_string(),
            timestamp_ms: TRADE_MS + 1,
            currency: "USDT".to_string(),
            balance_change: 499.95,
            balance: Some(1_000.0),
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
        sources.journal_fills[0] = JournalFillObservation {
            line: 4,
            fill: FillRecord {
                ts_ms: TRADE_MS,
                account_id: Some("main".to_string()),
                fill_id: "trade-spot".to_string(),
                order_id: "reap-spot".to_string(),
                symbol: "BTC-USDT".to_string(),
                side: Side::Sell,
                price: 50_000.0,
                qty: 0.01,
                liquidity: Some(FillLiquidity::Maker),
                fee: Some(FillFee {
                    amount: -0.05,
                    currency: "USDT".to_string(),
                }),
            },
        };
        let mut options = options();
        options.minimum_derivative_close_bills = 0;
        set_boundary_cash(&mut sources.opening_account_boundary, 500.05);

        let report = build_report(sources, options, "c".repeat(64));

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
            line: 10,
            started_at_ms: FUNDING_MS + 110,
            session_id: "4d5e6f".to_string(),
            account_id: "main".to_string(),
            strategy_name: sources.config.strategy.strategy_name.clone(),
            config_fingerprint: sources.config_fingerprint.clone(),
            account_identity_sha256: sources.account_identity_sha256.clone(),
        });
        let mut replay = sources.settlements[0].clone();
        replay.line = 11;
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
        sources.position_observations[1].quantity = -8.0;

        let report = build_report(sources, options(), "c".repeat(64));

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&EconomicReconciliationFailure::FundingFormulaMismatches)
        );
        assert_eq!(
            report.funding_formula_samples[0].expected_pnl_at_bill_mark,
            4.0
        );
        assert_eq!(report.funding_formula_samples[0].observed_pnl, -4.0);
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
        duplicate.line = 10;
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
            line: 10,
            started_at_ms: FUNDING_MS + 110,
            session_id: "4d5e6f".to_string(),
            account_id: "main".to_string(),
            strategy_name: sources.config.strategy.strategy_name.clone(),
            config_fingerprint: sources.config_fingerprint.clone(),
            account_identity_sha256: sources.account_identity_sha256.clone(),
        });
        let mut replay = sources.mark_price_observations[1].clone();
        replay.line = 11;
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
            line: 9,
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
                line: 10,
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
