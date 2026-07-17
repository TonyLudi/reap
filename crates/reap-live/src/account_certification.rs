use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reap_core::PINNED_JAVA_REVISION;
use reap_evidence_core::{
    EvidenceClientFactory, EvidenceClientFactoryError, EvidenceCredentialEnvironment,
    EvidenceHttpConfig, EvidenceReadError, EvidenceReadOnly,
};
use reap_live_contracts::{
    ACCOUNT_CERTIFICATION_SCHEMA_VERSION, AccountCertificationArtifact,
    AccountCertificationIndexEvidence, AccountCertificationSummary,
    AccountCertificationVerificationError, MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES,
    MAX_ACCOUNT_CERTIFICATION_CONFIG_BYTES, account_certification_index_ticker_endpoint,
    account_certification_required_index_symbols, build_account_certification_response_evidence,
    derive_account_certification_summary, validate_account_certification_account_id,
    verify_account_certification_artifact_bytes,
};
pub(crate) use reap_live_contracts::{
    AccountCertificationClockEvidence, AccountCertificationConfigEvidence,
};
#[cfg(test)]
use reap_live_contracts::{AccountCertificationResponseEvidence, evaluate_account_cash_policy};
use reap_venue::okx::{
    RestError, parse_okx_account_balance_response_json, parse_okx_account_config_response_json,
    parse_okx_account_positions_response_json, parse_okx_index_ticker_response_json,
};
use thiserror::Error;

#[cfg(test)]
use reap_evidence_core::{
    AccountBalanceResponse, AccountBillsPageResponse, AccountConfigResponse,
    AccountPositionsResponse, IndexTickerResponse, RecentFillsPageResponse,
    RegularOpenOrdersResponse, RegularOrderDetailsResponse,
};
#[cfg(test)]
use reap_venue::okx::parse_okx_server_time_response_json;

use crate::provenance::{current_executable_sha256, host_identity_sha256, sha256_bytes};
use crate::{LiveConfig, LiveConfigError};

pub const MIN_ACCOUNT_CERTIFICATION_INDEX_INTERVAL_MS: u64 = 100;

const ACCOUNT_CONFIG_ENDPOINT: &str = "/api/v5/account/config";
const ACCOUNT_BALANCE_ENDPOINT: &str = "/api/v5/account/balance";
const ACCOUNT_POSITIONS_ENDPOINT: &str = "/api/v5/account/positions";

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
    #[error("OKX account certification failed: {0}")]
    Evidence(#[from] EvidenceReadError),
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

impl From<AccountCertificationVerificationError> for AccountCertificationError {
    fn from(error: AccountCertificationVerificationError) -> Self {
        match error {
            AccountCertificationVerificationError::ConfigTooLarge {
                path,
                actual,
                limit,
            } => Self::ConfigTooLarge {
                path,
                actual,
                limit,
            },
            AccountCertificationVerificationError::Config(error) => Self::Config(error),
            AccountCertificationVerificationError::Rest(error) => Self::Rest(error),
            AccountCertificationVerificationError::ResponseTooLarge {
                endpoint,
                actual,
                limit,
            } => Self::ResponseTooLarge {
                endpoint,
                actual,
                limit,
            },
            AccountCertificationVerificationError::ParseArtifact { path, source } => {
                Self::ParseArtifact { path, source }
            }
            AccountCertificationVerificationError::ArtifactTooLarge { actual, limit } => {
                Self::ArtifactTooLarge { actual, limit }
            }
            AccountCertificationVerificationError::InvalidEvidence(message) => {
                Self::InvalidEvidence(message)
            }
        }
    }
}

fn evidence_credential_environment(
    account: &crate::LiveAccountConfig,
) -> EvidenceCredentialEnvironment {
    EvidenceCredentialEnvironment::new(
        &account.id,
        &account.api_key_env,
        &account.secret_key_env,
        &account.passphrase_env,
    )
}

fn map_factory_error(error: EvidenceClientFactoryError) -> AccountCertificationError {
    match error {
        EvidenceClientFactoryError::MissingCredential { account_id, name } => {
            AccountCertificationError::Config(LiveConfigError::MissingCredential {
                account_id,
                name,
            })
        }
        EvidenceClientFactoryError::InvalidConfiguration(message) => {
            AccountCertificationError::Transport(RestError::Transport(format!(
                "invalid evidence client configuration: {message}"
            )))
        }
        EvidenceClientFactoryError::Transport(message) => {
            AccountCertificationError::Transport(RestError::Transport(message))
        }
    }
}

/// Collects authenticated read-only account evidence into one create-new,
/// owner-readable artifact. A failed collection leaves an empty reserved file,
/// which the verifier rejects.
pub async fn collect_account_certification_path<F>(
    config_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    account_id: &str,
    factory: &F,
) -> Result<AccountCertificationSummary, AccountCertificationError>
where
    F: EvidenceClientFactory,
    F::Client: EvidenceReadOnly<Error = EvidenceReadError>,
{
    validate_account_certification_account_id(account_id)?;
    let output_path = output_path.as_ref();
    let mut output = reserve_output(output_path)?;
    let config_evidence = read_config(config_path.as_ref())?;
    let config = LiveConfig::from_toml(&config_evidence.toml)?;
    let account = config
        .account(account_id)
        .ok_or_else(|| AccountCertificationError::UnknownAccount(account_id.to_string()))?;
    let prepared = factory
        .prepare_credentials(&evidence_credential_environment(account))
        .map_err(map_factory_error)?;
    let executable_sha256 =
        current_executable_sha256().map_err(AccountCertificationError::Provenance)?;
    let host_identity_sha256 =
        host_identity_sha256().map_err(AccountCertificationError::Provenance)?;
    let client = factory
        .connect(
            prepared,
            &EvidenceHttpConfig::new(
                &config.venue.rest_url,
                config.venue.environment.is_demo(),
                Duration::from_millis(config.runtime.rest_connect_timeout_ms),
                Duration::from_millis(config.runtime.rest_request_timeout_ms),
            ),
        )
        .map_err(map_factory_error)?;
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

async fn collect_account_certification_with_client<C>(
    client: &C,
    config: &LiveConfig,
    config_evidence: AccountCertificationConfigEvidence,
    account_id: &str,
    executable_sha256: String,
    host_identity_sha256: String,
) -> Result<AccountCertificationArtifact, AccountCertificationError>
where
    C: EvidenceReadOnly<Error = EvidenceReadError> + ?Sized,
{
    let start_clock = sample_clock(client, config.runtime.max_exchange_clock_skew_ms).await?;
    let config_before_response = client.account_config().await?;
    let config_before =
        parse_okx_account_config_response_json(config_before_response.response_body().as_bytes())?;
    let balance_response = client.account_balance().await?;
    let balance =
        parse_okx_account_balance_response_json(balance_response.response_body().as_bytes())?;
    let mut index_tickers = Vec::new();
    let mut parsed_index_tickers = BTreeMap::new();
    for (offset, (currency, symbol)) in account_certification_required_index_symbols(&balance)?
        .into_iter()
        .enumerate()
    {
        if offset > 0 {
            tokio::time::sleep(Duration::from_millis(
                MIN_ACCOUNT_CERTIFICATION_INDEX_INTERVAL_MS,
            ))
            .await;
        }
        let raw = client.index_ticker(&symbol).await?;
        let ticker = parse_okx_index_ticker_response_json(raw.response_body().as_bytes())?;
        if ticker.symbol != symbol {
            return Err(RestError::InvalidField {
                field: "instId",
                value: ticker.symbol,
                message: format!("does not match requested {symbol}"),
            }
            .into());
        }
        let endpoint = account_certification_index_ticker_endpoint(&symbol);
        let (request_path, response_body) = raw.into_parts();
        let response =
            build_account_certification_response_evidence(&endpoint, &request_path, response_body)?;
        parsed_index_tickers.insert(currency.clone(), ticker);
        index_tickers.push(AccountCertificationIndexEvidence {
            currency,
            symbol,
            response,
        });
    }
    let positions_response = client.account_positions().await?;
    let positions =
        parse_okx_account_positions_response_json(positions_response.response_body().as_bytes())?;
    let config_after_response = client.account_config().await?;
    let config_after =
        parse_okx_account_config_response_json(config_after_response.response_body().as_bytes())?;
    let finish_clock = sample_clock(client, config.runtime.max_exchange_clock_skew_ms).await?;

    let (config_before_path, config_before_body) = config_before_response.into_parts();
    let account_config_before = build_account_certification_response_evidence(
        ACCOUNT_CONFIG_ENDPOINT,
        &config_before_path,
        config_before_body,
    )?;
    let (balance_path, balance_body) = balance_response.into_parts();
    let account_balance = build_account_certification_response_evidence(
        ACCOUNT_BALANCE_ENDPOINT,
        &balance_path,
        balance_body,
    )?;
    let (positions_path, positions_body) = positions_response.into_parts();
    let account_positions = build_account_certification_response_evidence(
        ACCOUNT_POSITIONS_ENDPOINT,
        &positions_path,
        positions_body,
    )?;
    let (config_after_path, config_after_body) = config_after_response.into_parts();
    let account_config_after = build_account_certification_response_evidence(
        ACCOUNT_CONFIG_ENDPOINT,
        &config_after_path,
        config_after_body,
    )?;
    let summary = derive_account_certification_summary(
        config,
        account_id,
        &config_before,
        &balance,
        &parsed_index_tickers,
        &positions,
        &config_after,
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
    verify_account_certification_artifact_bytes(&bytes, artifact_path).map_err(Into::into)
}

async fn sample_clock<C>(
    client: &C,
    maximum_skew_ms: u64,
) -> Result<AccountCertificationClockEvidence, AccountCertificationError>
where
    C: EvidenceReadOnly<Error = EvidenceReadError> + ?Sized,
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

fn unix_time_ms() -> Result<u64, AccountCertificationError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| AccountCertificationError::Clock(error.to_string()))?
        .as_millis();
    u64::try_from(millis).map_err(|error| AccountCertificationError::Clock(error.to_string()))
}

#[cfg(test)]
#[derive(Clone)]
pub(crate) struct NarrowEvidenceFake {
    responses: std::sync::Arc<
        std::sync::Mutex<std::collections::VecDeque<Result<String, EvidenceReadError>>>,
    >,
    requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

#[cfg(test)]
impl NarrowEvidenceFake {
    pub(crate) fn new(
        responses: std::collections::VecDeque<Result<String, EvidenceReadError>>,
        requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    ) -> Self {
        Self {
            responses: std::sync::Arc::new(std::sync::Mutex::new(responses)),
            requests,
        }
    }

    fn response(&self, path: String) -> Result<String, EvidenceReadError> {
        self.requests.lock().unwrap().push(path);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("missing mock response")
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl EvidenceReadOnly for NarrowEvidenceFake {
    type Error = EvidenceReadError;

    async fn server_time_ms(&self) -> Result<u64, Self::Error> {
        let body = self.response("/api/v5/public/time".to_string())?;
        parse_okx_server_time_response_json(body.as_bytes())
            .map_err(|error| EvidenceReadError::Other(error.to_string()))
    }

    async fn account_config(&self) -> Result<AccountConfigResponse, Self::Error> {
        let path = ACCOUNT_CONFIG_ENDPOINT.to_string();
        Ok(AccountConfigResponse::new(
            &path,
            self.response(path.clone())?,
        ))
    }

    async fn account_balance(&self) -> Result<AccountBalanceResponse, Self::Error> {
        let path = ACCOUNT_BALANCE_ENDPOINT.to_string();
        Ok(AccountBalanceResponse::new(
            &path,
            self.response(path.clone())?,
        ))
    }

    async fn account_positions(&self) -> Result<AccountPositionsResponse, Self::Error> {
        let path = ACCOUNT_POSITIONS_ENDPOINT.to_string();
        Ok(AccountPositionsResponse::new(
            &path,
            self.response(path.clone())?,
        ))
    }

    async fn index_ticker(&self, symbol: &str) -> Result<IndexTickerResponse, Self::Error> {
        let path = account_certification_index_ticker_endpoint(symbol);
        Ok(IndexTickerResponse::new(
            &path,
            self.response(path.clone())?,
        ))
    }

    async fn recent_fills_page(
        &self,
        after: Option<&str>,
    ) -> Result<RecentFillsPageResponse, Self::Error> {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        if let Some(after) = after {
            query.append_pair("after", after);
        }
        query.append_pair("limit", "100");
        let path = format!("/api/v5/trade/fills?{}", query.finish());
        Ok(RecentFillsPageResponse::new(
            &path,
            self.response(path.clone())?,
        ))
    }

    async fn account_bills_page(
        &self,
        begin_ms: u64,
        end_ms: u64,
        after: Option<&str>,
    ) -> Result<AccountBillsPageResponse, Self::Error> {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query.append_pair("begin", &begin_ms.to_string());
        query.append_pair("end", &end_ms.to_string());
        if let Some(after) = after {
            query.append_pair("after", after);
        }
        query.append_pair("limit", "100");
        let path = format!("/api/v5/account/bills?{}", query.finish());
        Ok(AccountBillsPageResponse::new(
            &path,
            self.response(path.clone())?,
        ))
    }

    async fn regular_order_details(
        &self,
        symbol: &str,
        exchange_order_id: &str,
        client_order_id: &str,
    ) -> Result<RegularOrderDetailsResponse, Self::Error> {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query.append_pair("instId", symbol);
        query.append_pair("ordId", exchange_order_id);
        query.append_pair("clOrdId", client_order_id);
        let path = format!("/api/v5/trade/order?{}", query.finish());
        Ok(RegularOrderDetailsResponse::new(
            &path,
            self.response(path.clone())?,
        ))
    }

    async fn regular_open_orders(&self) -> Result<RegularOpenOrdersResponse, Self::Error> {
        let path = "/api/v5/trade/orders-pending".to_string();
        Ok(RegularOpenOrdersResponse::new(
            &path,
            self.response(path.clone())?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap, VecDeque};
    use std::sync::{Arc, Mutex};

    use reap_live_contracts::OkxTradeModeConfig;
    use reap_risk::RiskLimits;
    use reap_strategy::ChaosConfig;
    use reap_venue::okx::{
        OkxAccountBalanceSnapshot, OkxAccountConfig, OkxAccountLevel, OkxAccountPositionsSnapshot,
        OkxApiKeyPermission, OkxBalanceDetail, OkxPositionMode,
    };

    use crate::{
        AlertConfig, HostGuardConfig, LiveAccountConfig, LiveStorageConfig, OkxVenueConfig,
        OperatorConfig, RuntimeConfig,
    };

    use super::*;

    fn live_config(level: OkxAccountLevel) -> LiveConfig {
        let mut strategy: ChaosConfig =
            toml::from_str(include_str!("../../../examples/iarb2-basic.toml")).unwrap();
        strategy.reference_data_stale_threshold_ms = Some(120_000);
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
                api_key_policy: crate::OkxApiKeyPolicyConfig::default(),
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
            api_key_label: "reap-demo".to_string(),
            api_key_permissions: BTreeSet::from([
                OkxApiKeyPermission::ReadOnly,
                OkxApiKeyPermission::Trade,
            ]),
            api_key_ip_bindings: BTreeSet::from(["203.0.113.5".to_string()]),
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

    fn refresh_response_hash(response: &mut AccountCertificationResponseEvidence) {
        response.bytes = response.body.len() as u64;
        response.sha256 = sha256_bytes(response.body.as_bytes());
    }

    #[test]
    fn verification_errors_map_to_existing_live_variants_and_messages() {
        let contract = AccountCertificationVerificationError::ConfigTooLarge {
            path: PathBuf::from("/tmp/live.toml"),
            actual: 9,
            limit: 8,
        };
        let expected = contract.to_string();
        let live = AccountCertificationError::from(contract);
        assert_eq!(live.to_string(), expected);
        assert!(matches!(
            live,
            AccountCertificationError::ConfigTooLarge {
                ref path,
                actual: 9,
                limit: 8,
            } if path == &PathBuf::from("/tmp/live.toml")
        ));

        let contract = AccountCertificationVerificationError::Config(LiveConfigError::Invalid(
            "bad account mapping".to_string(),
        ));
        let expected = contract.to_string();
        let live = AccountCertificationError::from(contract);
        assert_eq!(live.to_string(), expected);
        assert!(matches!(live, AccountCertificationError::Config(_)));

        let contract =
            AccountCertificationVerificationError::Rest(RestError::Transport("down".to_string()));
        let expected = contract.to_string();
        let live = AccountCertificationError::from(contract);
        assert_eq!(live.to_string(), expected);
        assert!(matches!(live, AccountCertificationError::Rest(_)));

        let contract = AccountCertificationVerificationError::ResponseTooLarge {
            endpoint: ACCOUNT_BALANCE_ENDPOINT.to_string(),
            actual: 17,
            limit: 16,
        };
        let expected = contract.to_string();
        let live = AccountCertificationError::from(contract);
        assert_eq!(live.to_string(), expected);
        assert!(matches!(
            live,
            AccountCertificationError::ResponseTooLarge {
                actual: 17,
                limit: 16,
                ..
            }
        ));

        let source = serde_json::from_slice::<serde_json::Value>(b"{").unwrap_err();
        let contract = AccountCertificationVerificationError::ParseArtifact {
            path: PathBuf::from("/tmp/artifact.json"),
            source,
        };
        let expected = contract.to_string();
        let live = AccountCertificationError::from(contract);
        assert_eq!(live.to_string(), expected);
        assert!(matches!(
            live,
            AccountCertificationError::ParseArtifact { ref path, .. }
                if path == &PathBuf::from("/tmp/artifact.json")
        ));

        let contract = AccountCertificationVerificationError::ArtifactTooLarge {
            actual: 101,
            limit: 100,
        };
        let expected = contract.to_string();
        let live = AccountCertificationError::from(contract);
        assert_eq!(live.to_string(), expected);
        assert!(matches!(
            live,
            AccountCertificationError::ArtifactTooLarge {
                actual: 101,
                limit: 100,
            }
        ));

        let contract =
            AccountCertificationVerificationError::InvalidEvidence("tampered".to_string());
        let expected = contract.to_string();
        let live = AccountCertificationError::from(contract);
        assert_eq!(live.to_string(), expected);
        assert!(matches!(
            live,
            AccountCertificationError::InvalidEvidence(ref message) if message == "tampered"
        ));
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
        let account_json = r#"{"code":"0","msg":"","data":[{"acctLv":"1","posMode":"net_mode","acctStpMode":"cancel_maker","uid":"7","mainUid":"6","label":"reap-demo","perm":"read_only,trade","ip":"203.0.113.5","enableSpotBorrow":false,"autoLoan":false,"spotBorrowAutoRepay":false}]}"#;
        let balance_json = r#"{"code":"0","msg":"","data":[{"uTime":"1000","totalEq":"1000","adjEq":"1000","borrowFroz":"0","notionalUsdForBorrow":"0","notionalUsd":"0","details":[{"ccy":"USDT","uTime":"1000","cashBal":"1000","availBal":"1000","eq":"1000","eqUsd":"1000","disEq":"1000","upl":"0","liab":"0","crossLiab":"0","interest":"0","borrowFroz":"0","maxLoan":"100","twap":"0"}]}]}"#;
        let index_json = format!(
            r#"{{"code":"0","msg":"","data":[{{"instId":"USDT-USD","idxPx":"1","ts":"{now}"}}]}}"#
        );
        let positions_json = r#"{"code":"0","msg":"","data":[]}"#;
        let responses = VecDeque::from([
            Ok(format!(
                r#"{{"code":"0","msg":"","data":[{{"ts":"{now}"}}]}}"#
            )),
            Ok(account_json.to_string()),
            Ok(balance_json.to_string()),
            Ok(index_json),
            Ok(positions_json.to_string()),
            Ok(account_json.to_string()),
            Ok(format!(
                r#"{{"code":"0","msg":"","data":[{{"ts":"{}"}}]}}"#,
                now + 1
            )),
        ]);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let client = NarrowEvidenceFake::new(responses, Arc::clone(&requests));
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
        assert!(artifact.summary.api_key_policy.passed);
        assert!(artifact.summary.api_key_policy.evidence_complete);
        assert_eq!(artifact.summary.api_key_policy.ip_binding_count, 1);
        assert_eq!(
            artifact.summary.api_key_policy.observed_permissions,
            BTreeSet::from([OkxApiKeyPermission::ReadOnly, OkxApiKeyPermission::Trade])
        );
        let paths = requests.lock().unwrap().iter().cloned().collect::<Vec<_>>();
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

        let mut rehashed_permission_tamper = artifact.clone();
        rehashed_permission_tamper.account_config_before.body = rehashed_permission_tamper
            .account_config_before
            .body
            .replace(
                r#""perm":"read_only,trade""#,
                r#""perm":"read_only,trade,withdraw""#,
            );
        refresh_response_hash(&mut rehashed_permission_tamper.account_config_before);
        std::fs::write(
            &path,
            serde_json::to_vec(&rehashed_permission_tamper).unwrap(),
        )
        .unwrap();
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
