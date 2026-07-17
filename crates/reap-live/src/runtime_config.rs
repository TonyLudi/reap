use std::path::Path;
use std::time::Duration;

use reap_telemetry::WebhookAlertConfig;
use sha2::{Digest, Sha256};

use crate::{
    AlertConfig, LiveConfig, LiveConfigError, LiveConfigFileEvidence, MAX_LIVE_CONFIG_BYTES,
    OperatorConfig,
};

/// Loads and validates a live configuration from a regular local file.
///
/// File-system inspection intentionally lives in `reap-live`, not in the
/// credential-free contracts crate.
pub fn load_live_config(path: impl AsRef<Path>) -> Result<LiveConfig, LiveConfigError> {
    load_live_config_with_evidence(path).map(|(config, _)| config)
}

/// Loads a live configuration and records the exact canonical file evidence.
pub fn load_live_config_with_evidence(
    path: impl AsRef<Path>,
) -> Result<(LiveConfig, LiveConfigFileEvidence), LiveConfigError> {
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
    let canonical = std::fs::canonicalize(path).map_err(|error| LiveConfigError::InvalidPath {
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
    let config = LiveConfig::from_toml(text)?;
    let evidence = LiveConfigFileEvidence {
        source_path: canonical,
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(&bytes)),
    };
    Ok((config, evidence))
}

/// Compatibility extension for callers that previously loaded through
/// `LiveConfig` directly. New code should use [`load_live_config`] or
/// [`load_live_config_with_evidence`] so host inspection is explicit.
pub trait LiveConfigRuntimeExt: Sized {
    fn load(path: impl AsRef<Path>) -> Result<Self, LiveConfigError>;

    fn load_with_evidence(
        path: impl AsRef<Path>,
    ) -> Result<(Self, LiveConfigFileEvidence), LiveConfigError>;
}

impl LiveConfigRuntimeExt for LiveConfig {
    fn load(path: impl AsRef<Path>) -> Result<Self, LiveConfigError> {
        load_live_config(path)
    }

    fn load_with_evidence(
        path: impl AsRef<Path>,
    ) -> Result<(Self, LiveConfigFileEvidence), LiveConfigError> {
        load_live_config_with_evidence(path)
    }
}

/// Compatibility extension for the runtime-owned operator secret lookup.
pub trait OperatorConfigRuntimeExt {
    fn secret_from_env(&self) -> Result<Option<Vec<u8>>, LiveConfigError>;
}

impl OperatorConfigRuntimeExt for OperatorConfig {
    fn secret_from_env(&self) -> Result<Option<Vec<u8>>, LiveConfigError> {
        operator_secret_from_env(self)
    }
}

/// Compatibility extension for the runtime-owned alert transport lookup.
pub trait AlertConfigRuntimeExt {
    fn webhook_from_env(&self) -> Result<Option<WebhookAlertConfig>, LiveConfigError>;
}

impl AlertConfigRuntimeExt for AlertConfig {
    fn webhook_from_env(&self) -> Result<Option<WebhookAlertConfig>, LiveConfigError> {
        alert_webhook_from_env(self)
    }
}

pub(crate) fn operator_secret_from_env(
    config: &OperatorConfig,
) -> Result<Option<Vec<u8>>, LiveConfigError> {
    if !config.enabled {
        return Ok(None);
    }
    let secret =
        std::env::var(&config.token_env).map_err(|_| LiveConfigError::MissingOperatorToken {
            name: config.token_env.clone(),
        })?;
    if secret.len() < 32 {
        return Err(LiveConfigError::OperatorTokenTooShort {
            name: config.token_env.clone(),
        });
    }
    Ok(Some(secret.into_bytes()))
}

pub(crate) fn alert_webhook_from_env(
    config: &AlertConfig,
) -> Result<Option<WebhookAlertConfig>, LiveConfigError> {
    if !config.enabled {
        return Ok(None);
    }
    let endpoint =
        std::env::var(&config.endpoint_env).map_err(|_| LiveConfigError::MissingAlertEndpoint {
            name: config.endpoint_env.clone(),
        })?;
    let bearer_token = config
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
        channel_capacity: config.channel_capacity,
        failure_channel_capacity: config.failure_channel_capacity,
        request_timeout: Duration::from_millis(config.request_timeout_ms),
        connect_timeout: Duration::from_millis(config.connect_timeout_ms),
        max_attempts: config.max_attempts,
        retry_backoff: Duration::from_millis(config.retry_backoff_ms),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_runtime_integrations_do_not_read_environment() {
        assert!(
            operator_secret_from_env(&OperatorConfig::default())
                .unwrap()
                .is_none()
        );
        assert!(
            alert_webhook_from_env(&AlertConfig::default())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn config_file_loading_retains_canonical_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("live.toml");
        let bytes = include_bytes!("../../../examples/live-okx-demo.toml");
        std::fs::write(&path, bytes).unwrap();

        let (config, evidence) = load_live_config_with_evidence(&path).unwrap();

        assert_eq!(config.venue.environment, crate::TradingEnvironment::Demo);
        assert_eq!(evidence.source_path, path.canonicalize().unwrap());
        assert_eq!(evidence.bytes, bytes.len() as u64);
        assert_eq!(
            evidence.sha256,
            format!("{:x}", Sha256::digest(bytes.as_slice()))
        );
    }

    #[cfg(unix)]
    #[test]
    fn config_file_loading_rejects_symbolic_links() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("live.toml");
        let alias = directory.path().join("alias.toml");
        std::fs::write(
            &path,
            include_bytes!("../../../examples/live-okx-demo.toml"),
        )
        .unwrap();
        symlink(&path, &alias).unwrap();

        assert!(matches!(
            load_live_config(&alias),
            Err(LiveConfigError::InvalidPath { .. })
        ));
    }

    #[test]
    fn config_file_loading_preserves_bounded_input_errors() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing.toml");
        assert!(matches!(
            load_live_config(&missing),
            Err(LiveConfigError::InvalidPath { path, .. }) if path == missing
        ));

        let invalid_utf8 = directory.path().join("invalid-utf8.toml");
        std::fs::write(&invalid_utf8, [0xff]).unwrap();
        assert!(matches!(
            load_live_config(&invalid_utf8),
            Err(LiveConfigError::Utf8 { .. })
        ));

        let too_large = directory.path().join("too-large.toml");
        let file = std::fs::File::create(&too_large).unwrap();
        file.set_len(MAX_LIVE_CONFIG_BYTES + 1).unwrap();
        assert!(matches!(
            load_live_config(&too_large),
            Err(LiveConfigError::TooLarge {
                actual,
                limit: MAX_LIVE_CONFIG_BYTES,
                ..
            }) if actual == MAX_LIVE_CONFIG_BYTES + 1
        ));
    }
}
