use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use reap_core::{AccountUpdate, PositionMarginMode};
use reap_order::PacingPolicy;
use reap_risk::RiskLimits;
use reap_strategy::{ChaosConfig, InstrumentConfig};
use reap_telemetry::WebhookAlertConfig;
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
    pub order_request_expiry_ms: u64,
    pub safety_latch_sync_timeout_ms: u64,
    pub max_exchange_clock_skew_ms: u64,
    pub exchange_clock_check_interval_ms: u64,
    pub cancel_all_after_timeout_secs: u64,
    pub cancel_all_after_heartbeat_ms: u64,
    pub ambiguous_submit_grace_ms: u64,
    pub fill_state_convergence_timeout_ms: u64,
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
            order_request_expiry_ms: 1_000,
            safety_latch_sync_timeout_ms: 2_000,
            max_exchange_clock_skew_ms: 250,
            exchange_clock_check_interval_ms: 30_000,
            cancel_all_after_timeout_secs: 30,
            cancel_all_after_heartbeat_ms: 1_000,
            ambiguous_submit_grace_ms: 5_000,
            fill_state_convergence_timeout_ms: 2_000,
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
    #[error("alert endpoint environment variable {name} is not set")]
    MissingAlertEndpoint { name: String },
    #[error("alert bearer token environment variable {name} is not set")]
    MissingAlertBearerToken { name: String },
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
        validate_live_strategy_topology(self, &mut errors);
        if let Some(error) = self.risk.validation_error() {
            errors.push(format!("risk: {error}"));
        }
        validate_production_stablecoin_guards(self, &mut errors);
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

    pub(crate) fn position_margin_mode_errors(
        &self,
        account_id: &str,
        update: &AccountUpdate,
    ) -> Vec<String> {
        let Some(account) = self.account(account_id) else {
            return vec![format!("unknown account {account_id}")];
        };
        let mut errors = update
            .positions
            .iter()
            .filter(|position| position.qty != 0.0)
            .filter_map(|position| {
                let instrument = self
                    .strategy
                    .instruments
                    .iter()
                    .find(|instrument| instrument.symbol == position.symbol)?;
                if !instrument.kind.is_derivative()
                    || self.account_for_symbol_unchecked(&position.symbol) != Some(account_id)
                {
                    return None;
                }
                let expected = match account.trade_modes.get(&position.symbol)? {
                    OkxTradeModeConfig::Cross => PositionMarginMode::Cross,
                    OkxTradeModeConfig::Isolated => PositionMarginMode::Isolated,
                    OkxTradeModeConfig::Cash => return None,
                };
                (position.margin_mode != Some(expected)).then(|| {
                    format!(
                        "{} expected {:?}, received {}",
                        position.symbol,
                        expected,
                        position
                            .margin_mode
                            .map(|mode| format!("{mode:?}"))
                            .unwrap_or_else(|| "no mgnMode".to_string())
                    )
                })
            })
            .collect::<Vec<_>>();
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
        ("order_request_expiry_ms", runtime.order_request_expiry_ms),
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
            "fill_state_convergence_timeout_ms",
            runtime.fill_state_convergence_timeout_ms,
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
    if runtime.order_request_expiry_ms > runtime.rest_request_timeout_ms {
        errors.push(
            "runtime.order_request_expiry_ms must not exceed rest_request_timeout_ms".to_string(),
        );
    }
    if runtime.fill_state_convergence_timeout_ms <= runtime.timer_interval_ms {
        errors.push(
            "runtime.fill_state_convergence_timeout_ms must be longer than timer_interval_ms"
                .to_string(),
        );
    }
    if runtime.max_exchange_clock_skew_ms >= runtime.order_request_expiry_ms {
        errors.push(
            "runtime.max_exchange_clock_skew_ms must be shorter than order_request_expiry_ms"
                .to_string(),
        );
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
}
