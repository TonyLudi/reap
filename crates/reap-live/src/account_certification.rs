use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reap_core::PINNED_JAVA_REVISION;
use reap_venue::okx::{
    HttpTransport, OkxAccountBalanceSnapshot, OkxAccountConfig, OkxAccountLevel,
    OkxAccountPositionsSnapshot, OkxIndexTickerSnapshot, OkxInstrumentType, OkxRestClient,
    OkxSigner, ReqwestTransport, RestError, parse_okx_account_balance_response_json,
    parse_okx_account_config_response_json, parse_okx_account_positions_response_json,
    parse_okx_index_ticker_response_json,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::provenance::{
    current_executable_sha256, host_identity_sha256, okx_account_identity_sha256, sha256_bytes,
};
use crate::{LiveConfig, LiveConfigError, OkxTradeModeConfig, TradingEnvironment};

pub const ACCOUNT_CASH_POLICY_VERSION: u32 = 1;
pub const ACCOUNT_CERTIFICATION_SCHEMA_VERSION: u32 = 2;
pub const MAX_ACCOUNT_CERTIFICATION_CONFIG_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_ACCOUNT_CERTIFICATION_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES: u64 = 48 * 1024 * 1024;
pub const MAX_ACCOUNT_CERTIFICATION_SPAN_MS: u64 = 30_000;
pub const MAX_ACCOUNT_CERTIFICATION_INDEX_STALENESS_MS: u64 = 10_000;
pub const MIN_ACCOUNT_CERTIFICATION_INDEX_INTERVAL_MS: u64 = 100;
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

#[derive(Debug, Error)]
pub enum AccountCertificationError {
    #[error("failed to reserve account-certification output {path}: {source}")]
    ReserveOutput {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid live config path {path}: {message}")]
    InvalidConfigPath { path: PathBuf, message: String },
    #[error("failed to read live config {path}: {source}")]
    ReadConfig {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("live config {path} is {actual} bytes; limit is {limit}")]
    ConfigTooLarge {
        path: PathBuf,
        actual: u64,
        limit: u64,
    },
    #[error("live configuration is invalid: {0}")]
    Config(#[from] LiveConfigError),
    #[error("configured account {0} does not exist")]
    UnknownAccount(String),
    #[error("failed to fingerprint account-certification provenance: {0}")]
    Provenance(String),
    #[error("failed to initialize OKX transport: {0}")]
    Transport(#[source] RestError),
    #[error("OKX account certification failed: {0}")]
    Rest(#[from] RestError),
    #[error("account-certification response for {endpoint} is {actual} bytes; limit is {limit}")]
    ResponseTooLarge {
        endpoint: String,
        actual: u64,
        limit: u64,
    },
    #[error("exchange clock evidence is invalid: {0}")]
    Clock(String),
    #[error("failed to serialize account-certification artifact: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("failed to parse account-certification artifact {path}: {source}")]
    ParseArtifact {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("account-certification artifact is {actual} bytes; limit is {limit}")]
    ArtifactTooLarge { actual: u64, limit: u64 },
    #[error("failed to write account-certification output {path}: {source}")]
    WriteOutput {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid account-certification artifact path {path}: {message}")]
    InvalidArtifactPath { path: PathBuf, message: String },
    #[error("failed to read account-certification artifact {path}: {source}")]
    ReadArtifact {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid account-certification evidence: {0}")]
    InvalidEvidence(String),
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

/// Collects authenticated read-only account evidence into one create-new,
/// owner-readable artifact. A failed collection leaves an empty reserved file,
/// which the verifier rejects.
pub async fn collect_account_certification_path(
    config_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    account_id: &str,
) -> Result<AccountCertificationSummary, AccountCertificationError> {
    validate_account_id(account_id)?;
    let output_path = output_path.as_ref();
    let mut output = reserve_output(output_path)?;
    let config_evidence = read_config(config_path.as_ref())?;
    let config = LiveConfig::from_toml(&config_evidence.toml)?;
    let account = config
        .account(account_id)
        .ok_or_else(|| AccountCertificationError::UnknownAccount(account_id.to_string()))?;
    let credentials = account.credentials_from_env()?;
    let executable_sha256 =
        current_executable_sha256().map_err(AccountCertificationError::Provenance)?;
    let host_identity_sha256 =
        host_identity_sha256().map_err(AccountCertificationError::Provenance)?;
    let transport = ReqwestTransport::with_timeouts(
        &config.venue.rest_url,
        Duration::from_millis(config.runtime.rest_connect_timeout_ms),
        Duration::from_millis(config.runtime.rest_request_timeout_ms),
    )
    .map_err(AccountCertificationError::Transport)?;
    let client = OkxRestClient::new(
        transport,
        OkxSigner::new(credentials, config.venue.environment.is_demo()),
    );
    let artifact = collect_account_certification_with_client(
        &client,
        &config,
        config_evidence,
        account_id,
        executable_sha256,
        host_identity_sha256,
    )
    .await?;
    let mut bytes = serde_json::to_vec_pretty(&artifact)?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES {
        return Err(AccountCertificationError::ArtifactTooLarge {
            actual: bytes.len() as u64,
            limit: MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES,
        });
    }
    output
        .write_all(&bytes)
        .and_then(|()| output.sync_all())
        .map_err(|source| AccountCertificationError::WriteOutput {
            path: output_path.to_path_buf(),
            source,
        })?;
    sync_parent(output_path)?;
    Ok(artifact.summary)
}

async fn collect_account_certification_with_client<T>(
    client: &OkxRestClient<T>,
    config: &LiveConfig,
    config_evidence: AccountCertificationConfigEvidence,
    account_id: &str,
    executable_sha256: String,
    host_identity_sha256: String,
) -> Result<AccountCertificationArtifact, AccountCertificationError>
where
    T: HttpTransport,
{
    let start_clock = sample_clock(client, config.runtime.max_exchange_clock_skew_ms).await?;
    let config_before = client.account_config_raw().await?;
    let balance = client.account_balance_raw().await?;
    let mut index_tickers = Vec::new();
    let mut parsed_index_tickers = BTreeMap::new();
    for (offset, (currency, symbol)) in required_index_symbols(&balance.snapshot)?
        .into_iter()
        .enumerate()
    {
        if offset > 0 {
            tokio::time::sleep(Duration::from_millis(
                MIN_ACCOUNT_CERTIFICATION_INDEX_INTERVAL_MS,
            ))
            .await;
        }
        let raw = client.index_ticker_raw(&symbol).await?;
        let endpoint = index_ticker_endpoint(&symbol);
        let response = response_evidence(&endpoint, &raw.request_path, raw.response_body)?;
        parsed_index_tickers.insert(currency.clone(), raw.ticker);
        index_tickers.push(AccountCertificationIndexEvidence {
            currency,
            symbol,
            response,
        });
    }
    let positions = client.account_positions_raw(None, None).await?;
    let config_after = client.account_config_raw().await?;
    let finish_clock = sample_clock(client, config.runtime.max_exchange_clock_skew_ms).await?;

    let account_config_before = response_evidence(
        ACCOUNT_CONFIG_ENDPOINT,
        &config_before.request_path,
        config_before.response_body,
    )?;
    let account_balance = response_evidence(
        ACCOUNT_BALANCE_ENDPOINT,
        &balance.request_path,
        balance.response_body,
    )?;
    let account_positions = response_evidence(
        ACCOUNT_POSITIONS_ENDPOINT,
        &positions.request_path,
        positions.response_body,
    )?;
    let account_config_after = response_evidence(
        ACCOUNT_CONFIG_ENDPOINT,
        &config_after.request_path,
        config_after.response_body,
    )?;
    let summary = derive_summary(
        config,
        account_id,
        &config_before.config,
        &balance.snapshot,
        &parsed_index_tickers,
        &positions.snapshot,
        &config_after.config,
        &start_clock,
        &finish_clock,
    )?;

    Ok(AccountCertificationArtifact {
        schema_version: ACCOUNT_CERTIFICATION_SCHEMA_VERSION,
        java_reference_revision: PINNED_JAVA_REVISION.to_string(),
        reap_version: env!("CARGO_PKG_VERSION").to_string(),
        executable_sha256,
        host_identity_sha256,
        config_fingerprint: config.fingerprint()?,
        config: config_evidence,
        start_clock,
        finish_clock,
        account_config_before,
        account_balance,
        index_tickers,
        account_positions,
        account_config_after,
        summary,
    })
}

/// Reopens and re-derives a certification artifact without credentials.
pub fn verify_account_certification_path(
    artifact_path: impl AsRef<Path>,
) -> Result<AccountCertificationSummary, AccountCertificationError> {
    Ok(verify_account_certification_artifact_path(artifact_path)?.summary)
}

/// Reopens and re-derives a certification artifact without credentials,
/// returning the exact artifact whose embedded evidence was validated.
pub fn verify_account_certification_artifact_path(
    artifact_path: impl AsRef<Path>,
) -> Result<AccountCertificationArtifact, AccountCertificationError> {
    let artifact_path = artifact_path.as_ref();
    let bytes = read_artifact(artifact_path)?;
    let artifact: AccountCertificationArtifact =
        serde_json::from_slice(&bytes).map_err(|source| {
            AccountCertificationError::ParseArtifact {
                path: artifact_path.to_path_buf(),
                source,
            }
        })?;
    validate_artifact_header(&artifact)?;

    let config_bytes = artifact.config.toml.as_bytes();
    if artifact.config.bytes != config_bytes.len() as u64
        || artifact.config.sha256 != sha256_bytes(config_bytes)
    {
        return invalid_evidence("embedded live config byte count or SHA-256 does not match");
    }
    if artifact.config.bytes > MAX_ACCOUNT_CERTIFICATION_CONFIG_BYTES {
        return Err(AccountCertificationError::ConfigTooLarge {
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
    let derived = derive_summary(
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

#[allow(clippy::too_many_arguments)]
fn derive_summary(
    config: &LiveConfig,
    account_id: &str,
    config_before: &OkxAccountConfig,
    balance: &OkxAccountBalanceSnapshot,
    index_tickers: &BTreeMap<String, OkxIndexTickerSnapshot>,
    positions: &OkxAccountPositionsSnapshot,
    config_after: &OkxAccountConfig,
    start_clock: &AccountCertificationClockEvidence,
    finish_clock: &AccountCertificationClockEvidence,
) -> Result<AccountCertificationSummary, AccountCertificationError> {
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
    let equity = evaluate_account_equity(balance, index_tickers, start_clock, finish_clock)?;
    let evidence_complete = equity.evidence_complete;
    let passed = account_identity_stable
        && account_settings_stable
        && clock_evidence_valid
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

fn required_index_symbols(
    balance: &OkxAccountBalanceSnapshot,
) -> Result<BTreeMap<String, String>, AccountCertificationError> {
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

fn currency_index_symbol(currency: &str) -> Result<Option<String>, AccountCertificationError> {
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

fn index_ticker_endpoint(symbol: &str) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("instId", symbol);
    format!("{MARKET_INDEX_TICKERS_ENDPOINT}?{}", query.finish())
}

fn verify_index_ticker_evidence(
    balance: &OkxAccountBalanceSnapshot,
    evidence: &[AccountCertificationIndexEvidence],
) -> Result<BTreeMap<String, OkxIndexTickerSnapshot>, AccountCertificationError> {
    let expected = required_index_symbols(balance)?;
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
        let endpoint = index_ticker_endpoint(expected_symbol);
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
) -> Result<AccountEquityEvaluation, AccountCertificationError> {
    let expected_indexes = required_index_symbols(balance)?;
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

async fn sample_clock<T>(
    client: &OkxRestClient<T>,
    maximum_skew_ms: u64,
) -> Result<AccountCertificationClockEvidence, AccountCertificationError>
where
    T: HttpTransport,
{
    let before = unix_time_ms()?;
    let server_ms = client.server_time_ms().await?;
    let after = unix_time_ms()?;
    let local_midpoint_ms = before.saturating_add(after.saturating_sub(before) / 2);
    let absolute_skew_ms = local_midpoint_ms.abs_diff(server_ms);
    if absolute_skew_ms > maximum_skew_ms {
        return Err(AccountCertificationError::Clock(format!(
            "absolute local/exchange skew {absolute_skew_ms} ms exceeds configured limit {maximum_skew_ms} ms"
        )));
    }
    Ok(AccountCertificationClockEvidence {
        local_midpoint_ms,
        server_ms,
        absolute_skew_ms,
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

fn response_evidence(
    expected_endpoint: &str,
    request_path: &str,
    body: String,
) -> Result<AccountCertificationResponseEvidence, AccountCertificationError> {
    if request_path != expected_endpoint {
        return invalid_evidence(format!(
            "collector requested {request_path:?}; expected {expected_endpoint:?}"
        ));
    }
    let bytes = body.len() as u64;
    if bytes > MAX_ACCOUNT_CERTIFICATION_RESPONSE_BYTES {
        return Err(AccountCertificationError::ResponseTooLarge {
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
) -> Result<(), AccountCertificationError> {
    if response.endpoint != endpoint {
        return invalid_evidence(format!(
            "response endpoint {:?} does not match {endpoint:?}",
            response.endpoint
        ));
    }
    if response.bytes > MAX_ACCOUNT_CERTIFICATION_RESPONSE_BYTES {
        return Err(AccountCertificationError::ResponseTooLarge {
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
) -> Result<(), AccountCertificationError> {
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
    validate_account_id(&artifact.summary.account_id)?;
    if artifact.config.source_path.trim().is_empty() {
        return invalid_evidence("embedded config source path is empty");
    }
    if !artifact.summary.evidence_complete {
        return invalid_evidence("artifact is not marked evidence-complete");
    }
    Ok(())
}

fn read_config(
    path: &Path,
) -> Result<AccountCertificationConfigEvidence, AccountCertificationError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        AccountCertificationError::InvalidConfigPath {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AccountCertificationError::InvalidConfigPath {
            path: path.to_path_buf(),
            message: "must be a regular file and not a symbolic link".to_string(),
        });
    }
    if metadata.len() > MAX_ACCOUNT_CERTIFICATION_CONFIG_BYTES {
        return Err(AccountCertificationError::ConfigTooLarge {
            path: path.to_path_buf(),
            actual: metadata.len(),
            limit: MAX_ACCOUNT_CERTIFICATION_CONFIG_BYTES,
        });
    }
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        AccountCertificationError::InvalidConfigPath {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    let bytes =
        std::fs::read(&canonical).map_err(|source| AccountCertificationError::ReadConfig {
            path: canonical.clone(),
            source,
        })?;
    if bytes.len() as u64 > MAX_ACCOUNT_CERTIFICATION_CONFIG_BYTES {
        return Err(AccountCertificationError::ConfigTooLarge {
            path: canonical,
            actual: bytes.len() as u64,
            limit: MAX_ACCOUNT_CERTIFICATION_CONFIG_BYTES,
        });
    }
    let toml =
        String::from_utf8(bytes).map_err(|error| AccountCertificationError::InvalidConfigPath {
            path: canonical.clone(),
            message: format!("config is not valid UTF-8: {error}"),
        })?;
    let source_path = canonical
        .to_str()
        .ok_or_else(|| AccountCertificationError::InvalidConfigPath {
            path: canonical.clone(),
            message: "canonical path is not valid UTF-8".to_string(),
        })?
        .to_string();
    Ok(AccountCertificationConfigEvidence {
        source_path,
        bytes: toml.len() as u64,
        sha256: sha256_bytes(toml.as_bytes()),
        toml,
    })
}

fn reserve_output(path: &Path) -> Result<File, AccountCertificationError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|source| AccountCertificationError::ReserveOutput {
            path: path.to_path_buf(),
            source,
        })
}

fn read_artifact(path: &Path) -> Result<Vec<u8>, AccountCertificationError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        AccountCertificationError::InvalidArtifactPath {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AccountCertificationError::InvalidArtifactPath {
            path: path.to_path_buf(),
            message: "must be a regular file and not a symbolic link".to_string(),
        });
    }
    if metadata.len() > MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES {
        return Err(AccountCertificationError::ArtifactTooLarge {
            actual: metadata.len(),
            limit: MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES,
        });
    }
    let bytes = std::fs::read(path).map_err(|source| AccountCertificationError::ReadArtifact {
        path: path.to_path_buf(),
        source,
    })?;
    if bytes.len() as u64 > MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES {
        return Err(AccountCertificationError::ArtifactTooLarge {
            actual: bytes.len() as u64,
            limit: MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES,
        });
    }
    Ok(bytes)
}

fn sync_parent(path: &Path) -> Result<(), AccountCertificationError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let directory =
        File::open(parent).map_err(|source| AccountCertificationError::WriteOutput {
            path: parent.to_path_buf(),
            source,
        })?;
    directory
        .sync_all()
        .map_err(|source| AccountCertificationError::WriteOutput {
            path: parent.to_path_buf(),
            source,
        })
}

fn validate_account_id(account_id: &str) -> Result<(), AccountCertificationError> {
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

fn unix_time_ms() -> Result<u64, AccountCertificationError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| AccountCertificationError::Clock(error.to_string()))?
        .as_millis();
    u64::try_from(millis).map_err(|error| AccountCertificationError::Clock(error.to_string()))
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn invalid_evidence<T>(message: impl Into<String>) -> Result<T, AccountCertificationError> {
    Err(AccountCertificationError::InvalidEvidence(message.into()))
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use reap_risk::RiskLimits;
    use reap_strategy::ChaosConfig;
    use reap_venue::okx::{
        HttpResponse, OkxBalanceDetail, OkxCredentials, OkxPositionMode, SignedRequest,
    };

    use crate::{
        AlertConfig, HostGuardConfig, LiveAccountConfig, LiveStorageConfig, OkxVenueConfig,
        OperatorConfig, RuntimeConfig,
    };

    use super::*;

    #[derive(Clone)]
    struct MockTransport {
        responses: Arc<Mutex<VecDeque<String>>>,
        requests: Arc<Mutex<Vec<SignedRequest>>>,
    }

    #[async_trait]
    impl HttpTransport for MockTransport {
        async fn execute(&self, request: SignedRequest) -> Result<HttpResponse, RestError> {
            self.requests.lock().unwrap().push(request);
            let body = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("missing mock response");
            Ok(HttpResponse { status: 200, body })
        }
    }

    fn live_config(level: OkxAccountLevel) -> LiveConfig {
        let mut strategy: ChaosConfig =
            toml::from_str(include_str!("../../../examples/iarb2-basic.toml")).unwrap();
        strategy.risk_groups[0].account_id = Some("main".to_string());
        let config = LiveConfig {
            strategy,
            risk: RiskLimits::default(),
            venue: OkxVenueConfig::default(),
            runtime: RuntimeConfig::default(),
            storage: LiveStorageConfig::default(),
            operator: OperatorConfig::default(),
            alerts: AlertConfig::default(),
            host_guard: HostGuardConfig::default(),
            accounts: vec![LiveAccountConfig {
                id: "main".to_string(),
                api_key_env: "KEY".to_string(),
                secret_key_env: "SECRET".to_string(),
                passphrase_env: "PASS".to_string(),
                expected_account_level: level,
                expected_position_mode: OkxPositionMode::NetMode,
                id_prefix: "reap".to_string(),
                node_id: 1,
                trade_modes: HashMap::from([
                    ("BTC-USDT".to_string(), OkxTradeModeConfig::Cash),
                    ("BTC-PERP".to_string(), OkxTradeModeConfig::Cross),
                ]),
            }],
        };
        assert!(config.validate().valid, "{:?}", config.validate().errors);
        config
    }

    fn account_config(level: OkxAccountLevel) -> OkxAccountConfig {
        OkxAccountConfig {
            account_level: level,
            position_mode: OkxPositionMode::NetMode,
            account_stp_mode: "cancel_maker".to_string(),
            user_id: "7".to_string(),
            main_user_id: "6".to_string(),
            enable_spot_borrow: Some(false),
            auto_loan: Some(false),
            spot_borrow_auto_repay: Some(false),
        }
    }

    fn balance(level: OkxAccountLevel) -> OkxAccountBalanceSnapshot {
        let borrowing_fields_apply = level != OkxAccountLevel::SingleCurrencyMargin;
        let multi = matches!(
            level,
            OkxAccountLevel::MultiCurrencyMargin | OkxAccountLevel::PortfolioMargin
        );
        OkxAccountBalanceSnapshot {
            update_time_ms: 1_000,
            total_equity_usd: Some(1_000.0),
            adjusted_equity_usd: Some(1_000.0),
            borrow_frozen_usd: borrowing_fields_apply.then_some(0.0),
            notional_usd_for_borrow: borrowing_fields_apply.then_some(0.0),
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
                liability: borrowing_fields_apply.then_some(0.0),
                cross_liability: borrowing_fields_apply.then_some(0.0),
                isolated_liability: multi.then_some(0.0),
                unrealized_loss_liability: multi.then_some(0.0),
                accrued_interest: borrowing_fields_apply.then_some(0.0),
                borrow_frozen_usd: borrowing_fields_apply.then_some(0.0),
                max_loan: borrowing_fields_apply.then_some(100.0),
                forced_repayment_indicator: Some(0),
            }],
        }
    }

    fn positions() -> OkxAccountPositionsSnapshot {
        OkxAccountPositionsSnapshot {
            update_time_ms: 0,
            positions: Vec::new(),
        }
    }

    fn clock(server_ms: u64) -> AccountCertificationClockEvidence {
        AccountCertificationClockEvidence {
            local_midpoint_ms: server_ms,
            server_ms,
            absolute_skew_ms: 0,
        }
    }

    fn refresh_response_hash(response: &mut AccountCertificationResponseEvidence) {
        response.bytes = response.body.len() as u64;
        response.sha256 = sha256_bytes(response.body.as_bytes());
    }

    #[test]
    fn cash_policy_passes_complete_zero_state_and_rejects_missing_or_nonzero_evidence() {
        let config = live_config(OkxAccountLevel::Simple);
        let account = account_config(OkxAccountLevel::Simple);
        let clean = evaluate_account_cash_policy(
            &config,
            "main",
            &account,
            &balance(OkxAccountLevel::Simple),
            &positions(),
        );
        assert!(clean.passed, "{:?}", clean.violations);

        let mut missing = balance(OkxAccountLevel::Simple);
        missing.details[0].liability = None;
        let missing =
            evaluate_account_cash_policy(&config, "main", &account, &missing, &positions());
        assert!(!missing.liability_evidence_complete);
        assert!(
            missing
                .violations
                .iter()
                .any(|value| value.contains("liab is absent"))
        );

        let mut nonzero = balance(OkxAccountLevel::Simple);
        nonzero.details[0].accrued_interest = Some(0.01);
        let mut borrowing = account;
        borrowing.enable_spot_borrow = Some(true);
        let nonzero =
            evaluate_account_cash_policy(&config, "main", &borrowing, &nonzero, &positions());
        assert!(!nonzero.borrowing_disabled);
        assert!(!nonzero.liabilities_zero);
        assert!(!nonzero.passed);
    }

    #[test]
    fn futures_mode_treats_documented_inapplicable_balance_fields_as_complete() {
        let config = live_config(OkxAccountLevel::SingleCurrencyMargin);
        let evaluation = evaluate_account_cash_policy(
            &config,
            "main",
            &account_config(OkxAccountLevel::SingleCurrencyMargin),
            &balance(OkxAccountLevel::SingleCurrencyMargin),
            &positions(),
        );
        assert!(evaluation.passed, "{:?}", evaluation.violations);
    }

    #[test]
    fn equity_conversion_rejects_missing_stale_and_inconsistent_evidence() {
        let balance = balance(OkxAccountLevel::Simple);
        let start = clock(20_000);
        let finish = clock(20_100);
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

    #[cfg(unix)]
    #[test]
    fn certification_output_is_create_new_and_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!(
            "reap-account-certification-mode-{}-{}.json",
            std::process::id(),
            unix_time_ms().unwrap()
        ));
        let file = reserve_output(&path).unwrap();
        drop(file);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(matches!(
            reserve_output(&path),
            Err(AccountCertificationError::ReserveOutput { .. })
        ));
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn raw_artifact_round_trips_and_tampering_is_rejected() {
        let config = live_config(OkxAccountLevel::Simple);
        let config_toml = toml::to_string(&config).unwrap();
        let now = unix_time_ms().unwrap();
        let account_json = r#"{"code":"0","msg":"","data":[{"acctLv":"1","posMode":"net_mode","acctStpMode":"cancel_maker","uid":"7","mainUid":"6","enableSpotBorrow":false,"autoLoan":false,"spotBorrowAutoRepay":false}]}"#;
        let balance_json = r#"{"code":"0","msg":"","data":[{"uTime":"1000","totalEq":"1000","adjEq":"1000","borrowFroz":"0","notionalUsdForBorrow":"0","notionalUsd":"0","details":[{"ccy":"USDT","uTime":"1000","cashBal":"1000","availBal":"1000","eq":"1000","eqUsd":"1000","disEq":"1000","upl":"0","liab":"0","crossLiab":"0","interest":"0","borrowFroz":"0","maxLoan":"100","twap":"0"}]}]}"#;
        let index_json = format!(
            r#"{{"code":"0","msg":"","data":[{{"instId":"USDT-USD","idxPx":"1","ts":"{now}"}}]}}"#
        );
        let positions_json = r#"{"code":"0","msg":"","data":[]}"#;
        let responses = VecDeque::from([
            format!(r#"{{"code":"0","msg":"","data":[{{"ts":"{now}"}}]}}"#),
            account_json.to_string(),
            balance_json.to_string(),
            index_json,
            positions_json.to_string(),
            account_json.to_string(),
            format!(r#"{{"code":"0","msg":"","data":[{{"ts":"{}"}}]}}"#, now + 1),
        ]);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let client = OkxRestClient::new(
            MockTransport {
                responses: Arc::new(Mutex::new(responses)),
                requests: Arc::clone(&requests),
            },
            OkxSigner::new(OkxCredentials::new("key", "secret", "pass"), true),
        );
        let artifact = collect_account_certification_with_client(
            &client,
            &config,
            AccountCertificationConfigEvidence {
                source_path: "/tmp/live.toml".to_string(),
                bytes: config_toml.len() as u64,
                sha256: sha256_bytes(config_toml.as_bytes()),
                toml: config_toml,
            },
            "main",
            "a".repeat(64),
            "b".repeat(64),
        )
        .await
        .unwrap();
        assert!(artifact.summary.passed, "{:?}", artifact.summary);
        let paths = requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request.path.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![
                "/api/v5/public/time",
                ACCOUNT_CONFIG_ENDPOINT,
                ACCOUNT_BALANCE_ENDPOINT,
                "/api/v5/market/index-tickers?instId=USDT-USD",
                ACCOUNT_POSITIONS_ENDPOINT,
                ACCOUNT_CONFIG_ENDPOINT,
                "/api/v5/public/time",
            ]
        );

        let path = std::env::temp_dir().join(format!(
            "reap-account-certification-{}-{}.json",
            std::process::id(),
            now
        ));
        std::fs::write(&path, serde_json::to_vec(&artifact).unwrap()).unwrap();
        let verified = verify_account_certification_path(&path).unwrap();
        assert_eq!(verified, artifact.summary);
        assert_eq!(
            verify_account_certification_artifact_path(&path).unwrap(),
            artifact
        );

        let mut missing_index = artifact.clone();
        missing_index.index_tickers.clear();
        std::fs::write(&path, serde_json::to_vec(&missing_index).unwrap()).unwrap();
        let error = verify_account_certification_path(&path).unwrap_err();
        assert!(error.to_string().contains("incomplete"));

        let mut rehashed_balance_tamper = artifact.clone();
        rehashed_balance_tamper.account_balance.body = rehashed_balance_tamper
            .account_balance
            .body
            .replace(r#""eqUsd":"1000""#, r#""eqUsd":"900""#);
        refresh_response_hash(&mut rehashed_balance_tamper.account_balance);
        std::fs::write(&path, serde_json::to_vec(&rehashed_balance_tamper).unwrap()).unwrap();
        let error = verify_account_certification_path(&path).unwrap_err();
        assert!(error.to_string().contains("summary does not match"));

        let mut rehashed_stale_index = artifact.clone();
        rehashed_stale_index.index_tickers[0].response.body = rehashed_stale_index.index_tickers[0]
            .response
            .body
            .replace(&format!(r#""ts":"{now}""#), r#""ts":"1""#);
        refresh_response_hash(&mut rehashed_stale_index.index_tickers[0].response);
        std::fs::write(&path, serde_json::to_vec(&rehashed_stale_index).unwrap()).unwrap();
        let error = verify_account_certification_path(&path).unwrap_err();
        assert!(error.to_string().contains("summary does not match"));

        let mut tampered = artifact;
        tampered.account_balance.body = tampered.account_balance.body.replace("1000", "1001");
        std::fs::write(&path, serde_json::to_vec(&tampered).unwrap()).unwrap();
        let error = verify_account_certification_path(&path).unwrap_err();
        assert!(error.to_string().contains("SHA-256"));
        std::fs::remove_file(path).unwrap();
    }
}
