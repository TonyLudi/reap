use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reap_core::PINNED_JAVA_REVISION;
use reap_venue::okx::{
    HttpTransport, OkxAccountBalanceSnapshot, OkxAccountConfig, OkxAccountLevel,
    OkxAccountPositionsSnapshot, OkxInstrumentType, OkxRestClient, OkxSigner, ReqwestTransport,
    RestError, parse_okx_account_balance_response_json, parse_okx_account_config_response_json,
    parse_okx_account_positions_response_json,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::provenance::{
    current_executable_sha256, host_identity_sha256, okx_account_identity_sha256, sha256_bytes,
};
use crate::{LiveConfig, LiveConfigError, OkxTradeModeConfig, TradingEnvironment};

pub const ACCOUNT_CASH_POLICY_VERSION: u32 = 1;
pub const ACCOUNT_CERTIFICATION_SCHEMA_VERSION: u32 = 1;
pub const MAX_ACCOUNT_CERTIFICATION_CONFIG_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_ACCOUNT_CERTIFICATION_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES: u64 = 48 * 1024 * 1024;
pub const MAX_ACCOUNT_CERTIFICATION_SPAN_MS: u64 = 30_000;

const ACCOUNT_CONFIG_ENDPOINT: &str = "/api/v5/account/config";
const ACCOUNT_BALANCE_ENDPOINT: &str = "/api/v5/account/balance";
const ACCOUNT_POSITIONS_ENDPOINT: &str = "/api/v5/account/positions";

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
pub struct AccountCertificationSummary {
    pub coverage: AccountCertificationCoverage,
    pub environment: TradingEnvironment,
    pub account_id: String,
    pub account_identity_sha256: String,
    pub account_identity_stable: bool,
    pub account_settings_stable: bool,
    pub clock_evidence_valid: bool,
    pub policy: AccountCashPolicyEvaluation,
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
        &positions.snapshot,
        &config_after.config,
        &start_clock,
        &finish_clock,
    );

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
    let positions =
        parse_okx_account_positions_response_json(artifact.account_positions.body.as_bytes())?;
    let config_after =
        parse_okx_account_config_response_json(artifact.account_config_after.body.as_bytes())?;
    let derived = derive_summary(
        &config,
        &artifact.summary.account_id,
        &config_before,
        &balance,
        &positions,
        &config_after,
        &artifact.start_clock,
        &artifact.finish_clock,
    );
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
    positions: &OkxAccountPositionsSnapshot,
    config_after: &OkxAccountConfig,
    start_clock: &AccountCertificationClockEvidence,
    finish_clock: &AccountCertificationClockEvidence,
) -> AccountCertificationSummary {
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
    let evidence_complete = true;
    let passed = account_identity_stable
        && account_settings_stable
        && clock_evidence_valid
        && policy.passed
        && evidence_complete;
    AccountCertificationSummary {
        coverage: AccountCertificationCoverage::PointInTimeCashAndZeroLiability,
        environment: config.venue.environment,
        account_id: account_id.to_string(),
        account_identity_sha256,
        account_identity_stable,
        account_settings_stable,
        clock_evidence_valid,
        policy,
        evidence_complete,
        passed,
        limitations: vec![
            "point-in-time evidence only; it does not prove historical absence of borrowing"
                .to_string(),
            "authenticated GETs are sequential, not an atomic exchange snapshot; quiesce account activity during collection"
                .to_string(),
            "collector host and executable hashes are provenance identifiers, not independently authenticated by offline verification"
                .to_string(),
            "it does not reconcile deposits, withdrawals, funding, PnL, taxes, or complete exchange statements"
                .to_string(),
        ],
    }
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
        let balance_json = r#"{"code":"0","msg":"","data":[{"uTime":"1000","totalEq":"1000","adjEq":"1000","borrowFroz":"0","notionalUsdForBorrow":"0","notionalUsd":"0","details":[{"ccy":"USDT","uTime":"1000","cashBal":"1000","availBal":"1000","eq":"1000","liab":"0","crossLiab":"0","interest":"0","borrowFroz":"0","maxLoan":"100","twap":"0"}]}]}"#;
        let positions_json = r#"{"code":"0","msg":"","data":[]}"#;
        let responses = VecDeque::from([
            format!(r#"{{"code":"0","msg":"","data":[{{"ts":"{now}"}}]}}"#),
            account_json.to_string(),
            balance_json.to_string(),
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

        let mut tampered = artifact;
        tampered.account_balance.body = tampered.account_balance.body.replace("1000", "1001");
        std::fs::write(&path, serde_json::to_vec(&tampered).unwrap()).unwrap();
        let error = verify_account_certification_path(&path).unwrap_err();
        assert!(error.to_string().contains("SHA-256"));
        std::fs::remove_file(path).unwrap();
    }
}
