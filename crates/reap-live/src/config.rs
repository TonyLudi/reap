use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use reap_core::{AccountUpdate, PositionMarginMode};
use reap_feed::OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS;
use reap_order::PacingPolicy;
use reap_risk::RiskLimits;
use reap_strategy::{ChaosConfig, InstrumentConfig};
use reap_telemetry::WebhookAlertConfig;
use reap_venue::okx::{OkxAccountLevel, OkxCredentials, OkxPositionMode, OkxTradeMode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;

pub const MAX_LIVE_CONFIG_BYTES: u64 = 16 * 1024 * 1024;
const MAX_REPORTED_UNKNOWN_FIELDS: usize = 64;
const MAX_CONNECTION_ATTEMPT_INTERVAL_MS: u64 = 60_000;
const MIN_EXCHANGE_STATUS_CHECK_INTERVAL_MS: u64 = 5_000;
const MAX_EXCHANGE_STATUS_LEAD_MS: u64 = 86_400_000;
pub const MAX_ORDER_WEBSOCKET_SESSIONS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveConfigFileEvidence {
    pub source_path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveConfig {
    pub strategy: ChaosConfig,
    #[serde(default)]
    pub risk: RiskLimits,
    #[serde(default)]
    pub venue: OkxVenueConfig,
    #[serde(default)]
    pub runtime: RuntimeConfig,
    #[serde(default)]
    pub storage: LiveStorageConfig,
    #[serde(default)]
    pub operator: OperatorConfig,
    #[serde(default)]
    pub alerts: AlertConfig,
    #[serde(default)]
    pub host_guard: HostGuardConfig,
    pub accounts: Vec<LiveAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OkxVenueConfig {
    pub environment: TradingEnvironment,
    pub rest_url: String,
    pub public_ws_url: String,
    pub private_ws_url: String,
    pub enable_vip_fills_channel: bool,
}

impl Default for OkxVenueConfig {
    fn default() -> Self {
        Self {
            environment: TradingEnvironment::Demo,
            rest_url: "https://openapi.okx.com".to_string(),
            public_ws_url: "wss://wspap.okx.com:8443/ws/v5/public".to_string(),
            private_ws_url: "wss://wspap.okx.com:8443/ws/v5/private".to_string(),
            enable_vip_fills_channel: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradingEnvironment {
    #[default]
    Demo,
    Production,
}

impl TradingEnvironment {
    pub fn is_demo(self) -> bool {
        self == Self::Demo
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OkxEndpointRegion {
    Global,
    UsAu,
    Eea,
    Turkey,
    DemoLoopback,
}

#[derive(Debug, Clone, Copy)]
struct OkxEndpointProfile {
    region: OkxEndpointRegion,
    environment: TradingEnvironment,
    rest_hosts: &'static [&'static str],
    websocket_host: &'static str,
}

const OKX_ENDPOINT_PROFILES: &[OkxEndpointProfile] = &[
    OkxEndpointProfile {
        region: OkxEndpointRegion::Global,
        environment: TradingEnvironment::Demo,
        rest_hosts: &["openapi.okx.com", "www.okx.com"],
        websocket_host: "wspap.okx.com",
    },
    OkxEndpointProfile {
        region: OkxEndpointRegion::Global,
        environment: TradingEnvironment::Production,
        rest_hosts: &["openapi.okx.com", "www.okx.com"],
        websocket_host: "ws.okx.com",
    },
    OkxEndpointProfile {
        region: OkxEndpointRegion::UsAu,
        environment: TradingEnvironment::Demo,
        rest_hosts: &["us.okx.com"],
        websocket_host: "wsuspap.okx.com",
    },
    OkxEndpointProfile {
        region: OkxEndpointRegion::UsAu,
        environment: TradingEnvironment::Production,
        rest_hosts: &["us.okx.com"],
        websocket_host: "wsus.okx.com",
    },
    OkxEndpointProfile {
        region: OkxEndpointRegion::Eea,
        environment: TradingEnvironment::Demo,
        rest_hosts: &["eea.okx.com"],
        websocket_host: "wseeapap.okx.com",
    },
    OkxEndpointProfile {
        region: OkxEndpointRegion::Eea,
        environment: TradingEnvironment::Production,
        rest_hosts: &["eea.okx.com"],
        websocket_host: "wseea.okx.com",
    },
    OkxEndpointProfile {
        region: OkxEndpointRegion::Turkey,
        environment: TradingEnvironment::Production,
        rest_hosts: &["tr.okx.com"],
        websocket_host: "ws.okx.com",
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveAccountConfig {
    pub id: String,
    pub api_key_env: String,
    pub secret_key_env: String,
    pub passphrase_env: String,
    pub expected_account_level: OkxAccountLevel,
    pub expected_position_mode: OkxPositionMode,
    #[serde(default = "default_id_prefix")]
    pub id_prefix: String,
    pub node_id: u16,
    pub trade_modes: HashMap<String, OkxTradeModeConfig>,
}

impl LiveAccountConfig {
    pub fn credentials_from_env(&self) -> Result<OkxCredentials, LiveConfigError> {
        Ok(OkxCredentials::new(
            required_env(&self.id, &self.api_key_env)?,
            required_env(&self.id, &self.secret_key_env)?,
            required_env(&self.id, &self.passphrase_env)?,
        ))
    }

    pub fn trade_mode(&self, symbol: &str) -> Option<OkxTradeMode> {
        self.trade_modes.get(symbol).copied().map(Into::into)
    }
}

fn default_id_prefix() -> String {
    "reap".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OkxTradeModeConfig {
    Cash,
    Cross,
    Isolated,
}

impl From<OkxTradeModeConfig> for OkxTradeMode {
    fn from(value: OkxTradeModeConfig) -> Self {
        match value {
            OkxTradeModeConfig::Cash => Self::Cash,
            OkxTradeModeConfig::Cross => Self::Cross,
            OkxTradeModeConfig::Isolated => Self::Isolated,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    pub event_channel_capacity: usize,
    pub feed_channel_capacity: usize,
    pub order_channel_capacity: usize,
    pub dedup_capacity_per_stream: usize,
    pub max_sequence_buffer: usize,
    pub max_subscriptions_per_socket: usize,
    pub public_connections_per_subscription: usize,
    pub order_websocket_sessions: usize,
    pub connection_attempt_interval_ms: u64,
    pub timer_interval_ms: u64,
    pub readiness_timeout_ms: u64,
    pub shutdown_timeout_ms: u64,
    pub rest_connect_timeout_ms: u64,
    pub rest_request_timeout_ms: u64,
    pub order_request_expiry_ms: u64,
    pub order_websocket_ack_timeout_ms: u64,
    pub safety_latch_sync_timeout_ms: u64,
    pub max_exchange_clock_skew_ms: u64,
    pub exchange_clock_check_interval_ms: u64,
    pub exchange_status_check_interval_ms: u64,
    pub exchange_status_lead_ms: u64,
    pub cancel_all_after_timeout_secs: u64,
    pub cancel_all_after_heartbeat_ms: u64,
    pub ambiguous_submit_grace_ms: u64,
    pub order_state_convergence_timeout_ms: u64,
    pub fill_state_convergence_timeout_ms: u64,
    pub max_fill_reconciliation_pages: usize,
    pub submit_requests_per_window: usize,
    pub cancel_requests_per_window: usize,
    pub reconcile_requests_per_window: usize,
    pub request_window_ms: u64,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            event_channel_capacity: 65_536,
            feed_channel_capacity: 65_536,
            order_channel_capacity: 4_096,
            dedup_capacity_per_stream: 100_000,
            max_sequence_buffer: 4_096,
            max_subscriptions_per_socket: 100,
            public_connections_per_subscription: 2,
            order_websocket_sessions: 8,
            connection_attempt_interval_ms: 400,
            timer_interval_ms: 100,
            readiness_timeout_ms: 30_000,
            shutdown_timeout_ms: 15_000,
            rest_connect_timeout_ms: 2_000,
            rest_request_timeout_ms: 5_000,
            order_request_expiry_ms: 1_000,
            order_websocket_ack_timeout_ms: 5_000,
            safety_latch_sync_timeout_ms: 2_000,
            max_exchange_clock_skew_ms: 250,
            exchange_clock_check_interval_ms: 30_000,
            exchange_status_check_interval_ms: 10_000,
            exchange_status_lead_ms: 60_000,
            cancel_all_after_timeout_secs: 30,
            cancel_all_after_heartbeat_ms: 1_000,
            ambiguous_submit_grace_ms: 10_000,
            order_state_convergence_timeout_ms: 5_000,
            fill_state_convergence_timeout_ms: 2_000,
            max_fill_reconciliation_pages: 20,
            submit_requests_per_window: 50,
            cancel_requests_per_window: 50,
            reconcile_requests_per_window: 20,
            request_window_ms: 2_000,
        }
    }
}

impl RuntimeConfig {
    pub fn pacing_policy(&self) -> PacingPolicy {
        PacingPolicy {
            submit_requests: self.submit_requests_per_window,
            cancel_requests: self.cancel_requests_per_window,
            reconcile_requests: self.reconcile_requests_per_window,
            window: Duration::from_millis(self.request_window_ms),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OperatorConfig {
    pub enabled: bool,
    pub socket_path: PathBuf,
    pub token_env: String,
    pub max_clock_skew_ms: u64,
    pub nonce_ttl_ms: u64,
    pub nonce_capacity: usize,
    pub request_timeout_ms: u64,
    pub max_request_bytes: usize,
    pub command_channel_capacity: usize,
}

impl Default for OperatorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            socket_path: PathBuf::from("var/reap/operator.sock"),
            token_env: "REAP_OPERATOR_TOKEN".to_string(),
            max_clock_skew_ms: 5_000,
            nonce_ttl_ms: 60_000,
            nonce_capacity: 4_096,
            request_timeout_ms: 2_000,
            max_request_bytes: 4_096,
            command_channel_capacity: 64,
        }
    }
}

impl OperatorConfig {
    pub fn secret_from_env(&self) -> Result<Option<Vec<u8>>, LiveConfigError> {
        if !self.enabled {
            return Ok(None);
        }
        let secret =
            std::env::var(&self.token_env).map_err(|_| LiveConfigError::MissingOperatorToken {
                name: self.token_env.clone(),
            })?;
        if secret.len() < 32 {
            return Err(LiveConfigError::OperatorTokenTooShort {
                name: self.token_env.clone(),
            });
        }
        Ok(Some(secret.into_bytes()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LiveStorageConfig {
    pub path: PathBuf,
    pub channel_capacity: usize,
    pub flush_every_records: usize,
}

impl Default for LiveStorageConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("var/reap/live-events.jsonl"),
            channel_capacity: 65_536,
            flush_every_records: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AlertConfig {
    pub enabled: bool,
    pub endpoint_env: String,
    pub bearer_token_env: Option<String>,
    pub channel_capacity: usize,
    pub failure_channel_capacity: usize,
    pub connect_timeout_ms: u64,
    pub request_timeout_ms: u64,
    pub max_attempts: usize,
    pub retry_backoff_ms: u64,
    pub shutdown_timeout_ms: u64,
    pub delivery_failure_is_fatal: bool,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint_env: "REAP_ALERT_WEBHOOK_URL".to_string(),
            bearer_token_env: None,
            channel_capacity: 256,
            failure_channel_capacity: 64,
            connect_timeout_ms: 1_000,
            request_timeout_ms: 2_000,
            max_attempts: 3,
            retry_backoff_ms: 250,
            shutdown_timeout_ms: 10_000,
            delivery_failure_is_fatal: true,
        }
    }
}

impl AlertConfig {
    pub fn webhook_from_env(&self) -> Result<Option<WebhookAlertConfig>, LiveConfigError> {
        if !self.enabled {
            return Ok(None);
        }
        let endpoint = std::env::var(&self.endpoint_env).map_err(|_| {
            LiveConfigError::MissingAlertEndpoint {
                name: self.endpoint_env.clone(),
            }
        })?;
        let bearer_token = self
            .bearer_token_env
            .as_ref()
            .map(|name| {
                std::env::var(name)
                    .map_err(|_| LiveConfigError::MissingAlertBearerToken { name: name.clone() })
            })
            .transpose()?;
        Ok(Some(WebhookAlertConfig {
            endpoint,
            bearer_token,
            channel_capacity: self.channel_capacity,
            failure_channel_capacity: self.failure_channel_capacity,
            request_timeout: Duration::from_millis(self.request_timeout_ms),
            connect_timeout: Duration::from_millis(self.connect_timeout_ms),
            max_attempts: self.max_attempts,
            retry_backoff: Duration::from_millis(self.retry_backoff_ms),
        }))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HostGuardConfig {
    pub enabled: bool,
    pub check_interval_ms: u64,
    pub min_disk_available_bytes: u64,
    pub min_memory_available_bytes: u64,
    pub require_clock_synchronized: bool,
}

impl Default for HostGuardConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            check_interval_ms: 10_000,
            min_disk_available_bytes: 5 * 1024 * 1024 * 1024,
            min_memory_available_bytes: 1024 * 1024 * 1024,
            require_clock_synchronized: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveConfigValidation {
    pub valid: bool,
    pub errors: Vec<String>,
}

#[derive(Debug, Error)]
pub enum LiveConfigError {
    #[error("invalid live config path {path}: {message}")]
    InvalidPath { path: PathBuf, message: String },
    #[error("failed to read live config {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("live config {path} is {actual} bytes; limit is {limit}")]
    TooLarge {
        path: PathBuf,
        actual: u64,
        limit: u64,
    },
    #[error("live config {path} is not UTF-8: {source}")]
    Utf8 {
        path: PathBuf,
        #[source]
        source: std::str::Utf8Error,
    },
    #[error("failed to parse live config: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("live configuration contains unknown fields: {0}")]
    UnknownFields(String),
    #[error("failed to fingerprint live config: {0}")]
    Fingerprint(#[from] serde_json::Error),
    #[error("live configuration is invalid: {0}")]
    Invalid(String),
    #[error("account {account_id} credential environment variable {name} is not set")]
    MissingCredential { account_id: String, name: String },
    #[error("operator token environment variable {name} is not set")]
    MissingOperatorToken { name: String },
    #[error("operator token environment variable {name} must contain at least 32 bytes")]
    OperatorTokenTooShort { name: String },
    #[error("alert endpoint environment variable {name} is not set")]
    MissingAlertEndpoint { name: String },
    #[error("alert bearer token environment variable {name} is not set")]
    MissingAlertBearerToken { name: String },
}

impl LiveConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, LiveConfigError> {
        Self::load_with_evidence(path).map(|(config, _)| config)
    }

    pub fn load_with_evidence(
        path: impl AsRef<Path>,
    ) -> Result<(Self, LiveConfigFileEvidence), LiveConfigError> {
        let path = path.as_ref();
        let metadata =
            std::fs::symlink_metadata(path).map_err(|error| LiveConfigError::InvalidPath {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(LiveConfigError::InvalidPath {
                path: path.to_path_buf(),
                message: "must be a regular file and not a symbolic link".to_string(),
            });
        }
        let canonical =
            std::fs::canonicalize(path).map_err(|error| LiveConfigError::InvalidPath {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?;
        if metadata.len() > MAX_LIVE_CONFIG_BYTES {
            return Err(LiveConfigError::TooLarge {
                path: canonical,
                actual: metadata.len(),
                limit: MAX_LIVE_CONFIG_BYTES,
            });
        }
        let bytes = std::fs::read(&canonical).map_err(|source| LiveConfigError::Read {
            path: canonical.clone(),
            source,
        })?;
        if bytes.len() as u64 > MAX_LIVE_CONFIG_BYTES {
            return Err(LiveConfigError::TooLarge {
                path: canonical,
                actual: bytes.len() as u64,
                limit: MAX_LIVE_CONFIG_BYTES,
            });
        }
        let text = std::str::from_utf8(&bytes).map_err(|source| LiveConfigError::Utf8 {
            path: canonical.clone(),
            source,
        })?;
        let config = Self::from_toml(text)?;
        let evidence = LiveConfigFileEvidence {
            source_path: canonical,
            bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        };
        Ok((config, evidence))
    }

    pub fn from_toml(text: &str) -> Result<Self, LiveConfigError> {
        let mut ignored_count = 0_u64;
        let mut ignored_paths = Vec::new();
        let deserializer = toml::Deserializer::parse(text)?;
        let config: Self = serde_ignored::deserialize(deserializer, |path| {
            ignored_count = ignored_count.saturating_add(1);
            if ignored_paths.len() < MAX_REPORTED_UNKNOWN_FIELDS {
                ignored_paths.push(path.to_string());
            }
        })?;
        if ignored_count > 0 {
            ignored_paths.sort();
            ignored_paths.dedup();
            let omitted = ignored_count.saturating_sub(ignored_paths.len() as u64);
            let mut message = ignored_paths.join(", ");
            if omitted > 0 {
                message.push_str(&format!(", and {omitted} additional field(s)"));
            }
            return Err(LiveConfigError::UnknownFields(message));
        }
        config.ensure_valid()?;
        Ok(config)
    }

    pub fn ensure_valid(&self) -> Result<(), LiveConfigError> {
        let validation = self.validate();
        if validation.valid {
            Ok(())
        } else {
            Err(LiveConfigError::Invalid(validation.errors.join("; ")))
        }
    }

    pub fn validate(&self) -> LiveConfigValidation {
        let mut errors = self.strategy.effective().validate().errors;
        validate_live_strategy_topology(self, &mut errors);
        if let Some(error) = self.risk.validation_error() {
            errors.push(format!("risk: {error}"));
        }
        validate_production_stablecoin_guards(self, &mut errors);
        let endpoint_region = validate_okx_venue_endpoints(&self.venue, &mut errors);
        validate_positive_runtime(&self.runtime, &mut errors);
        validate_connection_attempt_interval(&self.runtime, endpoint_region, &mut errors);
        if self.runtime.fill_state_convergence_timeout_ms > self.risk.max_private_age_ms {
            errors.push(
                "runtime.fill_state_convergence_timeout_ms must not exceed risk.max_private_age_ms"
                    .to_string(),
            );
        }
        if self.storage.path.as_os_str().is_empty() {
            errors.push("storage.path must not be empty".to_string());
        }
        if self.storage.channel_capacity == 0 {
            errors.push("storage.channel_capacity must be positive".to_string());
        }
        if self.storage.flush_every_records == 0 {
            errors.push("storage.flush_every_records must be positive".to_string());
        }
        validate_operator(&self.operator, &self.storage, &mut errors);
        validate_alerts(&self.alerts, &mut errors);
        validate_host_guard(&self.host_guard, &mut errors);

        let mut account_ids = HashSet::new();
        let mut node_ids = HashSet::new();
        for account in &self.accounts {
            if account.id.trim().is_empty() {
                errors.push("account id must not be empty".to_string());
            } else if !account_ids.insert(account.id.as_str()) {
                errors.push(format!("duplicate account id {}", account.id));
            }
            if !node_ids.insert(account.node_id) {
                errors.push(format!("duplicate account node_id {}", account.node_id));
            }
            for (field, value) in [
                ("api_key_env", account.api_key_env.as_str()),
                ("secret_key_env", account.secret_key_env.as_str()),
                ("passphrase_env", account.passphrase_env.as_str()),
                ("id_prefix", account.id_prefix.as_str()),
            ] {
                if value.trim().is_empty() {
                    errors.push(format!("account {} {field} must not be empty", account.id));
                }
            }
            if account.id_prefix.len() > 8
                || !account
                    .id_prefix
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
            {
                errors.push(format!(
                    "account {} id_prefix must contain 1-8 ASCII alphanumeric characters",
                    account.id
                ));
            }
            if account.expected_position_mode != OkxPositionMode::NetMode {
                errors.push(format!(
                    "account {} must use net_mode; long/short position aggregation is not supported",
                    account.id
                ));
            }
        }
        if self.accounts.is_empty() {
            errors.push("at least one live account is required".to_string());
        }

        let groups = self
            .strategy
            .risk_groups
            .iter()
            .map(|group| (group.name.as_str(), group))
            .collect::<HashMap<_, _>>();
        for group in &self.strategy.risk_groups {
            match group.account_id.as_deref() {
                None | Some("") => errors.push(format!(
                    "risk group {} must declare account_id for live trading",
                    group.name
                )),
                Some(account_id) if !account_ids.contains(account_id) => errors.push(format!(
                    "risk group {} references unknown account {}",
                    group.name, account_id
                )),
                Some(_) => {}
            }
        }
        for instrument in &self.strategy.instruments {
            validate_instrument_account(instrument, &groups, self, &mut errors);
        }
        for account in &self.accounts {
            for symbol in account.trade_modes.keys() {
                let owner = self.account_for_symbol_unchecked(symbol);
                if owner != Some(account.id.as_str()) {
                    errors.push(format!(
                        "account {} has trade mode for unowned symbol {}",
                        account.id, symbol
                    ));
                }
            }
        }

        errors.sort();
        errors.dedup();
        LiveConfigValidation {
            valid: errors.is_empty(),
            errors,
        }
    }

    pub fn required_symbols(&self) -> HashSet<String> {
        self.strategy
            .instruments
            .iter()
            .map(|instrument| instrument.symbol.clone())
            .collect()
    }

    pub fn fingerprint(&self) -> Result<String, LiveConfigError> {
        let mut canonical = serde_json::to_value(self)?;
        if let Some(config) = canonical.as_object_mut() {
            // Deployment-only controls must not invalidate order/recovery identity.
            config.remove("alerts");
            config.remove("host_guard");
        }
        let digest = Sha256::digest(serde_json::to_vec(&canonical)?);
        Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }

    /// Hash every effective setting for run-report and calibration provenance.
    pub fn evidence_fingerprint(&self) -> Result<String, LiveConfigError> {
        let canonical = serde_json::to_value(self)?;
        let digest = Sha256::digest(serde_json::to_vec(&canonical)?);
        Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }

    pub fn required_accounts(&self) -> HashSet<String> {
        self.accounts
            .iter()
            .map(|account| account.id.clone())
            .collect()
    }

    pub fn account(&self, account_id: &str) -> Option<&LiveAccountConfig> {
        self.accounts
            .iter()
            .find(|account| account.id == account_id)
    }

    pub fn account_for_symbol(&self, symbol: &str) -> Option<&LiveAccountConfig> {
        self.account_for_symbol_unchecked(symbol)
            .and_then(|account_id| self.account(account_id))
    }

    pub fn instruments_for_account<'a>(
        &'a self,
        account_id: &'a str,
    ) -> impl Iterator<Item = &'a InstrumentConfig> + 'a {
        self.strategy.instruments.iter().filter(move |instrument| {
            self.account_for_symbol_unchecked(&instrument.symbol) == Some(account_id)
        })
    }

    pub(crate) fn account_state_policy_errors(
        &self,
        account_id: &str,
        update: &AccountUpdate,
    ) -> Vec<String> {
        let Some(account) = self.account(account_id) else {
            return vec![format!("unknown account {account_id}")];
        };
        let mut errors = Vec::new();
        for balance in &update.balances {
            if balance.liability != 0.0 {
                errors.push(format!(
                    "currency {} liability {} is nonzero; live borrowing is unsupported",
                    balance.currency, balance.liability
                ));
            }
            if let Some(indicator) = balance.forced_repayment_indicator
                && indicator >= self.risk.forced_repayment_indicator_limit
            {
                errors.push(format!(
                    "currency {} forced repayment indicator {} reached limit {}",
                    balance.currency, indicator, self.risk.forced_repayment_indicator_limit
                ));
            }
        }
        for position in update
            .positions
            .iter()
            .filter(|position| position.qty != 0.0)
        {
            let Some(owner) = self.account_for_symbol_unchecked(&position.symbol) else {
                errors.push(format!(
                    "unmanaged nonzero position {} qty={}",
                    position.symbol, position.qty
                ));
                continue;
            };
            if owner != account_id {
                errors.push(format!(
                    "position {} belongs to configured account {owner}, received on {account_id}",
                    position.symbol
                ));
                continue;
            }
            let instrument = self
                .strategy
                .instruments
                .iter()
                .find(|instrument| instrument.symbol == position.symbol)
                .expect("position owner lookup requires a configured instrument");
            if !instrument.kind.is_derivative() {
                errors.push(format!(
                    "nonzero position {} is not a supported derivative position",
                    position.symbol
                ));
                continue;
            }
            let Some(trade_mode) = account.trade_modes.get(&position.symbol) else {
                errors.push(format!(
                    "position {} has no configured trade mode",
                    position.symbol
                ));
                continue;
            };
            let expected = match trade_mode {
                OkxTradeModeConfig::Cross => PositionMarginMode::Cross,
                OkxTradeModeConfig::Isolated => PositionMarginMode::Isolated,
                OkxTradeModeConfig::Cash => {
                    errors.push(format!(
                        "derivative position {} cannot use cash trade mode",
                        position.symbol
                    ));
                    continue;
                }
            };
            if position.margin_mode != Some(expected) {
                errors.push(format!(
                    "{} expected {:?}, received {}",
                    position.symbol,
                    expected,
                    position
                        .margin_mode
                        .map(|mode| format!("{mode:?}"))
                        .unwrap_or_else(|| "no mgnMode".to_string())
                ));
            }
        }
        errors.sort();
        errors.dedup();
        errors
    }

    fn account_for_symbol_unchecked(&self, symbol: &str) -> Option<&str> {
        let instrument = self
            .strategy
            .instruments
            .iter()
            .find(|instrument| instrument.symbol == symbol)?;
        self.strategy
            .risk_groups
            .iter()
            .find(|group| group.name == instrument.risk_group)?
            .account_id
            .as_deref()
    }
}

fn validate_live_strategy_topology(config: &LiveConfig, errors: &mut Vec<String>) {
    if config.strategy.master_strategy.is_some() {
        errors.push(
            "strategy.master_strategy is not supported by the live runtime because external StrategyUpdate liveness is not implemented"
                .to_string(),
        );
    }
    if config.strategy.strategy_group.is_some() {
        errors.push(
            "strategy.strategy_group is not supported by the live runtime because external group PnL and state updates are not implemented"
                .to_string(),
        );
    }
    for group in &config.strategy.risk_groups {
        for coin in &group.coins {
            if coin.borrow_limit_usd != 0.0 || coin.borrow_limit_coin != 0.0 {
                errors.push(format!(
                    "risk group {} currency {} must set borrow_limit_usd and borrow_limit_coin to zero; live borrowing and interest accounting are unsupported",
                    group.name, coin.currency
                ));
            }
        }
    }
}

fn validate_production_stablecoin_guards(config: &LiveConfig, errors: &mut Vec<String>) {
    if config.venue.environment != TradingEnvironment::Production {
        return;
    }
    for currency in ["USDT", "USDC"] {
        let currency_is_used = config.strategy.instruments.iter().any(|instrument| {
            instrument.base_currency == currency
                || instrument.quote_currency == currency
                || instrument.settle_currency == currency
                || instrument
                    .symbol
                    .split('-')
                    .any(|component| component == currency)
        });
        let required_symbol = format!("{currency}-USD");
        if currency_is_used
            && !config
                .risk
                .stablecoin_guards
                .iter()
                .any(|guard| guard.symbol == required_symbol)
        {
            errors.push(format!(
                "production risk requires stablecoin guard {required_symbol} because the strategy uses {currency}"
            ));
        }
    }
}

fn validate_instrument_account(
    instrument: &InstrumentConfig,
    groups: &HashMap<&str, &reap_strategy::RiskGroupConfig>,
    config: &LiveConfig,
    errors: &mut Vec<String>,
) {
    let Some(group) = groups.get(instrument.risk_group.as_str()) else {
        return;
    };
    let Some(account_id) = group.account_id.as_deref() else {
        return;
    };
    let Some(account) = config.account(account_id) else {
        return;
    };
    let Some(trade_mode) = account.trade_modes.get(&instrument.symbol) else {
        errors.push(format!(
            "account {} has no trade mode for symbol {}",
            account.id, instrument.symbol
        ));
        return;
    };
    if instrument.kind.is_spot() && *trade_mode != OkxTradeModeConfig::Cash {
        errors.push(format!(
            "account {} spot symbol {} must use cash trade mode; margin spot positions are not supported",
            account.id, instrument.symbol
        ));
    }
    if instrument.kind.is_derivative() && *trade_mode == OkxTradeModeConfig::Cash {
        errors.push(format!(
            "account {} derivative symbol {} cannot use cash trade mode",
            account.id, instrument.symbol
        ));
    }
}

fn validate_positive_runtime(runtime: &RuntimeConfig, errors: &mut Vec<String>) {
    for (name, value) in [
        (
            "event_channel_capacity",
            runtime.event_channel_capacity as u64,
        ),
        (
            "feed_channel_capacity",
            runtime.feed_channel_capacity as u64,
        ),
        (
            "order_channel_capacity",
            runtime.order_channel_capacity as u64,
        ),
        (
            "dedup_capacity_per_stream",
            runtime.dedup_capacity_per_stream as u64,
        ),
        ("max_sequence_buffer", runtime.max_sequence_buffer as u64),
        (
            "max_subscriptions_per_socket",
            runtime.max_subscriptions_per_socket as u64,
        ),
        (
            "public_connections_per_subscription",
            runtime.public_connections_per_subscription as u64,
        ),
        (
            "order_websocket_sessions",
            runtime.order_websocket_sessions as u64,
        ),
        ("timer_interval_ms", runtime.timer_interval_ms),
        ("readiness_timeout_ms", runtime.readiness_timeout_ms),
        ("shutdown_timeout_ms", runtime.shutdown_timeout_ms),
        ("rest_connect_timeout_ms", runtime.rest_connect_timeout_ms),
        ("rest_request_timeout_ms", runtime.rest_request_timeout_ms),
        ("order_request_expiry_ms", runtime.order_request_expiry_ms),
        (
            "order_websocket_ack_timeout_ms",
            runtime.order_websocket_ack_timeout_ms,
        ),
        (
            "safety_latch_sync_timeout_ms",
            runtime.safety_latch_sync_timeout_ms,
        ),
        (
            "max_exchange_clock_skew_ms",
            runtime.max_exchange_clock_skew_ms,
        ),
        (
            "exchange_clock_check_interval_ms",
            runtime.exchange_clock_check_interval_ms,
        ),
        (
            "exchange_status_check_interval_ms",
            runtime.exchange_status_check_interval_ms,
        ),
        ("exchange_status_lead_ms", runtime.exchange_status_lead_ms),
        (
            "cancel_all_after_timeout_secs",
            runtime.cancel_all_after_timeout_secs,
        ),
        (
            "cancel_all_after_heartbeat_ms",
            runtime.cancel_all_after_heartbeat_ms,
        ),
        (
            "ambiguous_submit_grace_ms",
            runtime.ambiguous_submit_grace_ms,
        ),
        (
            "order_state_convergence_timeout_ms",
            runtime.order_state_convergence_timeout_ms,
        ),
        (
            "fill_state_convergence_timeout_ms",
            runtime.fill_state_convergence_timeout_ms,
        ),
        (
            "max_fill_reconciliation_pages",
            runtime.max_fill_reconciliation_pages as u64,
        ),
        (
            "submit_requests_per_window",
            runtime.submit_requests_per_window as u64,
        ),
        (
            "cancel_requests_per_window",
            runtime.cancel_requests_per_window as u64,
        ),
        (
            "reconcile_requests_per_window",
            runtime.reconcile_requests_per_window as u64,
        ),
        ("request_window_ms", runtime.request_window_ms),
    ] {
        if value == 0 {
            errors.push(format!("runtime.{name} must be positive"));
        }
    }
    if runtime.rest_request_timeout_ms < runtime.rest_connect_timeout_ms {
        errors.push(
            "runtime.rest_request_timeout_ms must be at least rest_connect_timeout_ms".to_string(),
        );
    }
    if runtime.max_fill_reconciliation_pages > 1_000 {
        errors.push("runtime.max_fill_reconciliation_pages must not exceed 1000".to_string());
    }
    if runtime.order_websocket_sessions > MAX_ORDER_WEBSOCKET_SESSIONS {
        errors.push(format!(
            "runtime.order_websocket_sessions must not exceed {MAX_ORDER_WEBSOCKET_SESSIONS}"
        ));
    }
    if runtime.order_request_expiry_ms > runtime.rest_request_timeout_ms {
        errors.push(
            "runtime.order_request_expiry_ms must not exceed rest_request_timeout_ms".to_string(),
        );
    }
    if runtime.order_request_expiry_ms > runtime.order_websocket_ack_timeout_ms {
        errors.push(
            "runtime.order_request_expiry_ms must not exceed order_websocket_ack_timeout_ms"
                .to_string(),
        );
    }
    let websocket_ambiguity_window_ms = runtime
        .order_request_expiry_ms
        .saturating_add(runtime.order_websocket_ack_timeout_ms);
    if runtime.ambiguous_submit_grace_ms < websocket_ambiguity_window_ms {
        errors.push(
            "runtime.ambiguous_submit_grace_ms must cover order_request_expiry_ms plus order_websocket_ack_timeout_ms"
                .to_string(),
        );
    }
    if runtime.fill_state_convergence_timeout_ms <= runtime.timer_interval_ms {
        errors.push(
            "runtime.fill_state_convergence_timeout_ms must be longer than timer_interval_ms"
                .to_string(),
        );
    }
    if runtime.order_state_convergence_timeout_ms <= runtime.timer_interval_ms {
        errors.push(
            "runtime.order_state_convergence_timeout_ms must be longer than timer_interval_ms"
                .to_string(),
        );
    }
    if runtime.order_state_convergence_timeout_ms < runtime.rest_request_timeout_ms {
        errors.push(
            "runtime.order_state_convergence_timeout_ms must be at least rest_request_timeout_ms"
                .to_string(),
        );
    }
    if runtime.order_state_convergence_timeout_ms < runtime.order_websocket_ack_timeout_ms {
        errors.push(
            "runtime.order_state_convergence_timeout_ms must be at least order_websocket_ack_timeout_ms"
                .to_string(),
        );
    }
    if runtime.max_exchange_clock_skew_ms >= runtime.order_request_expiry_ms {
        errors.push(
            "runtime.max_exchange_clock_skew_ms must be shorter than order_request_expiry_ms"
                .to_string(),
        );
    }
    if runtime.exchange_status_check_interval_ms < MIN_EXCHANGE_STATUS_CHECK_INTERVAL_MS {
        errors.push(format!(
            "runtime.exchange_status_check_interval_ms must be at least {MIN_EXCHANGE_STATUS_CHECK_INTERVAL_MS} to respect the OKX endpoint limit"
        ));
    }
    if runtime.exchange_status_check_interval_ms > runtime.exchange_status_lead_ms {
        errors.push(
            "runtime.exchange_status_check_interval_ms must not exceed exchange_status_lead_ms"
                .to_string(),
        );
    }
    if runtime.exchange_status_lead_ms > MAX_EXCHANGE_STATUS_LEAD_MS {
        errors.push(format!(
            "runtime.exchange_status_lead_ms must not exceed {MAX_EXCHANGE_STATUS_LEAD_MS}"
        ));
    }
    if !(10..=120).contains(&runtime.cancel_all_after_timeout_secs) {
        errors.push("runtime.cancel_all_after_timeout_secs must be between 10 and 120".to_string());
    }
    if runtime.cancel_all_after_heartbeat_ms < 1_000 {
        errors.push(
            "runtime.cancel_all_after_heartbeat_ms must respect the 1 request/second endpoint limit"
                .to_string(),
        );
    }
    if runtime.cancel_all_after_heartbeat_ms
        >= runtime.cancel_all_after_timeout_secs.saturating_mul(1_000)
    {
        errors.push(
            "runtime.cancel_all_after_heartbeat_ms must be shorter than the exchange timeout"
                .to_string(),
        );
    }
    if runtime
        .rest_request_timeout_ms
        .saturating_mul(3)
        .saturating_add(runtime.cancel_all_after_heartbeat_ms)
        >= runtime.cancel_all_after_timeout_secs.saturating_mul(1_000)
    {
        errors.push(
            "runtime deadman timeout must cover three periodic safety REST timeouts plus one heartbeat interval"
                .to_string(),
        );
    }
}

fn validate_connection_attempt_interval(
    runtime: &RuntimeConfig,
    endpoint_region: Option<OkxEndpointRegion>,
    errors: &mut Vec<String>,
) {
    if endpoint_region != Some(OkxEndpointRegion::DemoLoopback)
        && runtime.connection_attempt_interval_ms < OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS
    {
        errors.push(format!(
            "runtime.connection_attempt_interval_ms must be at least {OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS} for official OKX endpoints"
        ));
    }
    if runtime.connection_attempt_interval_ms > MAX_CONNECTION_ATTEMPT_INTERVAL_MS {
        errors.push(format!(
            "runtime.connection_attempt_interval_ms must not exceed {MAX_CONNECTION_ATTEMPT_INTERVAL_MS}"
        ));
    }
}

fn validate_operator(
    operator: &OperatorConfig,
    storage: &LiveStorageConfig,
    errors: &mut Vec<String>,
) {
    if !operator.enabled {
        return;
    }
    if operator.socket_path.as_os_str().is_empty() {
        errors.push("operator.socket_path must not be empty".to_string());
    }
    if operator.socket_path == storage.path {
        errors.push("operator.socket_path must differ from storage.path".to_string());
    }
    if operator.token_env.trim().is_empty() {
        errors.push("operator.token_env must not be empty".to_string());
    }
    for (name, value) in [
        ("max_clock_skew_ms", operator.max_clock_skew_ms),
        ("nonce_ttl_ms", operator.nonce_ttl_ms),
        ("nonce_capacity", operator.nonce_capacity as u64),
        ("request_timeout_ms", operator.request_timeout_ms),
        ("max_request_bytes", operator.max_request_bytes as u64),
        (
            "command_channel_capacity",
            operator.command_channel_capacity as u64,
        ),
    ] {
        if value == 0 {
            errors.push(format!("operator.{name} must be positive"));
        }
    }
    if operator.nonce_ttl_ms < operator.max_clock_skew_ms.saturating_mul(2) {
        errors.push("operator.nonce_ttl_ms must be at least twice max_clock_skew_ms".to_string());
    }
    if operator.max_request_bytes < 512 || operator.max_request_bytes > 65_536 {
        errors.push("operator.max_request_bytes must be between 512 and 65536".to_string());
    }
}

fn validate_alerts(alerts: &AlertConfig, errors: &mut Vec<String>) {
    if !alerts.enabled {
        return;
    }
    if alerts.endpoint_env.trim().is_empty() {
        errors.push("alerts.endpoint_env must not be empty".to_string());
    }
    if alerts
        .bearer_token_env
        .as_ref()
        .is_some_and(|name| name.trim().is_empty())
    {
        errors.push("alerts.bearer_token_env must not be empty when set".to_string());
    }
    for (name, value) in [
        ("channel_capacity", alerts.channel_capacity as u64),
        (
            "failure_channel_capacity",
            alerts.failure_channel_capacity as u64,
        ),
        ("connect_timeout_ms", alerts.connect_timeout_ms),
        ("request_timeout_ms", alerts.request_timeout_ms),
        ("max_attempts", alerts.max_attempts as u64),
        ("retry_backoff_ms", alerts.retry_backoff_ms),
        ("shutdown_timeout_ms", alerts.shutdown_timeout_ms),
    ] {
        if value == 0 {
            errors.push(format!("alerts.{name} must be positive"));
        }
    }
    if alerts.request_timeout_ms < alerts.connect_timeout_ms {
        errors.push("alerts.request_timeout_ms must be at least connect_timeout_ms".to_string());
    }
    if alerts.max_attempts > 10 {
        errors.push("alerts.max_attempts must not exceed 10".to_string());
    }
    if alerts.connect_timeout_ms > 30_000 || alerts.request_timeout_ms > 60_000 {
        errors.push("alert connect/request timeouts exceed the 30s/60s limits".to_string());
    }
    if alerts.retry_backoff_ms > 60_000 || alerts.shutdown_timeout_ms > 300_000 {
        errors.push("alert retry/shutdown timeouts exceed the 60s/300s limits".to_string());
    }
    let mut retry_budget_ms = 0_u64;
    let mut backoff_ms = alerts.retry_backoff_ms;
    for _ in 1..alerts.max_attempts {
        retry_budget_ms = retry_budget_ms.saturating_add(backoff_ms);
        backoff_ms = backoff_ms.saturating_mul(2);
    }
    let delivery_budget_ms = alerts
        .request_timeout_ms
        .saturating_mul(alerts.max_attempts as u64)
        .saturating_add(retry_budget_ms);
    if alerts.shutdown_timeout_ms < delivery_budget_ms {
        errors.push(format!(
            "alerts.shutdown_timeout_ms must cover one worst-case delivery ({delivery_budget_ms}ms)"
        ));
    }
    if alerts.channel_capacity > 65_536 || alerts.failure_channel_capacity > 65_536 {
        errors.push("alert channel capacities must not exceed 65536".to_string());
    }
}

fn validate_host_guard(host_guard: &HostGuardConfig, errors: &mut Vec<String>) {
    if !host_guard.enabled {
        return;
    }
    for (name, value) in [
        ("check_interval_ms", host_guard.check_interval_ms),
        (
            "min_disk_available_bytes",
            host_guard.min_disk_available_bytes,
        ),
        (
            "min_memory_available_bytes",
            host_guard.min_memory_available_bytes,
        ),
    ] {
        if value == 0 {
            errors.push(format!("host_guard.{name} must be positive"));
        }
    }
}

#[derive(Debug)]
struct ParsedOkxEndpoint {
    host: String,
    loopback: bool,
}

impl OkxVenueConfig {
    pub fn endpoint_region(&self) -> Result<OkxEndpointRegion, Vec<String>> {
        let mut errors = Vec::new();
        let region = validate_okx_venue_endpoints(self, &mut errors);
        if errors.is_empty() {
            Ok(region.expect("valid OKX endpoint tuple has a region"))
        } else {
            errors.sort();
            errors.dedup();
            Err(errors)
        }
    }
}

fn validate_okx_venue_endpoints(
    venue: &OkxVenueConfig,
    errors: &mut Vec<String>,
) -> Option<OkxEndpointRegion> {
    let rest = parse_okx_endpoint(
        "venue.rest_url",
        &venue.rest_url,
        venue.environment,
        "https",
        "http",
        "/",
        443,
        errors,
    );
    let public = parse_okx_endpoint(
        "venue.public_ws_url",
        &venue.public_ws_url,
        venue.environment,
        "wss",
        "ws",
        "/ws/v5/public",
        8443,
        errors,
    );
    let private = parse_okx_endpoint(
        "venue.private_ws_url",
        &venue.private_ws_url,
        venue.environment,
        "wss",
        "ws",
        "/ws/v5/private",
        8443,
        errors,
    );
    if venue.public_ws_url == venue.private_ws_url {
        errors.push("venue public and private websocket URLs must differ".to_string());
    }
    let (Some(rest), Some(public), Some(private)) = (rest, public, private) else {
        return None;
    };
    if rest.loopback || public.loopback || private.loopback {
        if venue.environment == TradingEnvironment::Demo
            && rest.loopback
            && public.loopback
            && private.loopback
        {
            return Some(OkxEndpointRegion::DemoLoopback);
        }
        if rest.loopback && public.loopback && private.loopback {
            errors.push("venue loopback endpoint tuple is demo-test only".to_string());
        } else {
            errors.push(
                "venue endpoint tuple must not mix loopback and official OKX endpoints".to_string(),
            );
        }
        return None;
    }
    let profile = OKX_ENDPOINT_PROFILES.iter().find(|profile| {
        profile.environment == venue.environment
            && profile.rest_hosts.contains(&rest.host.as_str())
            && profile.websocket_host == public.host
            && profile.websocket_host == private.host
    });
    match profile {
        Some(profile) => Some(profile.region),
        None => {
            errors.push(format!(
                "venue endpoints are not a documented, region-consistent OKX {:?} tuple",
                venue.environment
            ));
            None
        }
    }
}

pub(crate) fn validate_okx_rest_origin(
    environment: TradingEnvironment,
    name: &str,
    value: &str,
    errors: &mut Vec<String>,
) -> Option<OkxEndpointRegion> {
    let endpoint = parse_okx_endpoint(name, value, environment, "https", "http", "/", 443, errors)?;
    if endpoint.loopback {
        if environment == TradingEnvironment::Demo {
            return Some(OkxEndpointRegion::DemoLoopback);
        }
        errors.push(format!("{name} loopback origin is demo-test only"));
        return None;
    }
    let profile = OKX_ENDPOINT_PROFILES.iter().find(|profile| {
        profile.environment == environment && profile.rest_hosts.contains(&endpoint.host.as_str())
    });
    match profile {
        Some(profile) => Some(profile.region),
        None => {
            errors.push(format!(
                "{name} host is not a documented OKX REST origin for {environment:?}"
            ));
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn parse_okx_endpoint(
    name: &str,
    value: &str,
    environment: TradingEnvironment,
    secure_scheme: &str,
    loopback_scheme: &str,
    expected_path: &str,
    official_port: u16,
    errors: &mut Vec<String>,
) -> Option<ParsedOkxEndpoint> {
    let initial_errors = errors.len();
    let url = match Url::parse(value) {
        Ok(url) => url,
        Err(error) => {
            errors.push(format!("{name} is invalid: {error}"));
            return None;
        }
    };
    let Some(host) = url.host_str() else {
        errors.push(format!("{name} must contain a host"));
        return None;
    };
    let loopback = is_loopback_host(host);
    let demo_loopback = environment == TradingEnvironment::Demo && loopback;
    if url.scheme() != secure_scheme && !(demo_loopback && url.scheme() == loopback_scheme) {
        errors.push(format!(
            "{name} must use {secure_scheme} (loopback {loopback_scheme} is demo-test only)"
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        errors.push(format!("{name} must not contain user information"));
    }
    if url.path() != expected_path || url.query().is_some() || url.fragment().is_some() {
        errors.push(format!(
            "{name} must use exact path {expected_path} without query or fragment"
        ));
    }
    if !demo_loopback && url.port_or_known_default() != Some(official_port) {
        errors.push(format!("{name} must use port {official_port}"));
    }
    if errors.len() != initial_errors {
        return None;
    }
    Some(ParsedOkxEndpoint {
        host: host.to_ascii_lowercase(),
        loopback,
    })
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn required_env(account_id: &str, name: &str) -> Result<String, LiveConfigError> {
    std::env::var(name).map_err(|_| LiveConfigError::MissingCredential {
        account_id: account_id.to_string(),
        name: name.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> LiveConfig {
        let mut strategy: ChaosConfig =
            toml::from_str(include_str!("../../../examples/iarb2-basic.toml")).unwrap();
        strategy.risk_groups[0].account_id = Some("main".to_string());
        LiveConfig {
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
                api_key_env: "OKX_API_KEY".to_string(),
                secret_key_env: "OKX_SECRET_KEY".to_string(),
                passphrase_env: "OKX_PASSPHRASE".to_string(),
                expected_account_level: OkxAccountLevel::SingleCurrencyMargin,
                expected_position_mode: OkxPositionMode::NetMode,
                id_prefix: "reap".to_string(),
                node_id: 1,
                trade_modes: HashMap::from([
                    ("BTC-USDT".to_string(), OkxTradeModeConfig::Cash),
                    ("BTC-PERP".to_string(), OkxTradeModeConfig::Cross),
                ]),
            }],
        }
    }

    fn venue(
        environment: TradingEnvironment,
        rest_url: &str,
        public_ws_url: &str,
        private_ws_url: &str,
    ) -> OkxVenueConfig {
        OkxVenueConfig {
            environment,
            rest_url: rest_url.to_string(),
            public_ws_url: public_ws_url.to_string(),
            private_ws_url: private_ws_url.to_string(),
            enable_vip_fills_channel: false,
        }
    }

    fn global_production_venue() -> OkxVenueConfig {
        venue(
            TradingEnvironment::Production,
            "https://openapi.okx.com",
            "wss://ws.okx.com:8443/ws/v5/public",
            "wss://ws.okx.com:8443/ws/v5/private",
        )
    }

    #[test]
    fn documented_okx_endpoint_profiles_are_accepted() {
        let cases = [
            (OkxVenueConfig::default(), OkxEndpointRegion::Global),
            (global_production_venue(), OkxEndpointRegion::Global),
            (
                venue(
                    TradingEnvironment::Demo,
                    "https://us.okx.com",
                    "wss://wsuspap.okx.com:8443/ws/v5/public",
                    "wss://wsuspap.okx.com:8443/ws/v5/private",
                ),
                OkxEndpointRegion::UsAu,
            ),
            (
                venue(
                    TradingEnvironment::Production,
                    "https://us.okx.com",
                    "wss://wsus.okx.com:8443/ws/v5/public",
                    "wss://wsus.okx.com:8443/ws/v5/private",
                ),
                OkxEndpointRegion::UsAu,
            ),
            (
                venue(
                    TradingEnvironment::Demo,
                    "https://eea.okx.com",
                    "wss://wseeapap.okx.com:8443/ws/v5/public",
                    "wss://wseeapap.okx.com:8443/ws/v5/private",
                ),
                OkxEndpointRegion::Eea,
            ),
            (
                venue(
                    TradingEnvironment::Production,
                    "https://eea.okx.com",
                    "wss://wseea.okx.com:8443/ws/v5/public",
                    "wss://wseea.okx.com:8443/ws/v5/private",
                ),
                OkxEndpointRegion::Eea,
            ),
            (
                venue(
                    TradingEnvironment::Production,
                    "https://tr.okx.com",
                    "wss://ws.okx.com:8443/ws/v5/public",
                    "wss://ws.okx.com:8443/ws/v5/private",
                ),
                OkxEndpointRegion::Turkey,
            ),
        ];
        for (venue, expected) in cases {
            assert_eq!(venue.endpoint_region(), Ok(expected));
        }
    }

    #[test]
    fn authenticated_endpoints_reject_untrusted_or_incoherent_hosts() {
        let arbitrary = OkxVenueConfig {
            rest_url: "https://credentials.example".to_string(),
            ..OkxVenueConfig::default()
        };
        let errors = arbitrary.endpoint_region().unwrap_err();
        assert!(errors.iter().any(|error| error.contains("documented")));

        let mixed_region = OkxVenueConfig {
            rest_url: "https://us.okx.com".to_string(),
            ..OkxVenueConfig::default()
        };
        let errors = mixed_region.endpoint_region().unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("region-consistent"))
        );

        let mut production_with_demo_ws = global_production_venue();
        production_with_demo_ws.public_ws_url = "wss://wspap.okx.com:8443/ws/v5/public".to_string();
        let errors = production_with_demo_ws.endpoint_region().unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("region-consistent"))
        );
    }

    #[test]
    fn authenticated_endpoints_require_tls_exact_ports_paths_and_no_userinfo() {
        let mutations = [
            (
                "http://openapi.okx.com",
                "wss://wspap.okx.com:8443/ws/v5/public",
                "wss://wspap.okx.com:8443/ws/v5/private",
                "must use https",
            ),
            (
                "https://user@openapi.okx.com",
                "wss://wspap.okx.com:8443/ws/v5/public",
                "wss://wspap.okx.com:8443/ws/v5/private",
                "user information",
            ),
            (
                "https://openapi.okx.com/api/v5",
                "wss://wspap.okx.com:8443/ws/v5/public",
                "wss://wspap.okx.com:8443/ws/v5/private",
                "exact path /",
            ),
            (
                "https://openapi.okx.com?redirect=1",
                "wss://wspap.okx.com:8443/ws/v5/public",
                "wss://wspap.okx.com:8443/ws/v5/private",
                "without query",
            ),
            (
                "https://openapi.okx.com",
                "wss://wspap.okx.com/ws/v5/public",
                "wss://wspap.okx.com:8443/ws/v5/private",
                "port 8443",
            ),
            (
                "https://openapi.okx.com",
                "wss://wspap.okx.com:8443/ws/v5/private",
                "wss://wspap.okx.com:8443/ws/v5/public",
                "exact path",
            ),
            (
                "https://openapi.okx.com",
                "wss://wspap.okx.com:8443/ws/v5/public#fragment",
                "wss://wspap.okx.com:8443/ws/v5/private",
                "fragment",
            ),
        ];
        for (rest, public, private, expected) in mutations {
            let errors = venue(TradingEnvironment::Demo, rest, public, private)
                .endpoint_region()
                .unwrap_err();
            assert!(
                errors.iter().any(|error| error.contains(expected)),
                "expected {expected:?} in {errors:?}"
            );
        }
    }

    #[test]
    fn cleartext_loopback_endpoints_are_demo_test_only() {
        let loopback = venue(
            TradingEnvironment::Demo,
            "http://127.0.0.1:18080",
            "ws://localhost:18081/ws/v5/public",
            "ws://[::1]:18082/ws/v5/private",
        );
        assert_eq!(
            loopback.endpoint_region(),
            Ok(OkxEndpointRegion::DemoLoopback)
        );

        let mut production = loopback;
        production.environment = TradingEnvironment::Production;
        let errors = production.endpoint_region().unwrap_err();
        assert!(errors.iter().any(|error| error.contains("must use https")));
        assert!(errors.iter().any(|error| error.contains("must use wss")));

        let tls_loopback = venue(
            TradingEnvironment::Production,
            "https://127.0.0.1",
            "wss://127.0.0.1:8443/ws/v5/public",
            "wss://127.0.0.1:8443/ws/v5/private",
        );
        let errors = tls_loopback.endpoint_region().unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("loopback endpoint tuple is demo-test only"))
        );
    }

    #[test]
    fn official_websocket_connection_attempts_require_exchange_safe_pacing() {
        let mut config = valid_config();
        config.runtime.connection_attempt_interval_ms = OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS - 1;
        let validation = config.validate();
        assert!(!validation.valid);
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("must be at least 334"))
        );

        config.venue = venue(
            TradingEnvironment::Demo,
            "http://127.0.0.1:18080",
            "ws://127.0.0.1:18081/ws/v5/public",
            "ws://127.0.0.1:18082/ws/v5/private",
        );
        config.runtime.connection_attempt_interval_ms = 0;
        assert!(config.validate().valid);

        config.runtime.connection_attempt_interval_ms = MAX_CONNECTION_ATTEMPT_INTERVAL_MS + 1;
        let validation = config.validate();
        assert!(!validation.valid);
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("must not exceed 60000"))
        );
    }

    #[test]
    fn order_websocket_pool_size_is_positive_and_bounded() {
        let mut config = valid_config();
        config.runtime.order_websocket_sessions = 0;
        let validation = config.validate();
        assert!(!validation.valid);
        assert!(
            validation.errors.iter().any(|error| {
                error.contains("runtime.order_websocket_sessions must be positive")
            })
        );

        config.runtime.order_websocket_sessions = MAX_ORDER_WEBSOCKET_SESSIONS + 1;
        let validation = config.validate();
        assert!(!validation.valid);
        assert!(validation.errors.iter().any(|error| {
            error.contains("runtime.order_websocket_sessions must not exceed 16")
        }));
    }

    #[test]
    fn order_websocket_timeouts_preserve_expiry_and_reconciliation_windows() {
        let mut config = valid_config();
        config.runtime.order_websocket_ack_timeout_ms =
            config.runtime.order_request_expiry_ms.saturating_sub(1);
        let validation = config.validate();
        assert!(!validation.valid);
        assert!(validation.errors.iter().any(|error| {
            error.contains(
                "runtime.order_request_expiry_ms must not exceed order_websocket_ack_timeout_ms",
            )
        }));

        let mut config = valid_config();
        config.runtime.ambiguous_submit_grace_ms = config
            .runtime
            .order_request_expiry_ms
            .saturating_add(config.runtime.order_websocket_ack_timeout_ms)
            .saturating_sub(1);
        let validation = config.validate();
        assert!(!validation.valid);
        assert!(validation.errors.iter().any(|error| {
            error.contains(
                "runtime.ambiguous_submit_grace_ms must cover order_request_expiry_ms plus order_websocket_ack_timeout_ms",
            )
        }));

        config.runtime.ambiguous_submit_grace_ms = 10_000;
        config.runtime.order_websocket_ack_timeout_ms = config
            .runtime
            .order_state_convergence_timeout_ms
            .saturating_add(1);
        let validation = config.validate();
        assert!(!validation.valid);
        assert!(validation.errors.iter().any(|error| {
            error.contains(
                "runtime.order_state_convergence_timeout_ms must be at least order_websocket_ack_timeout_ms",
            )
        }));
    }

    #[test]
    fn live_parser_rejects_unknown_fields_at_every_config_layer() {
        let mut document = toml::Value::try_from(valid_config()).unwrap();
        document
            .as_table_mut()
            .unwrap()
            .insert("top_level_typo".to_string(), toml::Value::Boolean(true));
        document["venue"]
            .as_table_mut()
            .unwrap()
            .insert("rest_url_typo".to_string(), toml::Value::Boolean(true));
        document["risk"]
            .as_table_mut()
            .unwrap()
            .insert("max_drawdown_typo".to_string(), toml::Value::Boolean(true));
        document["runtime"].as_table_mut().unwrap().insert(
            "timer_interval_typo".to_string(),
            toml::Value::Boolean(true),
        );
        document["strategy"]["instruments"].as_array_mut().unwrap()[0]
            .as_table_mut()
            .unwrap()
            .insert("tick_size_typo".to_string(), toml::Value::Boolean(true));
        document["accounts"].as_array_mut().unwrap()[0]
            .as_table_mut()
            .unwrap()
            .insert("node_id_typo".to_string(), toml::Value::Boolean(true));

        let error = LiveConfig::from_toml(&toml::to_string(&document).unwrap())
            .unwrap_err()
            .to_string();

        assert!(error.contains("top_level_typo"), "{error}");
        assert!(error.contains("rest_url_typo"), "{error}");
        assert!(error.contains("max_drawdown_typo"), "{error}");
        assert!(error.contains("timer_interval_typo"), "{error}");
        assert!(error.contains("tick_size_typo"), "{error}");
        assert!(error.contains("node_id_typo"), "{error}");
    }

    #[test]
    fn live_config_requires_total_account_and_trade_mode_mapping() {
        let config = valid_config();
        assert!(config.validate().valid, "{:?}", config.validate().errors);
        assert_eq!(config.account_for_symbol("BTC-USDT").unwrap().id, "main");

        let mut missing = config;
        missing.accounts[0].trade_modes.remove("BTC-PERP");
        let report = missing.validate();
        assert!(!report.valid);
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("no trade mode for symbol BTC-PERP"))
        );
    }

    #[test]
    fn live_config_rejects_unmodeled_spot_and_derivative_trade_modes() {
        let mut margin_spot = valid_config();
        margin_spot.accounts[0]
            .trade_modes
            .insert("BTC-USDT".to_string(), OkxTradeModeConfig::Cross);
        let report = margin_spot.validate();
        assert!(report.errors.iter().any(|error| {
            error.contains(
                "spot symbol BTC-USDT must use cash trade mode; margin spot positions are not supported",
            )
        }));

        let mut cash_derivative = valid_config();
        cash_derivative.accounts[0]
            .trade_modes
            .insert("BTC-PERP".to_string(), OkxTradeModeConfig::Cash);
        let report = cash_derivative.validate();
        assert!(report.errors.iter().any(|error| {
            error.contains("derivative symbol BTC-PERP cannot use cash trade mode")
        }));
    }

    #[test]
    fn production_environment_is_explicit_in_parsed_config() {
        let mut config = valid_config();
        config.venue = global_production_venue();
        let missing_guard = config.validate();
        assert!(!missing_guard.valid);
        assert!(
            missing_guard.errors.iter().any(|error| {
                error.contains("production risk requires stablecoin guard USDT-USD")
            })
        );
        config.risk.stablecoin_guards = vec![reap_risk::StablecoinGuardConfig {
            symbol: "USDT-USD".to_string(),
            max_downside_deviation: 0.01,
        }];
        assert!(config.validate().valid);
        assert!(!config.venue.environment.is_demo());
    }

    #[test]
    fn live_config_rejects_external_strategy_topology_without_coordination_feeds() {
        let mut config = valid_config();
        config.strategy.master_strategy = Some("leader".to_string());
        config.strategy.strategy_group = Some("portfolio".to_string());
        assert!(config.strategy.effective().validate().valid);

        let report = config.validate();

        assert!(!report.valid);
        assert!(report.errors.iter().any(|error| {
            error.contains("master_strategy") && error.contains("StrategyUpdate liveness")
        }));
        assert!(report.errors.iter().any(|error| {
            error.contains("strategy_group") && error.contains("group PnL and state updates")
        }));
    }

    #[test]
    fn live_config_rejects_nonzero_borrow_limits() {
        let mut config = valid_config();
        config.strategy.risk_groups[0].coins = vec![reap_strategy::CoinConfig {
            currency: "USDT".to_string(),
            borrow_limit_usd: 100.0,
            borrow_limit_coin: 100.0,
            ..reap_strategy::CoinConfig::default()
        }];

        let report = config.validate();

        assert!(!report.valid);
        assert!(report.errors.iter().any(|error| {
            error.contains("must set borrow_limit_usd and borrow_limit_coin to zero")
        }));
    }

    #[test]
    fn enabled_operator_service_requires_bounded_distinct_configuration() {
        assert!(
            OperatorConfig::default()
                .secret_from_env()
                .unwrap()
                .is_none()
        );

        let mut config = valid_config();
        config.operator.enabled = true;
        config.operator.socket_path = config.storage.path.clone();
        config.operator.max_request_bytes = 1;
        config.operator.nonce_ttl_ms = config.operator.max_clock_skew_ms;
        let report = config.validate();

        assert!(!report.valid);
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("must differ from storage.path"))
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("between 512 and 65536"))
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("at least twice max_clock_skew_ms"))
        );
    }

    #[test]
    fn exchange_clock_budget_must_fit_inside_order_expiry() {
        let mut config = valid_config();
        config.runtime.max_exchange_clock_skew_ms = config.runtime.order_request_expiry_ms;

        let report = config.validate();

        assert!(!report.valid);
        assert!(
            report.errors.iter().any(|error| error.contains(
                "max_exchange_clock_skew_ms must be shorter than order_request_expiry_ms"
            ))
        );
    }

    #[test]
    fn periodic_safety_checks_must_fit_inside_deadman_horizon() {
        let mut config = valid_config();
        config.runtime.rest_request_timeout_ms = 15_000;

        let report = config.validate();

        assert!(!report.valid);
        assert!(report.errors.iter().any(|error| {
            error.contains(
                "deadman timeout must cover three periodic safety REST timeouts plus one heartbeat",
            )
        }));
    }

    #[test]
    fn exchange_status_guard_respects_endpoint_rate_and_lead_window() {
        let mut config = valid_config();
        config.runtime.exchange_status_check_interval_ms = 4_999;
        config.runtime.exchange_status_lead_ms = 4_000;

        let report = config.validate();

        assert!(!report.valid);
        assert!(report.errors.iter().any(|error| {
            error.contains("exchange_status_check_interval_ms must be at least 5000")
        }));
        assert!(
            report.errors.iter().any(|error| {
                error.contains("exchange_status_check_interval_ms must not exceed")
            })
        );
    }

    #[test]
    fn order_state_convergence_budget_must_cover_rest_request() {
        let mut config = valid_config();
        config.runtime.order_state_convergence_timeout_ms =
            config.runtime.rest_request_timeout_ms - 1;

        let report = config.validate();

        assert!(!report.valid);
        assert!(report.errors.iter().any(|error| error.contains(
            "order_state_convergence_timeout_ms must be at least rest_request_timeout_ms"
        )));
    }

    #[test]
    fn enabled_alerts_and_host_guard_require_bounded_settings() {
        assert!(AlertConfig::default().webhook_from_env().unwrap().is_none());

        let mut config = valid_config();
        config.alerts.enabled = true;
        config.alerts.endpoint_env.clear();
        config.alerts.request_timeout_ms = 10;
        config.alerts.connect_timeout_ms = 20;
        config.alerts.max_attempts = 11;
        config.host_guard.enabled = true;
        config.host_guard.check_interval_ms = 0;
        let report = config.validate();

        assert!(!report.valid);
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("alerts.endpoint_env must not be empty"))
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("at least connect_timeout_ms"))
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("max_attempts must not exceed 10"))
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("host_guard.check_interval_ms must be positive"))
        );
    }

    #[test]
    fn config_fingerprint_is_stable_across_hash_map_instances() {
        assert_eq!(
            valid_config().fingerprint().unwrap(),
            valid_config().fingerprint().unwrap()
        );

        let baseline = valid_config();
        let mut reordered = valid_config();
        reordered.accounts[0].trade_modes = [
            ("BTC-PERP".to_string(), OkxTradeModeConfig::Cross),
            ("BTC-USDT".to_string(), OkxTradeModeConfig::Cash),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            baseline.evidence_fingerprint().unwrap(),
            reordered.evidence_fingerprint().unwrap()
        );
    }

    #[test]
    fn deployment_guards_do_not_change_checkpoint_identity() {
        let baseline = valid_config();
        let mut guarded = baseline.clone();
        guarded.alerts.enabled = true;
        guarded.alerts.endpoint_env = "DIFFERENT_ALERT_ENDPOINT".to_string();
        guarded.host_guard.enabled = true;
        guarded.host_guard.min_disk_available_bytes = u64::MAX;

        assert_eq!(
            baseline.fingerprint().unwrap(),
            guarded.fingerprint().unwrap()
        );
        assert_ne!(
            baseline.evidence_fingerprint().unwrap(),
            guarded.evidence_fingerprint().unwrap()
        );
    }

    #[test]
    fn fill_state_convergence_timeout_must_exceed_timer_resolution() {
        let mut config = valid_config();
        config.runtime.fill_state_convergence_timeout_ms = config.runtime.timer_interval_ms;

        let report = config.validate();

        assert!(!report.valid);
        assert!(report.errors.iter().any(|error| {
            error.contains(
                "runtime.fill_state_convergence_timeout_ms must be longer than timer_interval_ms",
            )
        }));

        let mut config = valid_config();
        config.runtime.fill_state_convergence_timeout_ms = config.risk.max_private_age_ms + 1;
        let report = config.validate();
        assert!(report.errors.iter().any(|error| {
            error.contains(
                "runtime.fill_state_convergence_timeout_ms must not exceed risk.max_private_age_ms",
            )
        }));
    }

    #[test]
    fn fill_reconciliation_page_limit_is_bounded() {
        let mut config = valid_config();
        config.runtime.max_fill_reconciliation_pages = 0;
        let report = config.validate();
        assert!(report.errors.iter().any(|error| {
            error.contains("runtime.max_fill_reconciliation_pages must be positive")
        }));

        let mut config = valid_config();
        config.runtime.max_fill_reconciliation_pages = 1_001;
        let report = config.validate();
        assert!(report.errors.iter().any(|error| {
            error.contains("runtime.max_fill_reconciliation_pages must not exceed 1000")
        }));
    }
}
