use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use reap_core::PINNED_JAVA_REVISION;
use reap_venue::okx::{
    OkxAccountBalanceSnapshot, OkxAccountConfig, OkxAccountLevel, OkxAccountPositionsSnapshot,
    OkxIndexTickerSnapshot, OkxInstrumentType, RestError, parse_okx_account_balance_response_json,
    parse_okx_account_config_response_json, parse_okx_account_positions_response_json,
    parse_okx_index_ticker_response_json,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    LiveConfig, LiveConfigError, OkxApiKeyPolicyEvaluation, OkxTradeModeConfig, TradingEnvironment,
    evaluate_okx_api_key_policy,
};

pub const ACCOUNT_CASH_POLICY_VERSION: u32 = 1;
pub const ACCOUNT_CERTIFICATION_SCHEMA_VERSION: u32 = 3;
pub const MAX_ACCOUNT_CERTIFICATION_CONFIG_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_ACCOUNT_CERTIFICATION_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES: u64 = 48 * 1024 * 1024;
pub const MAX_ACCOUNT_CERTIFICATION_SPAN_MS: u64 = 30_000;
pub const MAX_ACCOUNT_CERTIFICATION_INDEX_STALENESS_MS: u64 = 10_000;
pub const ACCOUNT_EQUITY_AGGREGATE_ABS_TOLERANCE_USD: f64 = 1e-8;
pub const ACCOUNT_EQUITY_AGGREGATE_REL_TOLERANCE: f64 = 1e-12;
pub const ACCOUNT_EQUITY_INDEX_ABS_TOLERANCE_USD: f64 = 0.01;
pub const ACCOUNT_EQUITY_INDEX_REL_TOLERANCE: f64 = 0.001;

const ACCOUNT_CONFIG_ENDPOINT: &str = "/api/v5/account/config";
const ACCOUNT_BALANCE_ENDPOINT: &str = "/api/v5/account/balance";
const ACCOUNT_POSITIONS_ENDPOINT: &str = "/api/v5/account/positions";
const MARKET_INDEX_TICKERS_ENDPOINT: &str = "/api/v5/market/index-tickers";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountCertificationCoverage {
    PointInTimeCashAndZeroLiability,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountCertificationClockEvidence {
    pub local_midpoint_ms: u64,
    pub server_ms: u64,
    pub absolute_skew_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountCertificationConfigEvidence {
    pub source_path: String,
    pub bytes: u64,
    pub sha256: String,
    pub toml: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountCertificationResponseEvidence {
    pub endpoint: String,
    pub bytes: u64,
    pub sha256: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountCertificationIndexEvidence {
    pub currency: String,
    pub symbol: String,
    pub response: AccountCertificationResponseEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountEquityConversionSample {
    pub currency: String,
    pub native_equity: f64,
    pub reported_equity_usd: f64,
    pub index_symbol: Option<String>,
    pub index_price: f64,
    pub index_timestamp_ms: Option<u64>,
    pub independently_converted_equity_usd: f64,
    pub absolute_difference: f64,
    pub relative_difference: f64,
    pub effective_tolerance_usd: f64,
    pub validated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountEquityEvaluation {
    pub total_equity_usd: Option<f64>,
    pub reported_currency_equity_usd: f64,
    pub independently_converted_equity_usd: f64,
    pub aggregate_reported_difference_usd: Option<f64>,
    pub aggregate_independent_difference_usd: Option<f64>,
    pub aggregate_reported_tolerance_usd: Option<f64>,
    pub aggregate_independent_tolerance_usd: Option<f64>,
    pub currencies: u64,
    pub direct_index_tickers: u64,
    pub conversion_samples: Vec<AccountEquityConversionSample>,
    pub evidence_complete: bool,
    pub passed: bool,
    pub violations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountCertificationSummary {
    pub coverage: AccountCertificationCoverage,
    pub environment: TradingEnvironment,
    pub account_id: String,
    pub account_identity_sha256: String,
    pub account_identity_stable: bool,
    pub account_settings_stable: bool,
    pub clock_evidence_valid: bool,
    pub api_key_policy: OkxApiKeyPolicyEvaluation,
    pub policy: AccountCashPolicyEvaluation,
    pub equity: AccountEquityEvaluation,
    pub evidence_complete: bool,
    pub passed: bool,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountCertificationArtifact {
    pub schema_version: u32,
    pub java_reference_revision: String,
    pub reap_version: String,
    pub executable_sha256: String,
    pub host_identity_sha256: String,
    pub config: AccountCertificationConfigEvidence,
    pub config_fingerprint: String,
    pub start_clock: AccountCertificationClockEvidence,
    pub finish_clock: AccountCertificationClockEvidence,
    pub account_config_before: AccountCertificationResponseEvidence,
    pub account_balance: AccountCertificationResponseEvidence,
    pub index_tickers: Vec<AccountCertificationIndexEvidence>,
    pub account_positions: AccountCertificationResponseEvidence,
    pub account_config_after: AccountCertificationResponseEvidence,
    pub summary: AccountCertificationSummary,
}

/// A point-in-time, mode-aware evaluation of the account state that Reap can
/// account for safely. It intentionally does not claim historical absence of
/// borrowing or statement-level economic reconciliation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountCashPolicyEvaluation {
    pub policy_version: u32,
    pub account_mode_matches: bool,
    pub configured_spot_cash_only: bool,
    pub configured_borrow_limits_zero: bool,
    pub borrowing_disabled: bool,
    pub liability_evidence_complete: bool,
    pub liabilities_zero: bool,
    pub margin_positions_absent: bool,
    pub passed: bool,
    pub violations: Vec<String>,
}

/// Credential-free failures produced while parsing or re-deriving embedded
/// account-certification evidence. Filesystem authority remains with callers.
#[derive(Debug, Error)]
pub enum AccountCertificationVerificationError {
    #[error("live config {path} is {actual} bytes; limit is {limit}")]
    ConfigTooLarge {
        path: PathBuf,
        actual: u64,
        limit: u64,
    },
    #[error("live configuration is invalid: {0}")]
    Config(#[from] LiveConfigError),
    #[error("OKX account certification failed: {0}")]
    Rest(#[from] RestError),
    #[error("account-certification response for {endpoint} is {actual} bytes; limit is {limit}")]
    ResponseTooLarge {
        endpoint: String,
        actual: u64,
        limit: u64,
    },
    #[error("failed to parse account-certification artifact {path}: {source}")]
    ParseArtifact {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("account-certification artifact is {actual} bytes; limit is {limit}")]
    ArtifactTooLarge { actual: u64, limit: u64 },
    #[error("invalid account-certification evidence: {0}")]
    InvalidEvidence(String),
}

/// Parses and verifies already-read artifact bytes. The source path is an
/// evidence label used only for preserving parse-error context; this function
/// performs no filesystem access.
pub fn verify_account_certification_artifact_bytes(
    bytes: &[u8],
    source_path: impl Into<PathBuf>,
) -> Result<AccountCertificationArtifact, AccountCertificationVerificationError> {
    if bytes.len() as u64 > MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES {
        return Err(AccountCertificationVerificationError::ArtifactTooLarge {
            actual: bytes.len() as u64,
            limit: MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES,
        });
    }
    let path = source_path.into();
    let artifact = serde_json::from_slice(bytes).map_err(|source| {
        AccountCertificationVerificationError::ParseArtifact {
            path: path.clone(),
            source,
        }
    })?;
    verify_account_certification_artifact(artifact)
}

/// Re-derives a deserialized artifact entirely from its embedded evidence.
fn verify_account_certification_artifact(
    artifact: AccountCertificationArtifact,
) -> Result<AccountCertificationArtifact, AccountCertificationVerificationError> {
    validate_artifact_header(&artifact)?;

    let config_bytes = artifact.config.toml.as_bytes();
    if artifact.config.bytes != config_bytes.len() as u64
        || artifact.config.sha256 != sha256_bytes(config_bytes)
    {
        return invalid_evidence("embedded live config byte count or SHA-256 does not match");
    }
    if artifact.config.bytes > MAX_ACCOUNT_CERTIFICATION_CONFIG_BYTES {
        return Err(AccountCertificationVerificationError::ConfigTooLarge {
            path: PathBuf::from(&artifact.config.source_path),
            actual: artifact.config.bytes,
            limit: MAX_ACCOUNT_CERTIFICATION_CONFIG_BYTES,
        });
    }
    let config = LiveConfig::from_toml(&artifact.config.toml)?;
    if config.fingerprint()? != artifact.config_fingerprint {
        return invalid_evidence("embedded live config fingerprint does not match");
    }
    if config.venue.environment != artifact.summary.environment {
        return invalid_evidence("embedded live config environment does not match the summary");
    }
    if config.account(&artifact.summary.account_id).is_none() {
        return invalid_evidence("summary account does not exist in the embedded live config");
    }

    validate_response_evidence(&artifact.account_config_before, ACCOUNT_CONFIG_ENDPOINT)?;
    validate_response_evidence(&artifact.account_balance, ACCOUNT_BALANCE_ENDPOINT)?;
    validate_response_evidence(&artifact.account_positions, ACCOUNT_POSITIONS_ENDPOINT)?;
    validate_response_evidence(&artifact.account_config_after, ACCOUNT_CONFIG_ENDPOINT)?;
    let config_before =
        parse_okx_account_config_response_json(artifact.account_config_before.body.as_bytes())?;
    let balance =
        parse_okx_account_balance_response_json(artifact.account_balance.body.as_bytes())?;
    let index_tickers = verify_index_ticker_evidence(&balance, &artifact.index_tickers)?;
    let positions =
        parse_okx_account_positions_response_json(artifact.account_positions.body.as_bytes())?;
    let config_after =
        parse_okx_account_config_response_json(artifact.account_config_after.body.as_bytes())?;
    let derived = derive_account_certification_summary(
        &config,
        &artifact.summary.account_id,
        &config_before,
        &balance,
        &index_tickers,
        &positions,
        &config_after,
        &artifact.start_clock,
        &artifact.finish_clock,
    )?;
    if artifact.summary != derived {
        return invalid_evidence(
            "stored account-certification summary does not match raw evidence",
        );
    }
    Ok(artifact)
}

pub fn evaluate_account_cash_policy(
    config: &LiveConfig,
    account_id: &str,
    account: &OkxAccountConfig,
    balance: &OkxAccountBalanceSnapshot,
    positions: &OkxAccountPositionsSnapshot,
) -> AccountCashPolicyEvaluation {
    let mut violations = BTreeSet::new();
    let Some(expected) = config.account(account_id) else {
        violations.insert(format!("configured account {account_id} does not exist"));
        return finish_evaluation(false, false, false, false, false, false, false, violations);
    };

    let account_mode_matches = account.account_level == expected.expected_account_level
        && account.position_mode == expected.expected_position_mode;
    if account.account_level != expected.expected_account_level {
        violations.insert(format!(
            "account level {:?} does not match configured {:?}",
            account.account_level, expected.expected_account_level
        ));
    }
    if account.position_mode != expected.expected_position_mode {
        violations.insert(format!(
            "position mode {:?} does not match configured {:?}",
            account.position_mode, expected.expected_position_mode
        ));
    }

    let mut configured_spot_cash_only = true;
    for instrument in config.instruments_for_account(account_id) {
        if instrument.kind.is_spot()
            && expected.trade_modes.get(&instrument.symbol) != Some(&OkxTradeModeConfig::Cash)
        {
            configured_spot_cash_only = false;
            violations.insert(format!(
                "spot symbol {} is not configured with cash trade mode",
                instrument.symbol
            ));
        }
    }

    let mut configured_borrow_limits_zero = true;
    for group in config
        .strategy
        .risk_groups
        .iter()
        .filter(|group| group.account_id.as_deref() == Some(account_id))
    {
        for coin in &group.coins {
            if coin.borrow_limit_usd != 0.0 || coin.borrow_limit_coin != 0.0 {
                configured_borrow_limits_zero = false;
                violations.insert(format!(
                    "risk group {} currency {} has a nonzero configured borrow limit",
                    group.name, coin.currency
                ));
            }
        }
    }

    let mut borrowing_disabled = true;
    for (field, value) in [
        ("enableSpotBorrow", account.enable_spot_borrow),
        ("autoLoan", account.auto_loan),
    ] {
        if value == Some(true) {
            borrowing_disabled = false;
            violations.insert(format!("account setting {field} is enabled"));
        }
    }
    match account.account_level {
        OkxAccountLevel::Simple => {
            if account.enable_spot_borrow.is_none() {
                borrowing_disabled = false;
                violations
                    .insert("account setting enableSpotBorrow is absent for Spot mode".to_string());
            }
        }
        OkxAccountLevel::MultiCurrencyMargin | OkxAccountLevel::PortfolioMargin => {
            if account.auto_loan.is_none() {
                borrowing_disabled = false;
                violations.insert(
                    "account setting autoLoan is absent for a borrowing-capable account mode"
                        .to_string(),
                );
            }
        }
        OkxAccountLevel::SingleCurrencyMargin => {}
    }

    let borrowing_fields_apply = matches!(
        account.account_level,
        OkxAccountLevel::Simple
            | OkxAccountLevel::MultiCurrencyMargin
            | OkxAccountLevel::PortfolioMargin
    );
    let mut liability_evidence_complete = true;
    let mut liabilities_zero = true;
    if balance.update_time_ms == 0 {
        liability_evidence_complete = false;
        violations.insert("account balance uTime is absent or zero".to_string());
    }
    if balance.total_equity_usd.is_none() {
        liability_evidence_complete = false;
        violations.insert("account totalEq is absent".to_string());
    }
    if balance.details.is_empty() {
        liability_evidence_complete = false;
        violations.insert("account balance contains no currency details".to_string());
    }
    check_optional_zero(
        "account.borrowFroz",
        balance.borrow_frozen_usd,
        borrowing_fields_apply,
        &mut liability_evidence_complete,
        &mut liabilities_zero,
        &mut violations,
    );
    check_optional_zero(
        "account.notionalUsdForBorrow",
        balance.notional_usd_for_borrow,
        borrowing_fields_apply,
        &mut liability_evidence_complete,
        &mut liabilities_zero,
        &mut violations,
    );

    for detail in &balance.details {
        let prefix = format!("balance[{}]", detail.currency);
        if detail.currency.trim().is_empty() {
            liability_evidence_complete = false;
            violations.insert("account balance contains an empty currency".to_string());
        }
        if detail.update_time_ms == 0 {
            liability_evidence_complete = false;
            violations.insert(format!("{prefix}.uTime is absent or zero"));
        }
        check_optional_zero(
            &format!("{prefix}.liab"),
            detail.liability,
            borrowing_fields_apply,
            &mut liability_evidence_complete,
            &mut liabilities_zero,
            &mut violations,
        );
        check_optional_zero(
            &format!("{prefix}.crossLiab"),
            detail.cross_liability,
            borrowing_fields_apply,
            &mut liability_evidence_complete,
            &mut liabilities_zero,
            &mut violations,
        );
        check_optional_zero(
            &format!("{prefix}.interest"),
            detail.accrued_interest,
            borrowing_fields_apply,
            &mut liability_evidence_complete,
            &mut liabilities_zero,
            &mut violations,
        );
        check_optional_zero(
            &format!("{prefix}.borrowFroz"),
            detail.borrow_frozen_usd,
            borrowing_fields_apply,
            &mut liability_evidence_complete,
            &mut liabilities_zero,
            &mut violations,
        );
        check_optional_zero(
            &format!("{prefix}.isoLiab"),
            detail.isolated_liability,
            matches!(
                account.account_level,
                OkxAccountLevel::MultiCurrencyMargin | OkxAccountLevel::PortfolioMargin
            ),
            &mut liability_evidence_complete,
            &mut liabilities_zero,
            &mut violations,
        );
        check_optional_zero(
            &format!("{prefix}.uplLiab"),
            detail.unrealized_loss_liability,
            matches!(
                account.account_level,
                OkxAccountLevel::MultiCurrencyMargin | OkxAccountLevel::PortfolioMargin
            ),
            &mut liability_evidence_complete,
            &mut liabilities_zero,
            &mut violations,
        );
    }

    let mut margin_positions_absent = true;
    for risk in &positions.positions {
        let prefix = format!("position[{}]", risk.position.symbol);
        if risk.instrument_type == OkxInstrumentType::Margin {
            margin_positions_absent = false;
            violations.insert(format!("{prefix} is an OKX MARGIN position"));
        }
        for (field, value) in [
            ("liab", risk.liability),
            ("interest", risk.accrued_interest),
            ("pendingCloseOrdLiabVal", risk.pending_close_order_liability),
            ("baseBorrowed", risk.base_borrowed),
            ("baseInterest", risk.base_interest),
            ("quoteBorrowed", risk.quote_borrowed),
            ("quoteInterest", risk.quote_interest),
        ] {
            check_optional_zero(
                &format!("{prefix}.{field}"),
                value,
                false,
                &mut liability_evidence_complete,
                &mut liabilities_zero,
                &mut violations,
            );
        }
    }

    finish_evaluation(
        account_mode_matches,
        configured_spot_cash_only,
        configured_borrow_limits_zero,
        borrowing_disabled,
        liability_evidence_complete,
        liabilities_zero,
        margin_positions_absent,
        violations,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish_evaluation(
    account_mode_matches: bool,
    configured_spot_cash_only: bool,
    configured_borrow_limits_zero: bool,
    borrowing_disabled: bool,
    liability_evidence_complete: bool,
    liabilities_zero: bool,
    margin_positions_absent: bool,
    violations: BTreeSet<String>,
) -> AccountCashPolicyEvaluation {
    let passed = account_mode_matches
        && configured_spot_cash_only
        && configured_borrow_limits_zero
        && borrowing_disabled
        && liability_evidence_complete
        && liabilities_zero
        && margin_positions_absent
        && violations.is_empty();
    AccountCashPolicyEvaluation {
        policy_version: ACCOUNT_CASH_POLICY_VERSION,
        account_mode_matches,
        configured_spot_cash_only,
        configured_borrow_limits_zero,
        borrowing_disabled,
        liability_evidence_complete,
        liabilities_zero,
        margin_positions_absent,
        passed,
        violations: violations.into_iter().collect(),
    }
}

fn check_optional_zero(
    field: &str,
    value: Option<f64>,
    required: bool,
    complete: &mut bool,
    zero: &mut bool,
    violations: &mut BTreeSet<String>,
) {
    match value {
        Some(value) if value != 0.0 => {
            *zero = false;
            violations.insert(format!("{field} is nonzero ({value})"));
        }
        Some(_) => {}
        None if required => {
            *complete = false;
            violations.insert(format!("{field} is absent in an applicable account mode"));
        }
        None => {}
    }
}

#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn derive_account_certification_summary(
    config: &LiveConfig,
    account_id: &str,
    config_before: &OkxAccountConfig,
    balance: &OkxAccountBalanceSnapshot,
    index_tickers: &BTreeMap<String, OkxIndexTickerSnapshot>,
    positions: &OkxAccountPositionsSnapshot,
    config_after: &OkxAccountConfig,
    start_clock: &AccountCertificationClockEvidence,
    finish_clock: &AccountCertificationClockEvidence,
) -> Result<AccountCertificationSummary, AccountCertificationVerificationError> {
    let account_identity_stable = !config_before.user_id.trim().is_empty()
        && !config_before.main_user_id.trim().is_empty()
        && config_before.user_id == config_after.user_id
        && config_before.main_user_id == config_after.main_user_id;
    let account_identity_sha256 = okx_account_identity_sha256(
        config.venue.environment,
        account_id,
        &config_before.user_id,
        &config_before.main_user_id,
    );
    let account_settings_stable = config_before == config_after;
    let clock_evidence_valid = clock_is_valid(
        start_clock,
        finish_clock,
        config.runtime.max_exchange_clock_skew_ms,
    );
    let policy =
        evaluate_account_cash_policy(config, account_id, config_before, balance, positions);
    let account = config.account(account_id).ok_or_else(|| {
        AccountCertificationVerificationError::InvalidEvidence(format!(
            "configured account {account_id} does not exist"
        ))
    })?;
    let api_key_policy = evaluate_okx_api_key_policy(&account.api_key_policy, config_before);
    let equity = evaluate_account_equity(balance, index_tickers, start_clock, finish_clock)?;
    let evidence_complete = api_key_policy.evidence_complete && equity.evidence_complete;
    let passed = account_identity_stable
        && account_settings_stable
        && clock_evidence_valid
        && api_key_policy.passed
        && policy.passed
        && equity.passed
        && evidence_complete;
    Ok(AccountCertificationSummary {
        coverage: AccountCertificationCoverage::PointInTimeCashAndZeroLiability,
        environment: config.venue.environment,
        account_id: account_id.to_string(),
        account_identity_sha256,
        account_identity_stable,
        account_settings_stable,
        clock_evidence_valid,
        api_key_policy,
        policy,
        equity,
        evidence_complete,
        passed,
        limitations: vec![
            "point-in-time evidence only; it does not prove historical absence of borrowing"
                .to_string(),
            "authenticated account GETs and public index GETs are sequential, not an atomic exchange snapshot; quiesce account activity during collection"
                .to_string(),
            "direct public CCY-USD index prices independently check conversion within tolerance but do not expose the exact internal OKX valuation tick"
                .to_string(),
            "collector host and executable hashes are provenance identifiers, not independently authenticated by offline verification"
                .to_string(),
            "it does not reconcile deposits, withdrawals, funding, PnL, taxes, or complete exchange statements"
                .to_string(),
        ],
    })
}

#[doc(hidden)]
pub fn account_certification_required_index_symbols(
    balance: &OkxAccountBalanceSnapshot,
) -> Result<BTreeMap<String, String>, AccountCertificationVerificationError> {
    let mut currencies = BTreeSet::new();
    let mut indexes = BTreeMap::new();
    for detail in &balance.details {
        let currency = detail.currency.as_str();
        let symbol = currency_index_symbol(currency)?;
        if !currencies.insert(currency.to_string()) {
            return invalid_evidence(format!(
                "account balance contains duplicate currency {currency:?}"
            ));
        }
        if let Some(symbol) = symbol {
            indexes.insert(currency.to_string(), symbol);
        }
    }
    Ok(indexes)
}

fn currency_index_symbol(
    currency: &str,
) -> Result<Option<String>, AccountCertificationVerificationError> {
    if currency.is_empty()
        || currency.len() > 32
        || !currency
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    {
        return invalid_evidence(format!(
            "balance currency {currency:?} must be 1-32 uppercase ASCII letters or digits"
        ));
    }
    Ok((currency != "USD").then(|| format!("{currency}-USD")))
}

#[doc(hidden)]
pub fn account_certification_index_ticker_endpoint(symbol: &str) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("instId", symbol);
    format!("{MARKET_INDEX_TICKERS_ENDPOINT}?{}", query.finish())
}

fn verify_index_ticker_evidence(
    balance: &OkxAccountBalanceSnapshot,
    evidence: &[AccountCertificationIndexEvidence],
) -> Result<BTreeMap<String, OkxIndexTickerSnapshot>, AccountCertificationVerificationError> {
    let expected = account_certification_required_index_symbols(balance)?;
    let mut parsed = BTreeMap::new();
    for item in evidence {
        let Some(expected_symbol) = expected.get(&item.currency) else {
            return invalid_evidence(format!(
                "unexpected index-ticker evidence for currency {:?}",
                item.currency
            ));
        };
        if &item.symbol != expected_symbol {
            return invalid_evidence(format!(
                "index symbol {:?} for currency {} does not match expected {:?}",
                item.symbol, item.currency, expected_symbol
            ));
        }
        let endpoint = account_certification_index_ticker_endpoint(expected_symbol);
        validate_response_evidence(&item.response, &endpoint)?;
        let ticker = parse_okx_index_ticker_response_json(item.response.body.as_bytes())?;
        if ticker.symbol != *expected_symbol {
            return invalid_evidence(format!(
                "index response symbol {:?} does not match expected {:?}",
                ticker.symbol, expected_symbol
            ));
        }
        if parsed.insert(item.currency.clone(), ticker).is_some() {
            return invalid_evidence(format!(
                "duplicate index-ticker evidence for currency {}",
                item.currency
            ));
        }
    }
    if parsed.len() != expected.len() {
        let missing = expected
            .keys()
            .filter(|currency| !parsed.contains_key(*currency))
            .cloned()
            .collect::<Vec<_>>();
        return invalid_evidence(format!(
            "index-ticker evidence set is incomplete; missing currencies {missing:?}"
        ));
    }
    Ok(parsed)
}

fn evaluate_account_equity(
    balance: &OkxAccountBalanceSnapshot,
    index_tickers: &BTreeMap<String, OkxIndexTickerSnapshot>,
    start_clock: &AccountCertificationClockEvidence,
    finish_clock: &AccountCertificationClockEvidence,
) -> Result<AccountEquityEvaluation, AccountCertificationVerificationError> {
    let expected_indexes = account_certification_required_index_symbols(balance)?;
    let mut violations = BTreeSet::new();
    let mut evidence_complete = true;
    if balance.details.is_empty() {
        evidence_complete = false;
        violations.insert("account balance contains no currency equity details".to_string());
    }
    let actual_index_currencies = index_tickers.keys().cloned().collect::<BTreeSet<_>>();
    let expected_index_currencies = expected_indexes.keys().cloned().collect::<BTreeSet<_>>();
    if actual_index_currencies != expected_index_currencies {
        evidence_complete = false;
        violations.insert(format!(
            "direct index evidence currencies {actual_index_currencies:?} do not match expected {expected_index_currencies:?}"
        ));
    }

    let total_equity_usd = match balance.total_equity_usd {
        Some(value) if value.is_finite() => Some(value),
        Some(_) => {
            evidence_complete = false;
            violations.insert("account totalEq is not finite".to_string());
            None
        }
        None => {
            evidence_complete = false;
            violations.insert("account totalEq is absent".to_string());
            None
        }
    };
    let maximum_future_ms = start_clock
        .absolute_skew_ms
        .max(finish_clock.absolute_skew_ms);
    let oldest_index_ms = start_clock
        .server_ms
        .saturating_sub(MAX_ACCOUNT_CERTIFICATION_INDEX_STALENESS_MS);
    let newest_index_ms = finish_clock.server_ms.saturating_add(maximum_future_ms);
    let mut reported_currency_equity_usd = 0.0;
    let mut independently_converted_equity_usd = 0.0;
    let mut conversion_samples = Vec::new();

    for detail in &balance.details {
        let currency = detail.currency.as_str();
        let mut sample_valid = true;
        let native_equity = match detail.equity {
            Some(value) if value.is_finite() => value,
            Some(_) => {
                evidence_complete = false;
                violations.insert(format!("balance[{currency}].eq is not finite"));
                continue;
            }
            None => {
                evidence_complete = false;
                violations.insert(format!("balance[{currency}].eq is absent"));
                continue;
            }
        };
        let reported_equity_usd = match detail.equity_usd {
            Some(value) if value.is_finite() => value,
            Some(_) => {
                evidence_complete = false;
                violations.insert(format!("balance[{currency}].eqUsd is not finite"));
                continue;
            }
            None => {
                evidence_complete = false;
                violations.insert(format!("balance[{currency}].eqUsd is absent"));
                continue;
            }
        };
        match detail.cash_balance {
            Some(value) if value.is_finite() => {}
            Some(_) => {
                evidence_complete = false;
                sample_valid = false;
                violations.insert(format!("balance[{currency}].cashBal is not finite"));
            }
            None => {
                evidence_complete = false;
                sample_valid = false;
                violations.insert(format!("balance[{currency}].cashBal is absent"));
            }
        }

        let (index_symbol, index_price, index_timestamp_ms) = if currency == "USD" {
            (None, 1.0, None)
        } else if let Some(ticker) = index_tickers.get(currency) {
            let expected_symbol = expected_indexes
                .get(currency)
                .expect("required index was derived from the same currency set");
            if ticker.symbol != *expected_symbol {
                evidence_complete = false;
                sample_valid = false;
                violations.insert(format!(
                    "index ticker for {currency} has symbol {:?}; expected {:?}",
                    ticker.symbol, expected_symbol
                ));
            }
            if !ticker.index_price.is_finite() || ticker.index_price <= 0.0 {
                evidence_complete = false;
                sample_valid = false;
                violations.insert(format!(
                    "index ticker for {currency} has invalid price {}",
                    ticker.index_price
                ));
            }
            if ticker.timestamp_ms < oldest_index_ms || ticker.timestamp_ms > newest_index_ms {
                evidence_complete = false;
                sample_valid = false;
                violations.insert(format!(
                    "index ticker for {currency} has timestamp {}; accepted range is {}..={}",
                    ticker.timestamp_ms, oldest_index_ms, newest_index_ms
                ));
            }
            (
                Some(expected_symbol.clone()),
                ticker.index_price,
                Some(ticker.timestamp_ms),
            )
        } else {
            evidence_complete = false;
            violations.insert(format!("direct index ticker for {currency} is absent"));
            continue;
        };

        let independent = native_equity * index_price;
        if !independent.is_finite() {
            evidence_complete = false;
            violations.insert(format!(
                "independent USD equity conversion for {currency} is not finite"
            ));
            continue;
        }
        let absolute_difference = (reported_equity_usd - independent).abs();
        let scale = reported_equity_usd.abs().max(independent.abs());
        let relative_difference = if scale == 0.0 {
            0.0
        } else {
            absolute_difference / scale
        };
        let effective_tolerance_usd =
            ACCOUNT_EQUITY_INDEX_ABS_TOLERANCE_USD.max(scale * ACCOUNT_EQUITY_INDEX_REL_TOLERANCE);
        if absolute_difference > effective_tolerance_usd {
            sample_valid = false;
            violations.insert(format!(
                "balance[{currency}].eqUsd differs from eq * index by {absolute_difference} USD; tolerance is {effective_tolerance_usd} USD"
            ));
        }
        reported_currency_equity_usd += reported_equity_usd;
        independently_converted_equity_usd += independent;
        conversion_samples.push(AccountEquityConversionSample {
            currency: currency.to_string(),
            native_equity,
            reported_equity_usd,
            index_symbol,
            index_price,
            index_timestamp_ms,
            independently_converted_equity_usd: independent,
            absolute_difference,
            relative_difference,
            effective_tolerance_usd,
            validated: sample_valid && absolute_difference <= effective_tolerance_usd,
        });
    }
    conversion_samples.sort_by(|left, right| left.currency.cmp(&right.currency));

    let mut aggregate_reported_difference_usd = None;
    let mut aggregate_independent_difference_usd = None;
    let mut aggregate_reported_tolerance_usd = None;
    let mut aggregate_independent_tolerance_usd = None;
    if let Some(total) = total_equity_usd {
        let reported_scale = total.abs().max(reported_currency_equity_usd.abs());
        let reported_tolerance = ACCOUNT_EQUITY_AGGREGATE_ABS_TOLERANCE_USD
            .max(reported_scale * ACCOUNT_EQUITY_AGGREGATE_REL_TOLERANCE);
        let reported_difference = (total - reported_currency_equity_usd).abs();
        aggregate_reported_difference_usd = Some(reported_difference);
        aggregate_reported_tolerance_usd = Some(reported_tolerance);
        if reported_difference > reported_tolerance {
            violations.insert(format!(
                "totalEq differs from the sum of eqUsd by {reported_difference} USD; tolerance is {reported_tolerance} USD"
            ));
        }

        let independent_scale = total.abs().max(independently_converted_equity_usd.abs());
        let independent_tolerance = ACCOUNT_EQUITY_INDEX_ABS_TOLERANCE_USD
            .max(independent_scale * ACCOUNT_EQUITY_INDEX_REL_TOLERANCE);
        let independent_difference = (total - independently_converted_equity_usd).abs();
        aggregate_independent_difference_usd = Some(independent_difference);
        aggregate_independent_tolerance_usd = Some(independent_tolerance);
        if independent_difference > independent_tolerance {
            violations.insert(format!(
                "totalEq differs from independently converted currency equity by {independent_difference} USD; tolerance is {independent_tolerance} USD"
            ));
        }
    }

    if conversion_samples.len() != balance.details.len() {
        evidence_complete = false;
    }
    let passed = evidence_complete
        && conversion_samples.iter().all(|sample| sample.validated)
        && violations.is_empty();
    Ok(AccountEquityEvaluation {
        total_equity_usd,
        reported_currency_equity_usd,
        independently_converted_equity_usd,
        aggregate_reported_difference_usd,
        aggregate_independent_difference_usd,
        aggregate_reported_tolerance_usd,
        aggregate_independent_tolerance_usd,
        currencies: balance.details.len() as u64,
        direct_index_tickers: index_tickers.len() as u64,
        conversion_samples,
        evidence_complete,
        passed,
        violations: violations.into_iter().collect(),
    })
}

fn clock_is_valid(
    start: &AccountCertificationClockEvidence,
    finish: &AccountCertificationClockEvidence,
    maximum_skew_ms: u64,
) -> bool {
    start.absolute_skew_ms == start.local_midpoint_ms.abs_diff(start.server_ms)
        && finish.absolute_skew_ms == finish.local_midpoint_ms.abs_diff(finish.server_ms)
        && start.absolute_skew_ms <= maximum_skew_ms
        && finish.absolute_skew_ms <= maximum_skew_ms
        && finish.local_midpoint_ms >= start.local_midpoint_ms
        && finish.server_ms >= start.server_ms
        && finish.server_ms.saturating_sub(start.server_ms).max(
            finish
                .local_midpoint_ms
                .saturating_sub(start.local_midpoint_ms),
        ) <= MAX_ACCOUNT_CERTIFICATION_SPAN_MS
}

#[doc(hidden)]
pub fn build_account_certification_response_evidence(
    expected_endpoint: &str,
    request_path: &str,
    body: String,
) -> Result<AccountCertificationResponseEvidence, AccountCertificationVerificationError> {
    if request_path != expected_endpoint {
        return invalid_evidence(format!(
            "collector requested {request_path:?}; expected {expected_endpoint:?}"
        ));
    }
    let bytes = body.len() as u64;
    if bytes > MAX_ACCOUNT_CERTIFICATION_RESPONSE_BYTES {
        return Err(AccountCertificationVerificationError::ResponseTooLarge {
            endpoint: expected_endpoint.to_string(),
            actual: bytes,
            limit: MAX_ACCOUNT_CERTIFICATION_RESPONSE_BYTES,
        });
    }
    Ok(AccountCertificationResponseEvidence {
        endpoint: expected_endpoint.to_string(),
        bytes,
        sha256: sha256_bytes(body.as_bytes()),
        body,
    })
}

fn validate_response_evidence(
    response: &AccountCertificationResponseEvidence,
    endpoint: &str,
) -> Result<(), AccountCertificationVerificationError> {
    if response.endpoint != endpoint {
        return invalid_evidence(format!(
            "response endpoint {:?} does not match {endpoint:?}",
            response.endpoint
        ));
    }
    if response.bytes > MAX_ACCOUNT_CERTIFICATION_RESPONSE_BYTES {
        return Err(AccountCertificationVerificationError::ResponseTooLarge {
            endpoint: endpoint.to_string(),
            actual: response.bytes,
            limit: MAX_ACCOUNT_CERTIFICATION_RESPONSE_BYTES,
        });
    }
    if response.bytes != response.body.len() as u64
        || response.sha256 != sha256_bytes(response.body.as_bytes())
    {
        return invalid_evidence(format!(
            "response {endpoint} byte count or SHA-256 does not match"
        ));
    }
    Ok(())
}

fn validate_artifact_header(
    artifact: &AccountCertificationArtifact,
) -> Result<(), AccountCertificationVerificationError> {
    if artifact.schema_version != ACCOUNT_CERTIFICATION_SCHEMA_VERSION {
        return invalid_evidence(format!(
            "schema version {} is unsupported; expected {ACCOUNT_CERTIFICATION_SCHEMA_VERSION}",
            artifact.schema_version
        ));
    }
    if artifact.java_reference_revision != PINNED_JAVA_REVISION {
        return invalid_evidence("pinned Java revision does not match this verifier");
    }
    if artifact.reap_version.trim().is_empty() {
        return invalid_evidence("collector Reap version is empty");
    }
    for (label, value) in [
        ("collector executable", artifact.executable_sha256.as_str()),
        (
            "collector host identity",
            artifact.host_identity_sha256.as_str(),
        ),
        ("config", artifact.config.sha256.as_str()),
        ("config fingerprint", artifact.config_fingerprint.as_str()),
        (
            "account identity",
            artifact.summary.account_identity_sha256.as_str(),
        ),
    ] {
        if !is_lower_sha256(value) {
            return invalid_evidence(format!("{label} SHA-256 is not lowercase hexadecimal"));
        }
    }
    validate_account_certification_account_id(&artifact.summary.account_id)?;
    if artifact.config.source_path.trim().is_empty() {
        return invalid_evidence("embedded config source path is empty");
    }
    if !artifact.summary.evidence_complete {
        return invalid_evidence("artifact is not marked evidence-complete");
    }
    Ok(())
}

#[doc(hidden)]
pub fn validate_account_certification_account_id(
    account_id: &str,
) -> Result<(), AccountCertificationVerificationError> {
    if account_id.is_empty() || account_id.trim() != account_id {
        return invalid_evidence(
            "account id must be non-empty and contain no surrounding whitespace",
        );
    }
    if account_id.len() > 128 {
        return invalid_evidence("account id exceeds 128 bytes");
    }
    Ok(())
}

/// Returns the stable, length-delimited identity of one configured OKX
/// account. This deliberately contains no credentials or host identity.
pub fn okx_account_identity_sha256(
    environment: TradingEnvironment,
    account_id: &str,
    user_id: &str,
    main_user_id: &str,
) -> String {
    let environment = match environment {
        TradingEnvironment::Demo => b"demo".as_slice(),
        TradingEnvironment::Production => b"production".as_slice(),
    };
    identity_sha256(
        b"reap-okx-account-v1",
        &[
            environment,
            account_id.as_bytes(),
            user_id.trim().as_bytes(),
            main_user_id.trim().as_bytes(),
        ],
    )
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn identity_sha256(domain: &[u8], fields: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    hasher.update((domain.len() as u64).to_le_bytes());
    hasher.update(domain);
    for field in fields {
        hasher.update((field.len() as u64).to_le_bytes());
        hasher.update(field);
    }
    format!("{:x}", hasher.finalize())
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn invalid_evidence<T>(
    message: impl Into<String>,
) -> Result<T, AccountCertificationVerificationError> {
    Err(AccountCertificationVerificationError::InvalidEvidence(
        message.into(),
    ))
}

#[cfg(test)]
mod tests {
    use reap_venue::okx::{OkxApiKeyPermission, OkxBalanceDetail, OkxIndexTickerSnapshot};

    use super::*;

    fn response(endpoint: &str, body: &str) -> AccountCertificationResponseEvidence {
        AccountCertificationResponseEvidence {
            endpoint: endpoint.to_string(),
            bytes: body.len() as u64,
            sha256: sha256_bytes(body.as_bytes()),
            body: body.to_string(),
        }
    }

    fn equity_balance() -> OkxAccountBalanceSnapshot {
        OkxAccountBalanceSnapshot {
            update_time_ms: 1_000,
            total_equity_usd: Some(1_000.0),
            adjusted_equity_usd: Some(1_000.0),
            borrow_frozen_usd: Some(0.0),
            notional_usd_for_borrow: Some(0.0),
            margin_ratio: None,
            notional_usd: Some(0.0),
            details: vec![OkxBalanceDetail {
                currency: "USDT".to_string(),
                update_time_ms: 1_000,
                cash_balance: Some(1_000.0),
                available_balance: Some(1_000.0),
                equity: Some(1_000.0),
                equity_usd: Some(1_000.0),
                discounted_equity_usd: Some(1_000.0),
                unrealized_pnl: Some(0.0),
                liability: Some(0.0),
                cross_liability: Some(0.0),
                isolated_liability: None,
                unrealized_loss_liability: None,
                accrued_interest: Some(0.0),
                borrow_frozen_usd: Some(0.0),
                max_loan: Some(100.0),
                forced_repayment_indicator: Some(0),
            }],
        }
    }

    fn fixed_artifact() -> AccountCertificationArtifact {
        AccountCertificationArtifact {
            schema_version: ACCOUNT_CERTIFICATION_SCHEMA_VERSION,
            java_reference_revision: PINNED_JAVA_REVISION.to_string(),
            reap_version: "0.1.0".to_string(),
            executable_sha256: "a".repeat(64),
            host_identity_sha256: "b".repeat(64),
            config: AccountCertificationConfigEvidence {
                source_path: "/secure/live.toml".to_string(),
                bytes: 11,
                sha256: "c".repeat(64),
                toml: "mode = \"x\"\n".to_string(),
            },
            config_fingerprint: "d".repeat(64),
            start_clock: AccountCertificationClockEvidence {
                local_midpoint_ms: 1_000,
                server_ms: 999,
                absolute_skew_ms: 1,
            },
            finish_clock: AccountCertificationClockEvidence {
                local_midpoint_ms: 1_100,
                server_ms: 1_101,
                absolute_skew_ms: 1,
            },
            account_config_before: response(ACCOUNT_CONFIG_ENDPOINT, "{\"before\":true}"),
            account_balance: response(ACCOUNT_BALANCE_ENDPOINT, "{\"balance\":1}"),
            index_tickers: vec![AccountCertificationIndexEvidence {
                currency: "USDT".to_string(),
                symbol: "USDT-USD".to_string(),
                response: response(
                    "/api/v5/market/index-tickers?instId=USDT-USD",
                    "{\"index\":1}",
                ),
            }],
            account_positions: response(ACCOUNT_POSITIONS_ENDPOINT, "{\"positions\":[]}"),
            account_config_after: response(ACCOUNT_CONFIG_ENDPOINT, "{\"after\":true}"),
            summary: AccountCertificationSummary {
                coverage: AccountCertificationCoverage::PointInTimeCashAndZeroLiability,
                environment: TradingEnvironment::Demo,
                account_id: "main".to_string(),
                account_identity_sha256: "e".repeat(64),
                account_identity_stable: true,
                account_settings_stable: true,
                clock_evidence_valid: true,
                api_key_policy: OkxApiKeyPolicyEvaluation {
                    api_key_label: "reap-demo".to_string(),
                    expected_permissions: BTreeSet::from([
                        OkxApiKeyPermission::ReadOnly,
                        OkxApiKeyPermission::Trade,
                    ]),
                    observed_permissions: BTreeSet::from([
                        OkxApiKeyPermission::ReadOnly,
                        OkxApiKeyPermission::Trade,
                    ]),
                    permissions_match: true,
                    ip_binding_required: true,
                    ip_binding_count: 1,
                    ip_binding_present: true,
                    evidence_complete: true,
                    passed: true,
                    violations: Vec::new(),
                },
                policy: AccountCashPolicyEvaluation {
                    policy_version: ACCOUNT_CASH_POLICY_VERSION,
                    account_mode_matches: true,
                    configured_spot_cash_only: true,
                    configured_borrow_limits_zero: true,
                    borrowing_disabled: true,
                    liability_evidence_complete: true,
                    liabilities_zero: true,
                    margin_positions_absent: true,
                    passed: true,
                    violations: Vec::new(),
                },
                equity: AccountEquityEvaluation {
                    total_equity_usd: Some(100.0),
                    reported_currency_equity_usd: 100.0,
                    independently_converted_equity_usd: 100.0,
                    aggregate_reported_difference_usd: Some(0.0),
                    aggregate_independent_difference_usd: Some(0.0),
                    aggregate_reported_tolerance_usd: Some(1e-8),
                    aggregate_independent_tolerance_usd: Some(0.1),
                    currencies: 1,
                    direct_index_tickers: 1,
                    conversion_samples: vec![AccountEquityConversionSample {
                        currency: "USDT".to_string(),
                        native_equity: 100.0,
                        reported_equity_usd: 100.0,
                        index_symbol: Some("USDT-USD".to_string()),
                        index_price: 1.0,
                        index_timestamp_ms: Some(1_050),
                        independently_converted_equity_usd: 100.0,
                        absolute_difference: 0.0,
                        relative_difference: 0.0,
                        effective_tolerance_usd: 0.1,
                        validated: true,
                    }],
                    evidence_complete: true,
                    passed: true,
                    violations: Vec::new(),
                },
                evidence_complete: true,
                passed: true,
                limitations: vec!["fixed serialization fixture".to_string()],
            },
        }
    }

    #[test]
    fn serialized_artifact_schema_and_hash_are_stable() {
        let artifact = fixed_artifact();
        let bytes = serde_json::to_vec(&artifact).unwrap();
        let round_trip: AccountCertificationArtifact = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(round_trip, artifact);
        assert_eq!(artifact.schema_version, 3);
        assert_eq!(
            sha256_bytes(&bytes),
            "7684a2f788c57f9072020a5538ed27eaf86aa4bb19e89b6450a78af61ceeecf0"
        );
    }

    #[test]
    fn account_identity_hash_is_stable_and_field_delimited() {
        let demo = okx_account_identity_sha256(TradingEnvironment::Demo, "main", "7", "6");
        assert_eq!(
            demo,
            "9658c99fd44c3caeac43717eefdb0ddcac497f4af352fa2e0c91b735d6108475"
        );
        assert_ne!(
            demo,
            okx_account_identity_sha256(TradingEnvironment::Production, "main", "7", "6")
        );
        assert_ne!(
            okx_account_identity_sha256(TradingEnvironment::Demo, "main", "ab", "c"),
            okx_account_identity_sha256(TradingEnvironment::Demo, "main", "a", "bc")
        );
    }

    #[test]
    fn equity_conversion_rejects_missing_stale_and_inconsistent_evidence() {
        let balance = equity_balance();
        let start = AccountCertificationClockEvidence {
            local_midpoint_ms: 20_000,
            server_ms: 20_000,
            absolute_skew_ms: 0,
        };
        let finish = AccountCertificationClockEvidence {
            local_midpoint_ms: 20_100,
            server_ms: 20_100,
            absolute_skew_ms: 0,
        };
        let indexes = BTreeMap::from([(
            "USDT".to_string(),
            OkxIndexTickerSnapshot {
                symbol: "USDT-USD".to_string(),
                index_price: 1.0,
                timestamp_ms: 20_050,
            },
        )]);
        let clean = evaluate_account_equity(&balance, &indexes, &start, &finish).unwrap();
        assert!(clean.passed, "{:?}", clean.violations);

        let missing = evaluate_account_equity(&balance, &BTreeMap::new(), &start, &finish).unwrap();
        assert!(!missing.evidence_complete);
        assert!(!missing.passed);

        let mut stale_indexes = indexes.clone();
        stale_indexes.get_mut("USDT").unwrap().timestamp_ms = 1;
        let stale = evaluate_account_equity(&balance, &stale_indexes, &start, &finish).unwrap();
        assert!(!stale.evidence_complete);
        assert!(
            stale
                .violations
                .iter()
                .any(|violation| violation.contains("timestamp"))
        );

        let mut inconsistent = balance.clone();
        inconsistent.details[0].equity_usd = Some(900.0);
        let inconsistent =
            evaluate_account_equity(&inconsistent, &indexes, &start, &finish).unwrap();
        assert!(!inconsistent.passed);
        assert!(
            inconsistent
                .violations
                .iter()
                .any(|violation| violation.contains("eqUsd differs"))
        );

        let mut aggregate = balance;
        aggregate.total_equity_usd = Some(900.0);
        let aggregate = evaluate_account_equity(&aggregate, &indexes, &start, &finish).unwrap();
        assert!(!aggregate.passed);
        assert!(
            aggregate
                .violations
                .iter()
                .any(|violation| violation.contains("sum of eqUsd"))
        );
    }

    #[test]
    fn byte_verifier_rejects_parse_and_size_failures_without_io() {
        let parse =
            verify_account_certification_artifact_bytes(b"{", "/tmp/artifact.json").unwrap_err();
        assert!(matches!(
            parse,
            AccountCertificationVerificationError::ParseArtifact { ref path, .. }
                if path == &PathBuf::from("/tmp/artifact.json")
        ));

        let oversized = vec![b' '; MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES as usize + 1];
        assert!(matches!(
            verify_account_certification_artifact_bytes(&oversized, "ignored").unwrap_err(),
            AccountCertificationVerificationError::ArtifactTooLarge {
                actual,
                limit: MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES,
            } if actual == MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES + 1
        ));
    }
}
