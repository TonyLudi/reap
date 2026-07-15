use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use reap_feed::{
    DEFAULT_OKX_CONNECTION_ATTEMPT_PACER_PATH, OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS,
};
use reap_live::{LiveConfig, OkxEndpointRegion, OkxVenueConfig, TradingEnvironment};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

const CONFIG_SCHEMA_VERSION: u32 = 1;
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaultProxyConfig {
    pub schema_version: u32,
    pub rest_listen: SocketAddr,
    pub public_ws_listen: SocketAddr,
    pub private_ws_listen: SocketAddr,
    pub order_ws_listen: SocketAddr,
    pub control_socket: PathBuf,
    pub evidence_directory: PathBuf,
    pub upstream: FaultProxyUpstream,
    #[serde(default = "default_connection_attempt_interval_ms")]
    pub connection_attempt_interval_ms: u64,
    #[serde(default = "default_connection_attempt_pacer_path")]
    pub connection_attempt_pacer_path: PathBuf,
    #[serde(default = "default_request_timeout_ms")]
    pub request_timeout_ms: u64,
    #[serde(default = "default_max_http_body_bytes")]
    pub max_http_body_bytes: usize,
    #[serde(default = "default_max_pending_faults")]
    pub max_pending_faults: usize,
    #[serde(default = "default_shutdown_timeout_ms")]
    pub shutdown_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaultProxyUpstream {
    pub rest_url: String,
    pub public_ws_url: String,
    pub private_ws_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaultProxyConfigEvidence {
    pub source_path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub effective_fingerprint: String,
}

#[derive(Debug, Error)]
pub enum FaultProxyConfigError {
    #[error("invalid fault-proxy config path {path}: {message}")]
    InvalidPath { path: PathBuf, message: String },
    #[error("failed to inspect fault-proxy config {path}: {source}")]
    Inspect {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("fault-proxy config {path} is {bytes} bytes; maximum is {maximum}")]
    TooLarge {
        path: PathBuf,
        bytes: u64,
        maximum: u64,
    },
    #[error("failed to read fault-proxy config {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse fault-proxy config {path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("invalid fault-proxy config: {0}")]
    Invalid(String),
    #[error("failed to serialize effective fault-proxy config: {0}")]
    Serialize(#[from] serde_json::Error),
}

impl FaultProxyConfig {
    pub fn load(
        path: impl AsRef<Path>,
    ) -> Result<(Self, FaultProxyConfigEvidence), FaultProxyConfigError> {
        let requested_path = path.as_ref();
        let metadata = fs::symlink_metadata(requested_path).map_err(|source| {
            FaultProxyConfigError::Inspect {
                path: requested_path.to_path_buf(),
                source,
            }
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(FaultProxyConfigError::InvalidPath {
                path: requested_path.to_path_buf(),
                message: "must be a regular file and not a symbolic link".to_string(),
            });
        }
        let path =
            fs::canonicalize(requested_path).map_err(|source| FaultProxyConfigError::Inspect {
                path: requested_path.to_path_buf(),
                source,
            })?;
        if metadata.len() > MAX_CONFIG_BYTES {
            return Err(FaultProxyConfigError::TooLarge {
                path,
                bytes: metadata.len(),
                maximum: MAX_CONFIG_BYTES,
            });
        }
        let bytes = fs::read(&path).map_err(|source| FaultProxyConfigError::Read {
            path: path.clone(),
            source,
        })?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err(FaultProxyConfigError::TooLarge {
                path,
                bytes: bytes.len() as u64,
                maximum: MAX_CONFIG_BYTES,
            });
        }
        let text = std::str::from_utf8(&bytes).map_err(|error| FaultProxyConfigError::Parse {
            path: path.clone(),
            message: error.to_string(),
        })?;
        let mut config: Self =
            toml::from_str(text).map_err(|error| FaultProxyConfigError::Parse {
                path: path.clone(),
                message: error.to_string(),
            })?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        config.control_socket = resolve_path(base, &config.control_socket);
        config.evidence_directory = resolve_path(base, &config.evidence_directory);
        config.validate()?;

        let effective = serde_json::to_vec(&config)?;
        Ok((
            config,
            FaultProxyConfigEvidence {
                source_path: path,
                bytes: bytes.len() as u64,
                sha256: hex_sha256(&bytes),
                effective_fingerprint: hex_sha256(&effective),
            },
        ))
    }

    pub fn validate(&self) -> Result<(), FaultProxyConfigError> {
        let mut errors = Vec::new();
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            errors.push(format!(
                "schema_version must be {CONFIG_SCHEMA_VERSION}, got {}",
                self.schema_version
            ));
        }
        let listeners = [
            ("rest_listen", self.rest_listen),
            ("public_ws_listen", self.public_ws_listen),
            ("private_ws_listen", self.private_ws_listen),
            ("order_ws_listen", self.order_ws_listen),
        ];
        for (name, address) in listeners {
            if !address.ip().is_loopback() {
                errors.push(format!("{name} must bind a loopback address"));
            }
            if address.port() == 0 {
                errors.push(format!("{name} must use an explicit nonzero port"));
            }
        }
        for left in 0..listeners.len() {
            for right in left + 1..listeners.len() {
                if listeners[left].1 == listeners[right].1 {
                    errors.push(format!(
                        "{} and {} must use distinct addresses",
                        listeners[left].0, listeners[right].0
                    ));
                }
            }
        }
        if self.control_socket.as_os_str().is_empty() {
            errors.push("control_socket must not be empty".to_string());
        }
        if self.evidence_directory.as_os_str().is_empty() {
            errors.push("evidence_directory must not be empty".to_string());
        }
        if self.request_timeout_ms == 0 || self.request_timeout_ms > 60_000 {
            errors.push("request_timeout_ms must be in 1..=60000".to_string());
        }
        if self.max_http_body_bytes == 0 || self.max_http_body_bytes > MAX_HTTP_BODY_BYTES {
            errors.push(format!(
                "max_http_body_bytes must be in 1..={MAX_HTTP_BODY_BYTES}"
            ));
        }
        if self.max_pending_faults == 0 || self.max_pending_faults > 1024 {
            errors.push("max_pending_faults must be in 1..=1024".to_string());
        }
        if self.shutdown_timeout_ms == 0 || self.shutdown_timeout_ms > 60_000 {
            errors.push("shutdown_timeout_ms must be in 1..=60000".to_string());
        }
        if self.connection_attempt_interval_ms < OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS
            || self.connection_attempt_interval_ms > 60_000
        {
            errors.push(format!(
                "connection_attempt_interval_ms must be in {OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS}..=60000"
            ));
        }
        if self.connection_attempt_pacer_path.as_os_str().is_empty() {
            errors.push("connection_attempt_pacer_path must not be empty".to_string());
        }
        if self.connection_attempt_pacer_path == self.control_socket {
            errors
                .push("connection_attempt_pacer_path must differ from control_socket".to_string());
        }
        if self.connection_attempt_pacer_path == self.evidence_directory {
            errors.push(
                "connection_attempt_pacer_path must differ from evidence_directory".to_string(),
            );
        }

        let venue = OkxVenueConfig {
            environment: TradingEnvironment::Demo,
            rest_url: self.upstream.rest_url.clone(),
            public_ws_url: self.upstream.public_ws_url.clone(),
            private_ws_url: self.upstream.private_ws_url.clone(),
            order_ws_url: None,
            enable_vip_fills_channel: false,
        };
        match venue.endpoint_region() {
            Ok(OkxEndpointRegion::DemoLoopback) => {
                errors.push("upstream endpoints must be official OKX demo endpoints".to_string());
            }
            Ok(_) => {}
            Err(upstream_errors) => errors.extend(
                upstream_errors
                    .into_iter()
                    .map(|error| format!("upstream {error}")),
            ),
        }

        if errors.is_empty() {
            Ok(())
        } else {
            errors.sort();
            errors.dedup();
            Err(FaultProxyConfigError::Invalid(errors.join("; ")))
        }
    }

    pub fn route_live_config(
        &self,
        live: &LiveConfig,
    ) -> Result<LiveConfig, FaultProxyConfigError> {
        live.ensure_valid()
            .map_err(|error| FaultProxyConfigError::Invalid(error.to_string()))?;
        if live.venue.environment != TradingEnvironment::Demo {
            return Err(FaultProxyConfigError::Invalid(
                "only a demo live config can be routed through the fault proxy".to_string(),
            ));
        }
        if live.venue.endpoint_region() == Ok(OkxEndpointRegion::DemoLoopback) {
            return Err(FaultProxyConfigError::Invalid(
                "source live config must use official OKX demo endpoints".to_string(),
            ));
        }
        if live.venue.rest_url != self.upstream.rest_url
            || live.venue.public_ws_url != self.upstream.public_ws_url
            || live.venue.private_ws_url != self.upstream.private_ws_url
            || live.venue.order_ws_url() != self.upstream.private_ws_url
        {
            return Err(FaultProxyConfigError::Invalid(
                "source live endpoint tuple must exactly match the proxy upstream tuple"
                    .to_string(),
            ));
        }
        if live.runtime.connection_attempt_interval_ms != self.connection_attempt_interval_ms
            || live.runtime.connection_attempt_pacer_path.as_ref()
                != Some(&self.connection_attempt_pacer_path)
        {
            return Err(FaultProxyConfigError::Invalid(
                "source live config and fault proxy must use the same connection-attempt interval and pacer path"
                    .to_string(),
            ));
        }
        let mut routed = live.clone();
        routed.venue.rest_url = format!("http://{}", self.rest_listen);
        routed.venue.public_ws_url = format!(
            "ws://{}{}",
            self.public_ws_listen,
            WebSocketEndpointPath::Public.as_str()
        );
        routed.venue.private_ws_url = format!(
            "ws://{}{}",
            self.private_ws_listen,
            WebSocketEndpointPath::Private.as_str()
        );
        routed.venue.order_ws_url = Some(format!(
            "ws://{}{}",
            self.order_ws_listen,
            WebSocketEndpointPath::Private.as_str()
        ));
        routed.runtime.connection_attempt_interval_ms = 0;
        routed.runtime.connection_attempt_pacer_path = None;
        routed
            .ensure_valid()
            .map_err(|error| FaultProxyConfigError::Invalid(error.to_string()))?;
        Ok(routed)
    }
}

enum WebSocketEndpointPath {
    Public,
    Private,
}

impl WebSocketEndpointPath {
    const fn as_str(&self) -> &'static str {
        match self {
            Self::Public => "/ws/v5/public",
            Self::Private => "/ws/v5/private",
        }
    }
}

fn resolve_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

const fn default_connection_attempt_interval_ms() -> u64 {
    400
}

fn default_connection_attempt_pacer_path() -> PathBuf {
    PathBuf::from(DEFAULT_OKX_CONNECTION_ATTEMPT_PACER_PATH)
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

const fn default_request_timeout_ms() -> u64 {
    10_000
}

const fn default_max_http_body_bytes() -> usize {
    1024 * 1024
}

const fn default_max_pending_faults() -> usize {
    64
}

const fn default_shutdown_timeout_ms() -> u64 {
    5_000
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> FaultProxyConfig {
        FaultProxyConfig {
            schema_version: 1,
            rest_listen: "127.0.0.1:18080".parse().unwrap(),
            public_ws_listen: "127.0.0.1:18081".parse().unwrap(),
            private_ws_listen: "127.0.0.1:18082".parse().unwrap(),
            order_ws_listen: "127.0.0.1:18083".parse().unwrap(),
            control_socket: "/tmp/reap-fault.sock".into(),
            evidence_directory: "/tmp/reap-fault-evidence".into(),
            upstream: FaultProxyUpstream {
                rest_url: "https://openapi.okx.com".to_string(),
                public_ws_url: "wss://wspap.okx.com:8443/ws/v5/public".to_string(),
                private_ws_url: "wss://wspap.okx.com:8443/ws/v5/private".to_string(),
            },
            connection_attempt_interval_ms: 400,
            connection_attempt_pacer_path: PathBuf::from(DEFAULT_OKX_CONNECTION_ATTEMPT_PACER_PATH),
            request_timeout_ms: 10_000,
            max_http_body_bytes: 1024 * 1024,
            max_pending_faults: 64,
            shutdown_timeout_ms: 5_000,
        }
    }

    #[test]
    fn accepts_bounded_loopback_to_official_demo_proxy() {
        config().validate().unwrap();
    }

    #[test]
    fn rejects_non_loopback_listener_and_non_demo_upstream() {
        let mut config = config();
        config.rest_listen = "0.0.0.0:18080".parse().unwrap();
        config.upstream.public_ws_url = "wss://ws.okx.com:8443/ws/v5/public".to_string();
        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("rest_listen must bind a loopback address"));
        assert!(error.contains("region-consistent"));
    }

    #[test]
    fn strict_load_resolves_artifact_paths() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("proxy.toml");
        fs::write(
            &path,
            r#"
schema_version = 1
rest_listen = "127.0.0.1:18080"
public_ws_listen = "127.0.0.1:18081"
private_ws_listen = "127.0.0.1:18082"
order_ws_listen = "127.0.0.1:18083"
control_socket = "run/control.sock"
evidence_directory = "evidence"

[upstream]
rest_url = "https://openapi.okx.com"
public_ws_url = "wss://wspap.okx.com:8443/ws/v5/public"
private_ws_url = "wss://wspap.okx.com:8443/ws/v5/private"
"#,
        )
        .unwrap();

        let (config, evidence) = FaultProxyConfig::load(&path).unwrap();
        assert_eq!(
            config.control_socket,
            directory.path().join("run/control.sock")
        );
        assert_eq!(config.evidence_directory, directory.path().join("evidence"));
        assert_eq!(evidence.bytes, fs::metadata(path).unwrap().len());
    }

    #[test]
    fn load_canonicalizes_source_before_resolving_artifact_paths() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("proxy.toml");
        fs::write(
            &path,
            include_bytes!("../../../examples/okx-demo-fault-proxy.toml"),
        )
        .unwrap();
        let alias_directory = directory.path().join("alias");
        fs::create_dir(&alias_directory).unwrap();
        let alias = alias_directory.join("../proxy.toml");

        let (exact_config, exact_evidence) = FaultProxyConfig::load(&path).unwrap();
        let (alias_config, alias_evidence) = FaultProxyConfig::load(&alias).unwrap();

        assert_eq!(alias_evidence, exact_evidence);
        assert_eq!(alias_config.control_socket, exact_config.control_socket);
        assert_eq!(
            alias_config.evidence_directory,
            exact_config.evidence_directory
        );
    }

    #[cfg(unix)]
    #[test]
    fn load_rejects_symbolic_link_input() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("proxy.toml");
        fs::write(
            &path,
            include_bytes!("../../../examples/okx-demo-fault-proxy.toml"),
        )
        .unwrap();
        let linked = directory.path().join("linked.toml");
        symlink(&path, &linked).unwrap();

        assert!(matches!(
            FaultProxyConfig::load(&linked),
            Err(FaultProxyConfigError::InvalidPath { .. })
        ));
    }

    #[test]
    fn routes_exact_official_demo_config_to_separate_loopback_paths() {
        let live =
            LiveConfig::from_toml(include_str!("../../../examples/live-okx-demo.toml")).unwrap();

        let routed = config().route_live_config(&live).unwrap();

        assert_eq!(routed.venue.rest_url, "http://127.0.0.1:18080");
        assert_eq!(
            routed.venue.public_ws_url,
            "ws://127.0.0.1:18081/ws/v5/public"
        );
        assert_eq!(
            routed.venue.private_ws_url,
            "ws://127.0.0.1:18082/ws/v5/private"
        );
        assert_eq!(
            routed.venue.order_ws_url(),
            "ws://127.0.0.1:18083/ws/v5/private"
        );
        assert_eq!(
            routed.venue.endpoint_region(),
            Ok(OkxEndpointRegion::DemoLoopback)
        );
        assert_eq!(routed.runtime.connection_attempt_interval_ms, 0);
        assert_eq!(routed.runtime.connection_attempt_pacer_path, None);
    }

    #[test]
    fn proxy_rejects_pacer_drift_and_control_path_collision() {
        let live =
            LiveConfig::from_toml(include_str!("../../../examples/live-okx-demo.toml")).unwrap();
        let mut proxy = config();
        proxy.connection_attempt_pacer_path = PathBuf::from("var/reap/another.pacer");
        let error = proxy.route_live_config(&live).unwrap_err().to_string();
        assert!(error.contains("same connection-attempt interval and pacer path"));

        proxy.connection_attempt_pacer_path = proxy.control_socket.clone();
        let error = proxy.validate().unwrap_err().to_string();
        assert!(error.contains("must differ from control_socket"));
    }
}
