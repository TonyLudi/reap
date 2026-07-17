use std::collections::HashSet;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use reap_core::{Channel, FeedPriority, Subscription, Venue};
use reap_feed::{
    DEFAULT_OKX_CONNECTION_ATTEMPT_PACER_PATH, OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS, SocketPlan,
    partition_subscriptions,
};
use reap_telemetry::HostGuardConfig;
use reap_venue::okx::{okx_capability_registration, okx_public_channel_registration};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::runtime::CaptureError;
use crate::writer::sha256_hex;

pub const MAX_CAPTURE_CONFIG_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CONNECTION_ATTEMPT_INTERVAL_MS: u64 = 60_000;
const MAX_REPORTED_UNKNOWN_FIELDS: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureConfig {
    #[serde(default)]
    pub venue: CaptureVenueConfig,
    #[serde(default)]
    pub runtime: CaptureRuntimeConfig,
    #[serde(default)]
    pub output: CaptureOutputConfig,
    #[serde(default)]
    pub host_guard: HostGuardConfig,
    pub subscriptions: Vec<CaptureSubscriptionConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureVenueConfig {
    pub public_ws_url: String,
}

impl Default for CaptureVenueConfig {
    fn default() -> Self {
        Self {
            public_ws_url: "wss://ws.okx.com:8443/ws/v5/public".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureRuntimeConfig {
    pub feed_channel_capacity: usize,
    pub writer_channel_capacity: usize,
    pub dedup_capacity_per_stream: usize,
    pub max_sequence_buffer: usize,
    pub max_subscriptions_per_socket: usize,
    pub connection_attempt_interval_ms: u64,
    pub connection_attempt_pacer_path: Option<PathBuf>,
    pub health_interval_ms: u64,
    pub max_book_age_ms: u64,
}

impl Default for CaptureRuntimeConfig {
    fn default() -> Self {
        Self {
            feed_channel_capacity: 65_536,
            writer_channel_capacity: 65_536,
            dedup_capacity_per_stream: 100_000,
            max_sequence_buffer: 4_096,
            max_subscriptions_per_socket: 100,
            connection_attempt_interval_ms: 400,
            connection_attempt_pacer_path: Some(PathBuf::from(
                DEFAULT_OKX_CONNECTION_ATTEMPT_PACER_PATH,
            )),
            health_interval_ms: 1_000,
            max_book_age_ms: 5_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureOutputConfig {
    pub raw_path: PathBuf,
    pub normalized_path: Option<PathBuf>,
    pub flush_every_records: usize,
    pub fsync_every_records: usize,
}

impl Default for CaptureOutputConfig {
    fn default() -> Self {
        Self {
            raw_path: PathBuf::from("var/reap/capture/okx-raw.jsonl"),
            normalized_path: None,
            flush_every_records: 1_024,
            fsync_every_records: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureSubscriptionConfig {
    pub channel: String,
    pub symbol: String,
    #[serde(default = "default_connections")]
    pub connections: usize,
    #[serde(default)]
    pub priority: CapturePriority,
}

fn default_connections() -> usize {
    2
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapturePriority {
    #[default]
    Critical,
    High,
    Normal,
    Low,
}

impl From<CapturePriority> for FeedPriority {
    fn from(value: CapturePriority) -> Self {
        match value {
            CapturePriority::Critical => Self::Critical,
            CapturePriority::High => Self::High,
            CapturePriority::Normal => Self::Normal,
            CapturePriority::Low => Self::Low,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureValidation {
    pub valid: bool,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureConfigFileEvidence {
    pub source_path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

impl CaptureConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, CaptureError> {
        Self::load_with_evidence(path).map(|(config, _)| config)
    }

    pub fn load_with_evidence(
        path: impl AsRef<Path>,
    ) -> Result<(Self, CaptureConfigFileEvidence), CaptureError> {
        let requested_path = path.as_ref();
        let metadata = std::fs::symlink_metadata(requested_path).map_err(|source| {
            CaptureError::ReadConfig {
                path: requested_path.to_path_buf(),
                source,
            }
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(CaptureError::InvalidConfigPath {
                path: requested_path.to_path_buf(),
                message: "must be a regular file and not a symbolic link".to_string(),
            });
        }
        let path =
            std::fs::canonicalize(requested_path).map_err(|source| CaptureError::ReadConfig {
                path: requested_path.to_path_buf(),
                source,
            })?;
        if metadata.len() > MAX_CAPTURE_CONFIG_BYTES {
            return Err(CaptureError::ConfigTooLarge {
                path,
                actual: metadata.len(),
                limit: MAX_CAPTURE_CONFIG_BYTES,
            });
        }
        let bytes = std::fs::read(&path).map_err(|source| CaptureError::ReadConfig {
            path: path.clone(),
            source,
        })?;
        if bytes.len() as u64 > MAX_CAPTURE_CONFIG_BYTES {
            return Err(CaptureError::ConfigTooLarge {
                path,
                actual: bytes.len() as u64,
                limit: MAX_CAPTURE_CONFIG_BYTES,
            });
        }
        let text = std::str::from_utf8(&bytes).map_err(|error| {
            CaptureError::InvalidConfig(format!("capture config is not UTF-8: {error}"))
        })?;
        let config = Self::from_toml(text)?;
        let evidence = CaptureConfigFileEvidence {
            source_path: path,
            bytes: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
        };
        Ok((config, evidence))
    }

    pub fn from_toml(text: &str) -> Result<Self, CaptureError> {
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
            return Err(CaptureError::UnknownFields(message));
        }
        config.ensure_valid()?;
        Ok(config)
    }

    pub fn fingerprint(&self) -> Result<String, CaptureError> {
        Ok(sha256_hex(&serde_json::to_vec(self)?))
    }

    pub fn ensure_valid(&self) -> Result<(), CaptureError> {
        let validation = self.validate();
        if validation.valid {
            Ok(())
        } else {
            Err(CaptureError::InvalidConfig(validation.errors.join("; ")))
        }
    }

    pub fn validate(&self) -> CaptureValidation {
        let mut errors = Vec::new();
        let loopback = validate_ws_url(&self.venue.public_ws_url, &mut errors);
        errors.extend(self.host_guard.validation_errors("host_guard"));
        for (name, value) in [
            (
                "runtime.feed_channel_capacity",
                self.runtime.feed_channel_capacity,
            ),
            (
                "runtime.writer_channel_capacity",
                self.runtime.writer_channel_capacity,
            ),
            (
                "runtime.dedup_capacity_per_stream",
                self.runtime.dedup_capacity_per_stream,
            ),
            (
                "runtime.max_sequence_buffer",
                self.runtime.max_sequence_buffer,
            ),
            (
                "runtime.max_subscriptions_per_socket",
                self.runtime.max_subscriptions_per_socket,
            ),
        ] {
            if value == 0 {
                errors.push(format!("{name} must be positive"));
            }
        }
        if self.runtime.health_interval_ms == 0 {
            errors.push("runtime.health_interval_ms must be positive".to_string());
        }
        if self.runtime.max_book_age_ms == 0 {
            errors.push("runtime.max_book_age_ms must be positive".to_string());
        }
        if !loopback
            && self.runtime.connection_attempt_interval_ms < OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS
        {
            errors.push(format!(
                "runtime.connection_attempt_interval_ms must be at least {OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS} for non-loopback OKX endpoints"
            ));
        }
        if self.runtime.connection_attempt_interval_ms > MAX_CONNECTION_ATTEMPT_INTERVAL_MS {
            errors.push(format!(
                "runtime.connection_attempt_interval_ms must not exceed {MAX_CONNECTION_ATTEMPT_INTERVAL_MS}"
            ));
        }
        match self.runtime.connection_attempt_pacer_path.as_ref() {
            Some(path) if path.as_os_str().is_empty() => errors.push(
                "runtime.connection_attempt_pacer_path must not be empty when set".to_string(),
            ),
            None if !loopback => errors.push(
                "runtime.connection_attempt_pacer_path is required for official OKX endpoints"
                    .to_string(),
            ),
            _ => {}
        }
        if let Some(path) = self.runtime.connection_attempt_pacer_path.as_ref() {
            if path == &self.output.raw_path {
                errors.push(
                    "runtime.connection_attempt_pacer_path must differ from output.raw_path"
                        .to_string(),
                );
            }
            if self.output.normalized_path.as_ref() == Some(path) {
                errors.push(
                    "runtime.connection_attempt_pacer_path must differ from output.normalized_path"
                        .to_string(),
                );
            }
        }
        if self.output.raw_path.as_os_str().is_empty() {
            errors.push("output.raw_path must not be empty".to_string());
        }
        if let Some(normalized_path) = &self.output.normalized_path {
            if normalized_path.as_os_str().is_empty() {
                errors.push("output.normalized_path must not be empty when set".to_string());
            }
            if self.output.raw_path == *normalized_path {
                errors.push("raw and normalized output paths must differ".to_string());
            }
        }
        if self.output.flush_every_records == 0 {
            errors.push("output.flush_every_records must be positive".to_string());
        }
        if self.subscriptions.is_empty() {
            errors.push("at least one public subscription is required".to_string());
        }

        let mut seen = HashSet::new();
        let mut book_symbols = HashSet::new();
        let mut has_book = false;
        for subscription in &self.subscriptions {
            let channel = subscription.channel.trim();
            let symbol = subscription.symbol.trim();
            if !supported_public_channel(channel) {
                errors.push(format!(
                    "unsupported public capture channel {}",
                    subscription.channel
                ));
            }
            if symbol.is_empty() {
                errors.push(format!("capture channel {channel} requires a symbol"));
            }
            if subscription.connections == 0 {
                errors.push(format!(
                    "capture subscription {channel}/{symbol} connections must be positive"
                ));
            }
            if !seen.insert((channel.to_string(), symbol.to_string())) {
                errors.push(format!("duplicate capture subscription {channel}/{symbol}"));
            }
            if is_book_channel(channel) {
                has_book = true;
                if !book_symbols.insert(symbol.to_string()) {
                    errors.push(format!(
                        "capture symbol {symbol} must use exactly one order-book channel"
                    ));
                }
            }
        }
        if !has_book {
            errors.push("at least one order-book subscription is required".to_string());
        }

        errors.sort();
        errors.dedup();
        CaptureValidation {
            valid: errors.is_empty(),
            errors,
        }
    }

    pub(crate) fn subscriptions(&self) -> Vec<Subscription> {
        self.subscriptions
            .iter()
            .map(CaptureSubscriptionConfig::subscription)
            .collect()
    }

    pub(crate) fn socket_plans(&self) -> Result<Vec<SocketPlan>, CaptureError> {
        let _connection_capability = okx_capability_registration("OKX-CONNECTION-CAPTURE-PUBLIC")
            .expect("capture public connection must remain in the OKX capability registry");
        Ok(partition_subscriptions(
            &self.subscriptions(),
            self.runtime.max_subscriptions_per_socket,
        )?)
    }

    pub(crate) fn expected_book_symbols(&self) -> HashSet<String> {
        self.subscriptions
            .iter()
            .filter(|subscription| is_book_channel(subscription.channel.trim()))
            .map(|subscription| subscription.symbol.trim().to_string())
            .collect()
    }
}

impl CaptureSubscriptionConfig {
    fn subscription(&self) -> Subscription {
        let registration = okx_public_channel_registration(self.channel.trim())
            .expect("validated capture channel must remain registered");
        let channel = match registration.endpoint_or_channel {
            "books" => Channel::Books,
            "trades" => Channel::Trades,
            channel => Channel::Custom(channel.to_string()),
        };
        let mut subscription = Subscription::public(
            Venue::Okx,
            channel,
            self.symbol.trim(),
            self.priority.into(),
        );
        subscription.connections = self.connections;
        subscription
    }
}

fn supported_public_channel(channel: &str) -> bool {
    okx_public_channel_registration(channel).is_some_and(|capability| {
        capability
            .requirement_ids
            .contains(&"CAPTURE-PUBLIC-MARKET")
    })
}

pub(crate) fn is_book_channel(channel: &str) -> bool {
    matches!(channel, "books" | "books-l2-tbt" | "books50-l2-tbt")
}

fn validate_ws_url(value: &str, errors: &mut Vec<String>) -> bool {
    match Url::parse(value) {
        Ok(url) => {
            let loopback = url.host_str().is_some_and(is_loopback_host);
            let loopback_ws = url.scheme() == "ws" && loopback;
            if url.scheme() != "wss" && !loopback_ws {
                errors.push(
                    "venue.public_ws_url must use wss (loopback ws is test-only)".to_string(),
                );
            }
            if !url.username().is_empty() || url.password().is_some() {
                errors.push("venue.public_ws_url must not contain user information".to_string());
            }
            loopback
        }
        Err(error) => {
            errors.push(format!("venue.public_ws_url is invalid: {error}"));
            false
        }
    }
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}
