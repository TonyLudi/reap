use std::{
    fs::OpenOptions,
    io::Read as _,
    path::{Path, PathBuf},
    str::FromStr as _,
};

use reap_pm_core::{
    EvmAddress, MAX_REQUIRED_SPENDERS, PmAssetId, PmChainId, PmConditionId, PmMarketId,
    PmMarketLifecycle, PmMarketMetadata, PmOutcomeLabel, PmOutcomeMetadata, PmQuantity,
    PmSpenderDomain, PmSpenderRequirement, PmTick, PmTokenId, U256,
};
use reap_polymarket_wire::PmWireScope;
use reap_telemetry::sha256_bytes;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

pub const PM_READ_ONLY_CONFIG_SCHEMA_VERSION: u32 = 1;
pub const MAX_PM_READ_ONLY_CONFIG_BYTES: u64 = 64 * 1_024;

const MAX_CREDENTIAL_SLOT_BYTES: usize = 128;
const MAX_CREDENTIAL_ENTRY_BYTES: usize = 128;
const MAX_TIMEOUT_MS: u64 = 60_000;
const MIN_USER_STREAM_DWELL_MS: u64 = 12_000;
const MAX_USER_STREAM_DWELL_MS: u64 = 300_000;
const MAX_RECONNECT_ATTEMPTS: u8 = 16;
const MAX_EVENT_CHANNEL_CAPACITY: usize = 1_024;
const CONFIG_FINGERPRINT_DOMAIN: &[u8] = b"reap.pm.read-only-smoke.config.v1\0";

const POLYGON_CHAIN_ID: u64 = 137;
const PUSD: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
const CONDITIONAL_TOKENS: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
const STANDARD_EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";
const NEGATIVE_RISK_EXCHANGE: &str = "0xe2222d279d744050d28e00520010520000310F59";

/// Strict, secret-free configuration for one production read-only smoke.
///
/// Credential fields are entry names inside an independently supplied,
/// protected credential directory. They are never environment-variable names,
/// paths, or secret values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmReadOnlySmokeConfig {
    pub schema_version: u32,
    pub credential_slot_id: String,
    pub signer_address: String,
    pub funder_address: String,
    pub chain_id: u64,
    pub signature_type: u8,
    pub condition_id: String,
    pub market_id: String,
    pub token_id: String,
    pub outcome: String,
    pub tick: String,
    pub minimum_order_size: String,
    pub negative_risk: bool,
    pub connect_timeout_ms: u64,
    pub request_timeout_ms: u64,
    pub user_stream_dwell_ms: u64,
    pub user_stream_idle_timeout_ms: u64,
    pub user_stream_pong_timeout_ms: u64,
    pub user_stream_max_reconnect_attempts: u8,
    pub user_stream_reconnect_backoff_ms: u64,
    pub user_stream_event_channel_capacity: usize,
    pub api_key_file: String,
    pub secret_file: String,
    pub passphrase_file: String,
}

impl PmReadOnlySmokeConfig {
    pub fn validate(&self) -> Result<(), PmReadOnlySmokeConfigError> {
        if self.schema_version != PM_READ_ONLY_CONFIG_SCHEMA_VERSION {
            return Err(invalid("unsupported read-only config schema version"));
        }
        validate_slot(&self.credential_slot_id)?;
        let signer = self.signer()?;
        let funder = self.funder()?;
        if signer != funder {
            return Err(invalid(
                "the fixed read-only profile requires signer=funder",
            ));
        }
        if self.chain_id != POLYGON_CHAIN_ID {
            return Err(invalid(
                "the fixed read-only profile requires Polygon chain 137",
            ));
        }
        if self.signature_type != 0 {
            return Err(invalid(
                "the fixed read-only profile requires signature_type=0",
            ));
        }
        let metadata = self.expected_metadata()?;
        let _ = reap_pm_core::PmGoalFTradingDomain::from_metadata(metadata)
            .map_err(|_| invalid("configured metadata is outside the fixed Goal-F domain"))?;
        if self.connect_timeout_ms == 0
            || self.connect_timeout_ms > MAX_TIMEOUT_MS
            || self.request_timeout_ms == 0
            || self.request_timeout_ms > MAX_TIMEOUT_MS
        {
            return Err(invalid("HTTP timeouts must be within 1..=60000 ms"));
        }
        if !(MIN_USER_STREAM_DWELL_MS..=MAX_USER_STREAM_DWELL_MS)
            .contains(&self.user_stream_dwell_ms)
        {
            return Err(invalid(
                "user-stream dwell must be within 12000..=300000 ms",
            ));
        }
        if self.user_stream_idle_timeout_ms <= 10_000
            || self.user_stream_idle_timeout_ms > MAX_TIMEOUT_MS
            || self.user_stream_pong_timeout_ms == 0
            || self.user_stream_pong_timeout_ms >= 10_000
            || self.user_stream_pong_timeout_ms >= self.user_stream_idle_timeout_ms
        {
            return Err(invalid("user-stream heartbeat/idle bounds are invalid"));
        }
        if self.user_stream_max_reconnect_attempts > MAX_RECONNECT_ATTEMPTS
            || self.user_stream_reconnect_backoff_ms == 0
            || self.user_stream_reconnect_backoff_ms > MAX_TIMEOUT_MS
            || self.user_stream_event_channel_capacity == 0
            || self.user_stream_event_channel_capacity > MAX_EVENT_CHANNEL_CAPACITY
        {
            return Err(invalid("user-stream reconnect/channel bounds are invalid"));
        }
        validate_credential_entry(&self.api_key_file)?;
        validate_credential_entry(&self.secret_file)?;
        validate_credential_entry(&self.passphrase_file)?;
        if self.api_key_file == self.secret_file
            || self.api_key_file == self.passphrase_file
            || self.secret_file == self.passphrase_file
        {
            return Err(invalid("credential entry names must be distinct"));
        }
        Ok(())
    }

    pub fn signer(&self) -> Result<EvmAddress, PmReadOnlySmokeConfigError> {
        EvmAddress::parse(&self.signer_address)
            .map_err(|_| invalid("signer_address is not a canonical EVM address"))
    }

    pub fn funder(&self) -> Result<EvmAddress, PmReadOnlySmokeConfigError> {
        EvmAddress::parse(&self.funder_address)
            .map_err(|_| invalid("funder_address is not a canonical EVM address"))
    }

    pub fn wire_scope(&self) -> Result<PmWireScope, PmReadOnlySmokeConfigError> {
        let condition = PmConditionId::parse(&self.condition_id)
            .map_err(|_| invalid("condition_id is not canonical"))?;
        let market = PmMarketId::parse(&self.market_id)
            .map_err(|_| invalid("market_id is not canonical"))?;
        let token_units = U256::from_str(&self.token_id)
            .map_err(|_| invalid("token_id is not canonical decimal"))?;
        if token_units.to_string() != self.token_id {
            return Err(invalid("token_id is not canonical decimal"));
        }
        let token = PmTokenId::new(token_units).map_err(|_| invalid("token_id must be nonzero"))?;
        Ok(PmWireScope::new(condition, market, token))
    }

    pub fn expected_metadata(&self) -> Result<PmMarketMetadata, PmReadOnlySmokeConfigError> {
        let scope = self.wire_scope()?;
        let label =
            PmOutcomeLabel::new(&self.outcome).map_err(|_| invalid("outcome label is invalid"))?;
        let tick = PmTick::parse_decimal(&self.tick)
            .map_err(|_| invalid("tick is not canonical decimal"))?;
        let minimum_order_size = PmQuantity::parse_decimal(&self.minimum_order_size)
            .map_err(|_| invalid("minimum_order_size is not canonical decimal"))?;
        let chain = PmChainId::new(POLYGON_CHAIN_ID)
            .map_err(|_| invalid("fixed Polygon chain is invalid"))?;
        let exchange = EvmAddress::parse(if self.negative_risk {
            NEGATIVE_RISK_EXCHANGE
        } else {
            STANDARD_EXCHANGE
        })
        .map_err(|_| invalid("fixed exchange address is invalid"))?;
        let domain = if self.negative_risk {
            PmSpenderDomain::NegativeRisk
        } else {
            PmSpenderDomain::Standard
        };
        let collateral = PmAssetId::collateral(
            EvmAddress::parse(PUSD).map_err(|_| invalid("fixed collateral is invalid"))?,
        );
        let outcome_asset = PmAssetId::outcome(
            EvmAddress::parse(CONDITIONAL_TOKENS)
                .map_err(|_| invalid("fixed conditional-token contract is invalid"))?,
            scope.token(),
        );
        let mut spenders = [None; MAX_REQUIRED_SPENDERS];
        spenders[0] = Some(PmSpenderRequirement::new(
            chain, exchange, domain, collateral,
        ));
        spenders[1] = Some(PmSpenderRequirement::new(
            chain,
            exchange,
            domain,
            outcome_asset,
        ));
        PmMarketMetadata::new(
            scope.condition(),
            scope.market(),
            PmOutcomeMetadata::new(scope.token(), label),
            PmMarketLifecycle::new(true, false, false, true, true),
            tick,
            minimum_order_size,
            self.negative_risk,
            chain,
            exchange,
            spenders,
            2,
        )
        .map_err(|_| invalid("configured market metadata violates the fixed profile"))
    }

    pub fn fingerprint(&self) -> Result<String, PmReadOnlySmokeConfigError> {
        self.validate()?;
        let canonical = serde_json::to_vec(self)
            .map_err(|_| invalid("read-only config could not be canonicalized"))?;
        let mut hasher = Sha256::new();
        hasher.update(CONFIG_FINGERPRINT_DOMAIN);
        hasher.update(canonical);
        Ok(hex_lower(&hasher.finalize()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmReadOnlyConfigEvidence {
    pub canonical_bytes: u64,
    pub canonical_sha256: String,
    pub canonical_toml: String,
}

#[derive(Debug, Error)]
pub enum PmReadOnlySmokeConfigError {
    #[error("invalid read-only config path {path}: {message}")]
    InvalidPath { path: PathBuf, message: String },
    #[error("read-only config {path} is {actual} bytes; limit is {limit}")]
    TooLarge {
        path: PathBuf,
        actual: u64,
        limit: u64,
    },
    #[error("failed to read read-only config {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("read-only config is not valid UTF-8")]
    NonUtf8,
    #[error("failed to parse the read-only config")]
    Parse,
    #[error("invalid read-only config: {0}")]
    Invalid(&'static str),
}

pub fn load_pm_read_only_smoke_config_path(
    path: impl AsRef<Path>,
) -> Result<(PmReadOnlySmokeConfig, PmReadOnlyConfigEvidence), PmReadOnlySmokeConfigError> {
    let path = path.as_ref();
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    }
    let mut file = options
        .open(path)
        .map_err(|source| PmReadOnlySmokeConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    let metadata = file
        .metadata()
        .map_err(|source| PmReadOnlySmokeConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    if !metadata.is_file() {
        return Err(PmReadOnlySmokeConfigError::InvalidPath {
            path: path.to_path_buf(),
            message: "must be a regular file and not a symbolic link".to_string(),
        });
    }
    if metadata.len() > MAX_PM_READ_ONLY_CONFIG_BYTES {
        return Err(PmReadOnlySmokeConfigError::TooLarge {
            path: path.to_path_buf(),
            actual: metadata.len(),
            limit: MAX_PM_READ_ONLY_CONFIG_BYTES,
        });
    }
    let mut bytes = Vec::with_capacity(
        usize::try_from(metadata.len())
            .unwrap_or(MAX_PM_READ_ONLY_CONFIG_BYTES as usize)
            .min(MAX_PM_READ_ONLY_CONFIG_BYTES as usize),
    );
    file.by_ref()
        .take(MAX_PM_READ_ONLY_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| PmReadOnlySmokeConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() as u64 > MAX_PM_READ_ONLY_CONFIG_BYTES {
        return Err(PmReadOnlySmokeConfigError::TooLarge {
            path: path.to_path_buf(),
            actual: bytes.len() as u64,
            limit: MAX_PM_READ_ONLY_CONFIG_BYTES,
        });
    }
    let post_read = file
        .metadata()
        .map_err(|source| PmReadOnlySmokeConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    if !opened_regular_file_metadata_is_stable(&metadata, &post_read, bytes.len() as u64) {
        return Err(PmReadOnlySmokeConfigError::InvalidPath {
            path: path.to_path_buf(),
            message: "opened file changed while it was read".to_string(),
        });
    }
    let text = String::from_utf8(bytes).map_err(|_| PmReadOnlySmokeConfigError::NonUtf8)?;
    let config: PmReadOnlySmokeConfig =
        toml::from_str(&text).map_err(|_| PmReadOnlySmokeConfigError::Parse)?;
    config.validate()?;
    let canonical_toml = toml::to_string(&config)
        .map_err(|_| invalid("read-only config could not be canonicalized"))?;
    let evidence = PmReadOnlyConfigEvidence {
        canonical_bytes: canonical_toml.len() as u64,
        canonical_sha256: sha256_bytes(canonical_toml.as_bytes()),
        canonical_toml,
    };
    Ok((config, evidence))
}

pub(crate) fn opened_regular_file_metadata_is_stable(
    before: &std::fs::Metadata,
    after: &std::fs::Metadata,
    bytes_read: u64,
) -> bool {
    if !before.is_file()
        || !after.is_file()
        || before.len() != bytes_read
        || after.len() != bytes_read
    {
        return false;
    }

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt as _;

        before.dev() == after.dev()
            && before.ino() == after.ino()
            && before.uid() == after.uid()
            && before.gid() == after.gid()
            && before.mode() == after.mode()
            && before.nlink() == after.nlink()
            && before.mtime() == after.mtime()
            && before.mtime_nsec() == after.mtime_nsec()
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec()
    }

    #[cfg(not(target_os = "linux"))]
    true
}

fn validate_slot(value: &str) -> Result<(), PmReadOnlySmokeConfigError> {
    if value.is_empty()
        || value.len() > MAX_CREDENTIAL_SLOT_BYTES
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
    {
        return Err(invalid("credential_slot_id is invalid"));
    }
    Ok(())
}

fn validate_credential_entry(value: &str) -> Result<(), PmReadOnlySmokeConfigError> {
    let path = Path::new(value);
    if value.is_empty()
        || value.len() > MAX_CREDENTIAL_ENTRY_BYTES
        || path.components().count() != 1
        || value == "."
        || value == ".."
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(invalid("credential file entry name is invalid"));
    }
    Ok(())
}

fn invalid(message: &'static str) -> PmReadOnlySmokeConfigError {
    PmReadOnlySmokeConfigError::Invalid(message)
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing hex to String cannot fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> PmReadOnlySmokeConfig {
        PmReadOnlySmokeConfig {
            schema_version: 1,
            credential_slot_id: "pm-read-v1".into(),
            signer_address: "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266".into(),
            funder_address: "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266".into(),
            chain_id: 137,
            signature_type: 0,
            condition_id: format!("0x{}", "11".repeat(32)),
            market_id: format!("0x{}", "22".repeat(32)),
            token_id: "1234".into(),
            outcome: "Yes".into(),
            tick: "0.01".into(),
            minimum_order_size: "5".into(),
            negative_risk: false,
            connect_timeout_ms: 5_000,
            request_timeout_ms: 15_000,
            user_stream_dwell_ms: 15_000,
            user_stream_idle_timeout_ms: 30_000,
            user_stream_pong_timeout_ms: 5_000,
            user_stream_max_reconnect_attempts: 1,
            user_stream_reconnect_backoff_ms: 500,
            user_stream_event_channel_capacity: 64,
            api_key_file: "api-key".into(),
            secret_file: "secret".into(),
            passphrase_file: "passphrase".into(),
        }
    }

    #[test]
    fn exact_fixed_profile_is_valid_and_fingerprinted() {
        let config = valid();
        config.validate().unwrap();
        assert_eq!(config.signer().unwrap(), config.funder().unwrap());
        assert_eq!(
            config.wire_scope().unwrap().token().units(),
            U256::from_u64(1234)
        );
        assert_eq!(
            config.expected_metadata().unwrap().required_spender_count(),
            2
        );
        assert_eq!(config.fingerprint().unwrap().len(), 64);
    }

    #[test]
    fn checked_in_operator_template_is_strict_and_valid() {
        let config: PmReadOnlySmokeConfig =
            toml::from_str(include_str!("../../../examples/pm-read-only-smoke.toml"))
                .expect("checked-in read-only template must parse");
        config
            .validate()
            .expect("checked-in read-only template must satisfy the closed profile");
    }

    #[test]
    fn config_evidence_canonicalization_discards_comments_and_source_paths() {
        const COMMENT_CANARY: &str = "secret-comment-canary-must-not-persist";
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("operator-profile.toml");
        let mut source = toml::to_string(&valid()).unwrap();
        source.push_str(&format!("\n# {COMMENT_CANARY}\n"));
        std::fs::write(&path, source).unwrap();

        let (_, evidence) = load_pm_read_only_smoke_config_path(&path).unwrap();
        assert!(!evidence.canonical_toml.contains(COMMENT_CANARY));
        assert!(!evidence.canonical_toml.contains(path.to_str().unwrap()));
        assert_eq!(
            evidence.canonical_bytes,
            evidence.canonical_toml.len() as u64
        );
        assert_eq!(
            evidence.canonical_sha256,
            sha256_bytes(evidence.canonical_toml.as_bytes())
        );

        let malformed = directory.path().join("malformed.toml");
        std::fs::write(
            &malformed,
            format!("schema_version = \"{COMMENT_CANARY}\"\ninvalid = ["),
        )
        .unwrap();
        let rendered = load_pm_read_only_smoke_config_path(&malformed)
            .unwrap_err()
            .to_string();
        assert!(!rendered.contains(COMMENT_CANARY));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn config_loader_rejects_a_symlink_before_reading_its_target() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("config.toml");
        let link = directory.path().join("config-link.toml");
        std::fs::write(&target, toml::to_string(&valid()).unwrap()).unwrap();
        symlink(&target, &link).unwrap();

        assert!(load_pm_read_only_smoke_config_path(&link).is_err());
    }

    #[test]
    fn mutation_adjacent_and_ambiguous_configurations_are_rejected() {
        for mutate in [
            |value: &mut PmReadOnlySmokeConfig| value.signature_type = 1,
            |value: &mut PmReadOnlySmokeConfig| value.chain_id = 1,
            |value: &mut PmReadOnlySmokeConfig| {
                value.funder_address = format!("0x{}", "33".repeat(20))
            },
            |value: &mut PmReadOnlySmokeConfig| value.api_key_file = "../key".into(),
            |value: &mut PmReadOnlySmokeConfig| value.secret_file = value.api_key_file.clone(),
        ] {
            let mut config = valid();
            mutate(&mut config);
            assert!(config.validate().is_err());
        }
    }
}
