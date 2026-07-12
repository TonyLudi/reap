use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use reap_order::PacingPolicy;
use reap_risk::RiskLimits;
use reap_strategy::{ChaosConfig, InstrumentConfig};
use reap_venue::okx::{OkxAccountLevel, OkxCredentials, OkxPositionMode, OkxTradeMode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;

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
    pub timer_interval_ms: u64,
    pub readiness_timeout_ms: u64,
    pub shutdown_timeout_ms: u64,
    pub rest_connect_timeout_ms: u64,
    pub rest_request_timeout_ms: u64,
    pub ambiguous_submit_grace_ms: u64,
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
            timer_interval_ms: 100,
            readiness_timeout_ms: 30_000,
            shutdown_timeout_ms: 15_000,
            rest_connect_timeout_ms: 2_000,
            rest_request_timeout_ms: 5_000,
            ambiguous_submit_grace_ms: 5_000,
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
pub struct LiveConfigValidation {
    pub valid: bool,
    pub errors: Vec<String>,
}

#[derive(Debug, Error)]
pub enum LiveConfigError {
    #[error("failed to read live config {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse live config: {0}")]
    Parse(#[from] toml::de::Error),
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
}

impl LiveConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, LiveConfigError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| LiveConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_toml(&text)
    }

    pub fn from_toml(text: &str) -> Result<Self, LiveConfigError> {
        let config: Self = toml::from_str(text)?;
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
        if let Some(error) = self.risk.validation_error() {
            errors.push(format!("risk: {error}"));
        }
        validate_url(
            "venue.rest_url",
            &self.venue.rest_url,
            &["http", "https"],
            &mut errors,
        );
        validate_url(
            "venue.public_ws_url",
            &self.venue.public_ws_url,
            &["ws", "wss"],
            &mut errors,
        );
        validate_url(
            "venue.private_ws_url",
            &self.venue.private_ws_url,
            &["ws", "wss"],
            &mut errors,
        );
        validate_positive_runtime(&self.runtime, &mut errors);
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
    if !account.trade_modes.contains_key(&instrument.symbol) {
        errors.push(format!(
            "account {} has no trade mode for symbol {}",
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
        ("timer_interval_ms", runtime.timer_interval_ms),
        ("readiness_timeout_ms", runtime.readiness_timeout_ms),
        ("shutdown_timeout_ms", runtime.shutdown_timeout_ms),
        ("rest_connect_timeout_ms", runtime.rest_connect_timeout_ms),
        ("rest_request_timeout_ms", runtime.rest_request_timeout_ms),
        (
            "ambiguous_submit_grace_ms",
            runtime.ambiguous_submit_grace_ms,
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

fn validate_url(name: &str, value: &str, schemes: &[&str], errors: &mut Vec<String>) {
    match Url::parse(value) {
        Ok(url) if schemes.contains(&url.scheme()) => {}
        Ok(url) => errors.push(format!(
            "{name} has unsupported scheme {}; expected {}",
            url.scheme(),
            schemes.join(" or ")
        )),
        Err(error) => errors.push(format!("{name} is invalid: {error}")),
    }
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
    fn production_environment_is_explicit_in_parsed_config() {
        let mut config = valid_config();
        config.venue.environment = TradingEnvironment::Production;
        assert!(config.validate().valid);
        assert!(!config.venue.environment.is_demo());
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
    fn config_fingerprint_is_stable_across_hash_map_instances() {
        assert_eq!(
            valid_config().fingerprint().unwrap(),
            valid_config().fingerprint().unwrap()
        );
    }
}
