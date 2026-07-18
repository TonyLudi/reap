use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use reap_core::{MarketEvent, NormalizedEvent, Position, Side};
use reap_storage::{
    FillRecord, RecoveredStorage, StorageError, StorageRecord, acquire_storage_lease,
    recover_jsonl_bytes_with_visitor,
};
use reap_venue::RemoteFill;
use reap_venue::okx::{OkxAccountBalanceSnapshot, OkxBill};
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod artifacts;
mod cash_continuity;
mod funding_bills;
mod position_basis;
mod report;
mod support;
mod trade_bills;

use crate::provenance::current_executable_sha256;
use crate::{
    AccountCertificationError, BillCollectionError, BillCollectionWindow, FillCollectionError,
    FillCollectionFileEvidence, LiveConfig, LiveConfigError, TradingEnvironment,
    verify_bill_collection_manifest_path, verify_fill_collection_manifest_path,
};
use artifacts::{
    bind_account_boundaries, bind_collection_manifests, read_account_boundary, read_input,
};
#[cfg(test)]
use position_basis::apply_derivative_fill;
use report::build_report;

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

#[cfg(test)]
#[path = "../tests/economic_statement_unit/mod.rs"]
mod tests;
