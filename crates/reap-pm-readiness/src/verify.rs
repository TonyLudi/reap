use std::{
    collections::BTreeSet,
    fs::OpenOptions,
    io::Read as _,
    path::{Path, PathBuf},
    str::FromStr as _,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use reap_pm_core::{
    EvmAddress, PmAssetId, PmBookQuantity, PmFillId, PmGoalFTradingDomain, PmPrice, PmQuantity,
    PmVenueOrderId, U256,
};
use reap_polymarket_live_adapter::{
    MAX_PM_AUTHENTICATED_CUT_PAGES, MAX_PM_AUTHENTICATED_ORDER_ROWS,
    MAX_PM_AUTHENTICATED_TRADE_ROWS,
};
use reap_polymarket_wire::{
    MAX_PM_LIVE_PAGE_ITEMS, MAX_PUBLIC_REST_BODY_BYTES, PmClobV2RequestScope,
    parse_live_clob_market_lifecycle, parse_live_clob_v2_metadata,
};
use reap_telemetry::current_executable_sha256;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    PM_READ_ONLY_CONFIG_SCHEMA_VERSION, PmReadOnlySmokeConfig, PmReadOnlySmokeConfigError,
    config::opened_regular_file_metadata_is_stable,
    load_pm_read_only_smoke_config_path,
    schema::{
        PM_READ_ONLY_ARTIFACT_SCHEMA_VERSION, PM_READ_ONLY_CONTRACT_VERSION, PM_READ_ONLY_COVERAGE,
        PM_READ_ONLY_PRODUCT, PmReadOnlyAccountEvidence, PmReadOnlyAllowanceEvidence,
        PmReadOnlyCollectionFailureEvidence, PmReadOnlyMetadataEvidence, PmReadOnlyOrderEvidence,
        PmReadOnlyReconciliationEvidence, PmReadOnlySmokeArtifact, PmReadOnlySmokeSummary,
        PmReadOnlyTeardownEvidence, PmReadOnlyTradeEvidence, PmReadOnlyTradeMakerEvidence,
        PmReadOnlyUserStreamEvidence,
    },
};

pub const MAX_PM_READ_ONLY_ARTIFACT_BYTES: u64 = 64 * 1_024 * 1_024;

const MAX_SMOKE_DURATION_MS: u64 = 30 * 60 * 1_000;
const MIN_SUPPORTED_UNIX_MS: u64 = 1_577_836_800_000; // 2020-01-01 UTC
const MAX_SUPPORTED_UNIX_MS: u64 = 32_503_680_000_000; // 3000-01-01 UTC
const MAX_HOST_NAME_BYTES: usize = 255;
const MAX_SHORT_FIELD_BYTES: usize = 128;
const MAX_STATUS_FIELD_BYTES: usize = 512;
const MAX_MAKER_ROWS_PER_TRADE: usize = 64;

const BINARY_NAME: &str = "reap-pm-readiness";
const BINARY_DOMAIN: &[u8] = b"reap.pm.read-only-smoke.binary.v1\0";
const HOST_DOMAIN: &[u8] = b"reap.pm.read-only-smoke.host.v1\0";
const SLOT_DOMAIN: &[u8] = b"reap.pm.read-only-smoke.credential-slot-nonsecret.v1\0";
const LIFECYCLE_DOMAIN: &[u8] = b"reap.pm.read-only-smoke.lifecycle-metadata.v1\0";
const TRADING_DOMAIN: &[u8] = b"reap.pm.read-only-smoke.trading-metadata.v1\0";
const JOINED_METADATA_DOMAIN: &[u8] = b"reap.pm.read-only-smoke.joined-metadata.v1\0";
const ACCOUNT_DOMAIN: &[u8] = b"reap.pm.read-only-smoke.account.v1\0";
const OPEN_ORDERS_DOMAIN: &[u8] = b"reap.pm.read-only-smoke.open-orders.v1\0";
const TRADES_DOMAIN: &[u8] = b"reap.pm.read-only-smoke.trades.v1\0";
const USER_STREAM_DOMAIN: &[u8] = b"reap.pm.read-only-smoke.user-stream.v1\0";
const ARTIFACT_DOMAIN: &[u8] = b"reap.pm.read-only-smoke.artifact.v1\0";

pub const PM_READ_ONLY_LIMITATIONS: [&str; 6] = [
    "observations are sequential point-in-time cuts, not one atomic venue snapshot",
    "raw authenticated HTTP responses and user WebSocket frames are intentionally not retained",
    "credential-owner binding is collector-validated and retained only as redacted counts",
    "the bounded user-stream dwell does not qualify reconnect, latency, capacity, or sustained operation",
    "offline verification checks public recomputable consistency and requires an independently trusted collection chain of custody",
    "this read-only artifact grants no mutation, strategy, risk, settlement, demo, or production authority",
];

const FAILURE_STAGES: [&str; 6] = [
    "public_metadata",
    "authenticated_account",
    "open_orders",
    "trades",
    "user_stream",
    "teardown",
];

#[derive(Debug, Error)]
pub enum PmReadOnlySmokeVerificationError {
    #[error("invalid read-only artifact path {path}: {message}")]
    InvalidPath { path: PathBuf, message: String },
    #[error("read-only artifact {path} is {actual} bytes; limit is {limit}")]
    TooLarge {
        path: PathBuf,
        actual: u64,
        limit: u64,
    },
    #[error("failed to read read-only artifact {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("read-only artifact exceeds its 64 MiB byte bound")]
    ArtifactTooLarge,
    #[error("read-only artifact is malformed or has an unknown field")]
    MalformedArtifact,
    #[error("invalid read-only artifact: {0}")]
    Invalid(&'static str),
    #[error("embedded read-only config is invalid: {0}")]
    Config(#[from] PmReadOnlySmokeConfigError),
    #[error("read-only smoke artifact is internally valid but did not pass every gate")]
    NotPassing,
    #[error("the artifact does not match the separately reviewed canonical config")]
    ReviewedConfigMismatch,
    #[error("the artifact's declared binary digest does not match the current verifier executable")]
    DeclaredBinaryMismatch,
    #[error("the current verifier executable digest is unavailable")]
    VerifierDigestUnavailable,
}

impl PmReadOnlyCollectionFailureEvidence {
    pub fn new(
        stage: impl Into<String>,
        kind: impl Into<String>,
        occurred_unix_ms: u64,
    ) -> Result<Self, PmReadOnlySmokeVerificationError> {
        let evidence = Self {
            stage: stage.into(),
            kind: kind.into(),
            occurred_unix_ms,
        };
        validate_failure(&evidence, 0, u64::MAX)?;
        Ok(evidence)
    }
}

impl PmReadOnlyMetadataEvidence {
    /// Parse, bind, hash, and project the two public authoritative responses.
    pub fn from_public_bodies(
        config: &PmReadOnlySmokeConfig,
        market_body: &[u8],
        clob_body: &[u8],
    ) -> Result<Self, PmReadOnlySmokeVerificationError> {
        config.validate()?;
        derive_metadata(config, market_body, clob_body)
    }
}

impl PmReadOnlySmokeArtifact {
    /// Build an artifact from collector-owned observations and derive every
    /// hash, count, fixed authorization value, limitation, and summary gate.
    #[allow(clippy::too_many_arguments)]
    pub fn from_collected(
        binary_sha256: String,
        host_name: String,
        host_os: String,
        host_arch: String,
        config: &PmReadOnlySmokeConfig,
        config_evidence: crate::PmReadOnlyConfigEvidence,
        started_unix_ms: u64,
        finished_unix_ms: u64,
        collection_failure: Option<PmReadOnlyCollectionFailureEvidence>,
        metadata: Option<PmReadOnlyMetadataEvidence>,
        account: Option<PmReadOnlyAccountEvidence>,
        reconciliation: Option<PmReadOnlyReconciliationEvidence>,
        user_stream: Option<PmReadOnlyUserStreamEvidence>,
        teardown: PmReadOnlyTeardownEvidence,
    ) -> Result<Self, PmReadOnlySmokeVerificationError> {
        Self::from_collected_with_mode(
            binary_sha256,
            host_name,
            host_os,
            host_arch,
            config,
            config_evidence,
            started_unix_ms,
            finished_unix_ms,
            collection_failure,
            metadata,
            account,
            reconciliation,
            user_stream,
            teardown,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_collected_draft(
        binary_sha256: String,
        host_name: String,
        host_os: String,
        host_arch: String,
        config: &PmReadOnlySmokeConfig,
        config_evidence: crate::PmReadOnlyConfigEvidence,
        started_unix_ms: u64,
        finished_unix_ms: u64,
        metadata: Option<PmReadOnlyMetadataEvidence>,
        account: Option<PmReadOnlyAccountEvidence>,
        reconciliation: Option<PmReadOnlyReconciliationEvidence>,
        user_stream: Option<PmReadOnlyUserStreamEvidence>,
        teardown: PmReadOnlyTeardownEvidence,
    ) -> Result<Self, PmReadOnlySmokeVerificationError> {
        Self::from_collected_with_mode(
            binary_sha256,
            host_name,
            host_os,
            host_arch,
            config,
            config_evidence,
            started_unix_ms,
            finished_unix_ms,
            None,
            metadata,
            account,
            reconciliation,
            user_stream,
            teardown,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_collected_with_mode(
        binary_sha256: String,
        host_name: String,
        host_os: String,
        host_arch: String,
        config: &PmReadOnlySmokeConfig,
        config_evidence: crate::PmReadOnlyConfigEvidence,
        started_unix_ms: u64,
        finished_unix_ms: u64,
        collection_failure: Option<PmReadOnlyCollectionFailureEvidence>,
        metadata: Option<PmReadOnlyMetadataEvidence>,
        account: Option<PmReadOnlyAccountEvidence>,
        reconciliation: Option<PmReadOnlyReconciliationEvidence>,
        user_stream: Option<PmReadOnlyUserStreamEvidence>,
        teardown: PmReadOnlyTeardownEvidence,
        enforce_failure_consistency: bool,
    ) -> Result<Self, PmReadOnlySmokeVerificationError> {
        let mut artifact = Self {
            schema_version: PM_READ_ONLY_ARTIFACT_SCHEMA_VERSION,
            product: PM_READ_ONLY_PRODUCT.to_string(),
            coverage: PM_READ_ONLY_COVERAGE.to_string(),
            contract_version: PM_READ_ONLY_CONTRACT_VERSION.to_string(),
            binary_name: BINARY_NAME.to_string(),
            binary_version: env!("CARGO_PKG_VERSION").to_string(),
            binary_sha256,
            binary_fingerprint_sha256: String::new(),
            host_name,
            host_os,
            host_arch,
            host_fingerprint_sha256: String::new(),
            config: config_evidence,
            config_fingerprint_sha256: String::new(),
            credential_slot_nonsecret_fingerprint_sha256: String::new(),
            started_unix_ms,
            finished_unix_ms,
            production_order_entry_authorized: false,
            mutation_roles_constructed: false,
            mutation_requests: 0,
            collection_failure,
            metadata,
            account,
            reconciliation,
            user_stream,
            teardown,
            limitations: Vec::new(),
            summary: empty_summary(),
            evidence_fingerprint_sha256: String::new(),
        };
        artifact.finalize_with_mode(config, enforce_failure_consistency)?;
        Ok(artifact)
    }

    /// Canonicalize collector projections and derive all redundant evidence.
    pub fn finalize(
        &mut self,
        config: &PmReadOnlySmokeConfig,
    ) -> Result<(), PmReadOnlySmokeVerificationError> {
        self.finalize_with_mode(config, true)
    }

    fn finalize_with_mode(
        &mut self,
        config: &PmReadOnlySmokeConfig,
        enforce_failure_consistency: bool,
    ) -> Result<(), PmReadOnlySmokeVerificationError> {
        config.validate()?;
        self.schema_version = PM_READ_ONLY_ARTIFACT_SCHEMA_VERSION;
        self.product = PM_READ_ONLY_PRODUCT.to_string();
        self.coverage = PM_READ_ONLY_COVERAGE.to_string();
        self.contract_version = PM_READ_ONLY_CONTRACT_VERSION.to_string();
        self.binary_name = BINARY_NAME.to_string();
        self.binary_version = env!("CARGO_PKG_VERSION").to_string();
        if self.production_order_entry_authorized
            || self.mutation_roles_constructed
            || self.mutation_requests != 0
            || self.teardown.mutation_roles_constructed
            || self.teardown.mutation_requests != 0
        {
            return Err(invalid(
                "read-only evidence construction received mutation authority or activity",
            ));
        }

        if let Some(metadata) = &self.metadata {
            let market = decode_public_body(
                &metadata.market_body_base64,
                metadata.market_body_bytes,
                &metadata.market_body_sha256,
            )?;
            let clob = decode_public_body(
                &metadata.clob_body_base64,
                metadata.clob_body_bytes,
                &metadata.clob_body_sha256,
            )?;
            self.metadata = Some(derive_metadata(config, &market, &clob)?);
        }
        if let Some(account) = &mut self.account {
            canonicalize_account(account)?;
            account.authenticated_response_count = 2;
            account.position_token_id = config.wire_scope()?.token().units().to_string();
            account
                .position_balance
                .clone_from(&account.outcome_balance);
            account.position_available = self.metadata.as_ref().is_some_and(metadata_is_tradeable);
            account.allowance_count = account.allowances.len() as u64;
            account.canonical_sha256 = account_fingerprint(account)?;
        }
        if let Some(reconciliation) = &mut self.reconciliation {
            canonicalize_reconciliation(reconciliation)?;
        }
        if let Some(user_stream) = &mut self.user_stream {
            user_stream.lifecycle_fingerprint_sha256 = user_stream_fingerprint(user_stream)?;
        }

        self.binary_fingerprint_sha256 = binary_fingerprint(self)?;
        self.host_fingerprint_sha256 = host_fingerprint(self)?;
        self.config_fingerprint_sha256 = config.fingerprint()?;
        self.credential_slot_nonsecret_fingerprint_sha256 = slot_fingerprint(config)?;
        self.limitations = expected_limitations();
        if parse_embedded_config(self)? != *config {
            return Err(invalid(
                "collector config differs from the embedded config evidence",
            ));
        }
        validate_pre_summary(self, config)?;
        self.summary = derive_summary(self, config)?;
        if enforce_failure_consistency {
            validate_failure_consistency(self)?;
        }
        self.evidence_fingerprint_sha256 = artifact_fingerprint(self)?;
        Ok(())
    }
}

pub fn verify_pm_read_only_smoke_artifact_bytes(
    bytes: &[u8],
) -> Result<PmReadOnlySmokeArtifact, PmReadOnlySmokeVerificationError> {
    if bytes.len() as u64 > MAX_PM_READ_ONLY_ARTIFACT_BYTES {
        return Err(PmReadOnlySmokeVerificationError::ArtifactTooLarge);
    }
    let artifact: PmReadOnlySmokeArtifact = serde_json::from_slice(bytes)
        .map_err(|_| PmReadOnlySmokeVerificationError::MalformedArtifact)?;
    verify_artifact(artifact)
}

/// Verify one artifact's structure and internal derivations.
///
/// This establishes neither external trust anchors nor artifact provenance.
/// Operator review should additionally use
/// [`verify_pm_read_only_smoke_path_with_anchors`] and preserve an independent
/// chain of custody for the collection process and output.
pub fn verify_pm_read_only_smoke_path(
    path: impl AsRef<Path>,
) -> Result<PmReadOnlySmokeArtifact, PmReadOnlySmokeVerificationError> {
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
        .map_err(|source| PmReadOnlySmokeVerificationError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    let metadata = file
        .metadata()
        .map_err(|source| PmReadOnlySmokeVerificationError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    if !metadata.is_file() {
        return Err(PmReadOnlySmokeVerificationError::InvalidPath {
            path: path.to_path_buf(),
            message: "must be a regular file and not a symbolic link".to_string(),
        });
    }
    #[cfg(target_os = "linux")]
    {
        if !linux_artifact_metadata_is_private(&metadata) {
            return Err(PmReadOnlySmokeVerificationError::InvalidPath {
                path: path.to_path_buf(),
                message: "must be owner-owned, single-link, and mode 0600".to_string(),
            });
        }
    }
    if metadata.len() > MAX_PM_READ_ONLY_ARTIFACT_BYTES {
        return Err(PmReadOnlySmokeVerificationError::TooLarge {
            path: path.to_path_buf(),
            actual: metadata.len(),
            limit: MAX_PM_READ_ONLY_ARTIFACT_BYTES,
        });
    }
    let mut bytes = Vec::with_capacity(
        usize::try_from(metadata.len())
            .unwrap_or(MAX_PM_READ_ONLY_ARTIFACT_BYTES as usize)
            .min(MAX_PM_READ_ONLY_ARTIFACT_BYTES as usize),
    );
    file.by_ref()
        .take(MAX_PM_READ_ONLY_ARTIFACT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| PmReadOnlySmokeVerificationError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() as u64 > MAX_PM_READ_ONLY_ARTIFACT_BYTES {
        return Err(PmReadOnlySmokeVerificationError::TooLarge {
            path: path.to_path_buf(),
            actual: bytes.len() as u64,
            limit: MAX_PM_READ_ONLY_ARTIFACT_BYTES,
        });
    }
    let post_read = file
        .metadata()
        .map_err(|source| PmReadOnlySmokeVerificationError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    if !opened_regular_file_metadata_is_stable(&metadata, &post_read, bytes.len() as u64) {
        return Err(PmReadOnlySmokeVerificationError::InvalidPath {
            path: path.to_path_buf(),
            message: "opened file changed while it was read".to_string(),
        });
    }
    #[cfg(target_os = "linux")]
    if !linux_artifact_metadata_is_private(&post_read) {
        return Err(PmReadOnlySmokeVerificationError::InvalidPath {
            path: path.to_path_buf(),
            message: "must remain owner-owned, single-link, and mode 0600".to_string(),
        });
    }
    verify_pm_read_only_smoke_artifact_bytes(&bytes)
}

/// Verify structural evidence against a separately reviewed config and require
/// its declared binary digest to match the currently running verifier.
///
/// These are consistency anchors, not an attestation: every artifact field and
/// fingerprint is public and recomputable. Authenticating who ran collection,
/// which host executed it, or whether the observations came from the venue
/// requires an independently trusted collection process and artifact chain of
/// custody (or a separately reviewed external attestation system).
pub fn verify_pm_read_only_smoke_path_with_anchors(
    artifact_path: impl AsRef<Path>,
    reviewed_config_path: impl AsRef<Path>,
) -> Result<PmReadOnlySmokeArtifact, PmReadOnlySmokeVerificationError> {
    let (reviewed_config, reviewed_evidence) =
        load_pm_read_only_smoke_config_path(reviewed_config_path)?;
    let artifact = verify_pm_read_only_smoke_path(artifact_path)?;
    if artifact.config != reviewed_evidence
        || artifact.config_fingerprint_sha256 != reviewed_config.fingerprint()?
    {
        return Err(PmReadOnlySmokeVerificationError::ReviewedConfigMismatch);
    }
    let verifier_sha256 = current_executable_sha256()
        .map_err(|_| PmReadOnlySmokeVerificationError::VerifierDigestUnavailable)?;
    if artifact.binary_sha256 != verifier_sha256 {
        return Err(PmReadOnlySmokeVerificationError::DeclaredBinaryMismatch);
    }
    Ok(artifact)
}

#[cfg(target_os = "linux")]
fn linux_effective_uid() -> Option<u32> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|line| line.starts_with("Uid:"))?
        .split_ascii_whitespace()
        .nth(2)?
        .parse()
        .ok()
}

#[cfg(target_os = "linux")]
fn linux_artifact_metadata_is_private(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;

    metadata.mode() & 0o7777 == 0o600
        && metadata.nlink() == 1
        && linux_effective_uid().is_some_and(|uid| uid == metadata.uid())
}

/// Turn a structurally verified artifact into an explicit passing gate.
pub fn require_pm_read_only_smoke_pass(
    artifact: &PmReadOnlySmokeArtifact,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    if artifact.summary.passed {
        Ok(())
    } else {
        Err(PmReadOnlySmokeVerificationError::NotPassing)
    }
}

fn verify_artifact(
    artifact: PmReadOnlySmokeArtifact,
) -> Result<PmReadOnlySmokeArtifact, PmReadOnlySmokeVerificationError> {
    validate_fixed_envelope(&artifact)?;
    let config = parse_embedded_config(&artifact)?;
    validate_pre_summary(&artifact, &config)?;
    let expected_summary = derive_summary(&artifact, &config)?;
    if artifact.summary != expected_summary {
        return Err(invalid(
            "stored per-gate summary differs from complete re-derivation",
        ));
    }
    validate_failure_consistency(&artifact)?;
    if artifact.evidence_fingerprint_sha256 != artifact_fingerprint(&artifact)? {
        return Err(invalid("artifact evidence fingerprint mismatch"));
    }
    Ok(artifact)
}

fn validate_fixed_envelope(
    artifact: &PmReadOnlySmokeArtifact,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    if artifact.schema_version != PM_READ_ONLY_ARTIFACT_SCHEMA_VERSION
        || artifact.product != PM_READ_ONLY_PRODUCT
        || artifact.coverage != PM_READ_ONLY_COVERAGE
        || artifact.contract_version != PM_READ_ONLY_CONTRACT_VERSION
    {
        return Err(invalid(
            "artifact schema, product, coverage, or contract version is unsupported",
        ));
    }
    if artifact.binary_name != BINARY_NAME
        || artifact.binary_version != env!("CARGO_PKG_VERSION")
        || !is_sha256(&artifact.binary_sha256)
        || artifact.binary_fingerprint_sha256 != binary_fingerprint(artifact)?
    {
        return Err(invalid("binary provenance is invalid"));
    }
    validate_visible_ascii(
        &artifact.host_name,
        MAX_HOST_NAME_BYTES,
        "host name is invalid",
    )?;
    validate_visible_ascii(
        &artifact.host_os,
        MAX_SHORT_FIELD_BYTES,
        "host OS is invalid",
    )?;
    validate_visible_ascii(
        &artifact.host_arch,
        MAX_SHORT_FIELD_BYTES,
        "host architecture is invalid",
    )?;
    if artifact.host_fingerprint_sha256 != host_fingerprint(artifact)? {
        return Err(invalid("host fingerprint mismatch"));
    }
    if artifact.started_unix_ms < MIN_SUPPORTED_UNIX_MS
        || artifact.finished_unix_ms > MAX_SUPPORTED_UNIX_MS
        || artifact.finished_unix_ms < artifact.started_unix_ms
        || artifact.finished_unix_ms - artifact.started_unix_ms > MAX_SMOKE_DURATION_MS
    {
        return Err(invalid(
            "artifact start/finish Unix-millisecond bounds are invalid",
        ));
    }
    if artifact.production_order_entry_authorized
        || artifact.mutation_roles_constructed
        || artifact.mutation_requests != 0
        || artifact.teardown.mutation_roles_constructed
        || artifact.teardown.mutation_requests != 0
    {
        return Err(invalid(
            "read-only artifact contains mutation authority or activity",
        ));
    }
    validate_teardown(&artifact.teardown, artifact.collection_failure.as_ref())?;
    Ok(())
}

fn parse_embedded_config(
    artifact: &PmReadOnlySmokeArtifact,
) -> Result<PmReadOnlySmokeConfig, PmReadOnlySmokeVerificationError> {
    let evidence = &artifact.config;
    if evidence.canonical_toml.len() as u64 > crate::MAX_PM_READ_ONLY_CONFIG_BYTES
        || evidence.canonical_bytes != evidence.canonical_toml.len() as u64
        || sha256_hex(evidence.canonical_toml.as_bytes()) != evidence.canonical_sha256
    {
        return Err(invalid("embedded config evidence is invalid"));
    }
    let config: PmReadOnlySmokeConfig = toml::from_str(&evidence.canonical_toml)
        .map_err(|_| invalid("embedded config TOML is malformed or has unknown fields"))?;
    config.validate()?;
    if toml::to_string(&config)
        .map_err(|_| invalid("embedded config could not be canonicalized"))?
        != evidence.canonical_toml
    {
        return Err(invalid("embedded config TOML is not canonical"));
    }
    if config.schema_version != PM_READ_ONLY_CONFIG_SCHEMA_VERSION
        || artifact.config_fingerprint_sha256 != config.fingerprint()?
        || artifact.credential_slot_nonsecret_fingerprint_sha256 != slot_fingerprint(&config)?
    {
        return Err(invalid("embedded config fingerprint or schema mismatch"));
    }
    Ok(config)
}

fn validate_pre_summary(
    artifact: &PmReadOnlySmokeArtifact,
    config: &PmReadOnlySmokeConfig,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    validate_fixed_envelope(artifact)?;

    if let Some(failure) = &artifact.collection_failure {
        validate_failure(failure, artifact.started_unix_ms, artifact.finished_unix_ms)?;
    }
    if artifact.account.is_some() && artifact.metadata.is_none()
        || artifact.reconciliation.is_some() && artifact.account.is_none()
    {
        return Err(invalid(
            "optional collection evidence is not a valid phase prefix",
        ));
    }
    if let Some(metadata) = &artifact.metadata {
        validate_metadata(metadata, config)?;
    }
    if let Some(account) = &artifact.account {
        let metadata = artifact
            .metadata
            .as_ref()
            .ok_or(invalid("account evidence requires public metadata"))?;
        validate_account(account, config, metadata)?;
    }
    if let Some(reconciliation) = &artifact.reconciliation {
        validate_reconciliation(reconciliation, config)?;
    }
    if let Some(user_stream) = &artifact.user_stream {
        validate_user_stream(user_stream, config, artifact)?;
    }
    if artifact.user_stream.is_some() != artifact.teardown.user_stream_task_started
        || artifact.teardown.user_stream_task_started
            && (!artifact.teardown.user_stream_task_joined
                || !artifact.teardown.user_stream_task_completed_cleanly)
    {
        return Err(invalid(
            "user-stream evidence contradicts teardown task-start evidence",
        ));
    }
    if (artifact.account.is_some()
        || artifact.reconciliation.is_some()
        || artifact.user_stream.is_some())
        && (!artifact.teardown.credentials_loaded
            || !artifact.teardown.credential_authority_task_started)
    {
        return Err(invalid(
            "authenticated evidence exists without credential-authority lifecycle evidence",
        ));
    }
    if artifact.limitations != expected_limitations() {
        return Err(invalid(
            "artifact limitations are incomplete or non-canonical",
        ));
    }
    Ok(())
}

fn validate_failure_consistency(
    artifact: &PmReadOnlySmokeArtifact,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    if artifact.summary.passed == artifact.collection_failure.is_some() {
        return Err(invalid("pass/failure discriminator is contradictory"));
    }
    let Some(failure) = artifact.collection_failure.as_ref() else {
        return Ok(());
    };
    let (stage_is_coherent, named_gate_failed) = match failure.stage.as_str() {
        "public_metadata" => (
            artifact.metadata.is_none()
                && artifact.account.is_none()
                && artifact.reconciliation.is_none()
                && artifact.user_stream.is_none()
                && artifact.teardown.credentials_loaded
                && !artifact.teardown.user_stream_task_started
                && !artifact.teardown.credential_authority_task_started,
            !artifact.summary.public_metadata_valid,
        ),
        "authenticated_account" | "open_orders" | "trades" => {
            let named_gate_failed = match failure.stage.as_str() {
                "authenticated_account" => {
                    !artifact.summary.account_balances_observed
                        || !artifact.summary.position_observed
                        || !artifact.summary.required_allowances_complete
                }
                "open_orders" => !artifact.summary.open_orders_complete,
                "trades" => !artifact.summary.trades_complete,
                _ => unreachable!("outer match closes the failure stages"),
            };
            (
                artifact.metadata.is_some()
                    && artifact.account.is_none()
                    && artifact.reconciliation.is_none()
                    && artifact.teardown.credentials_loaded,
                named_gate_failed,
            )
        }
        "user_stream" => (
            artifact.metadata.is_some()
                && artifact.account.is_some()
                && artifact.reconciliation.is_some()
                && artifact.user_stream.is_some()
                && artifact.teardown.user_stream_task_started
                && artifact.teardown.credential_authority_task_started,
            !artifact.summary.user_stream_authenticated,
        ),
        "teardown" => (
            artifact.metadata.is_some()
                && artifact.teardown.credentials_loaded
                && artifact.teardown.user_stream_task_started
                && artifact.teardown.credential_authority_task_started,
            !artifact.summary.teardown_complete,
        ),
        _ => (false, false),
    };
    if !stage_is_coherent || !named_gate_failed {
        return Err(invalid(
            "collection failure stage contradicts the retained evidence",
        ));
    }
    Ok(())
}

fn validate_failure(
    failure: &PmReadOnlyCollectionFailureEvidence,
    started_unix_ms: u64,
    finished_unix_ms: u64,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    if !FAILURE_STAGES.contains(&failure.stage.as_str())
        || !valid_failure_pair(&failure.stage, &failure.kind)
        || failure.occurred_unix_ms < started_unix_ms
        || failure.occurred_unix_ms > finished_unix_ms
    {
        return Err(invalid(
            "collection failure stage, kind, or timestamp is invalid",
        ));
    }
    Ok(())
}

fn valid_failure_pair(stage: &str, kind: &str) -> bool {
    match stage {
        "public_metadata" => matches!(
            kind,
            "invalid_input"
                | "timeout"
                | "transport"
                | "rejected_status"
                | "malformed_response"
                | "scope_mismatch"
                | "insufficient_evidence"
        ),
        "authenticated_account" => matches!(
            kind,
            "timeout"
                | "transport"
                | "rejected_status"
                | "malformed_response"
                | "scope_mismatch"
                | "owner_mismatch"
                | "insufficient_evidence"
        ),
        "open_orders" | "trades" => matches!(
            kind,
            "timeout"
                | "transport"
                | "rejected_status"
                | "malformed_response"
                | "scope_mismatch"
                | "owner_mismatch"
                | "incomplete_pagination"
                | "insufficient_evidence"
        ),
        "user_stream" => matches!(
            kind,
            "timeout"
                | "transport"
                | "malformed_response"
                | "scope_mismatch"
                | "owner_mismatch"
                | "reconnect_exhausted"
                | "insufficient_evidence"
        ),
        "teardown" => kind == "shutdown",
        _ => false,
    }
}

fn derive_metadata(
    config: &PmReadOnlySmokeConfig,
    market_body: &[u8],
    clob_body: &[u8],
) -> Result<PmReadOnlyMetadataEvidence, PmReadOnlySmokeVerificationError> {
    if market_body.len() > MAX_PUBLIC_REST_BODY_BYTES
        || clob_body.len() > MAX_PUBLIC_REST_BODY_BYTES
    {
        return Err(invalid("public metadata response exceeds its wire bound"));
    }
    let scope = config.wire_scope()?;
    let lifecycle = parse_live_clob_market_lifecycle(market_body, scope)
        .map_err(|_| invalid("public lifecycle metadata is invalid or out of scope"))?;
    let trading = parse_live_clob_v2_metadata(
        clob_body,
        PmClobV2RequestScope::new(scope.condition(), scope.token()),
    )
    .map_err(|_| invalid("public CLOB metadata is invalid or out of scope"))?;

    #[derive(Serialize)]
    struct LifecycleProjection {
        condition_id: String,
        market_id: String,
        active: bool,
        closed: bool,
        archived: bool,
        accepting_orders: bool,
        order_book_enabled: bool,
    }
    #[derive(Serialize)]
    struct TokenProjection {
        token_id: String,
        outcome: String,
    }
    #[derive(Serialize)]
    struct TradingProjection {
        requested_condition_id: String,
        reported_condition_id: Option<String>,
        tokens: Vec<TokenProjection>,
        configured_token_id: String,
        configured_outcome: String,
        tick: String,
        minimum_order_size: String,
        negative_risk: bool,
    }

    let live = lifecycle.lifecycle();
    let lifecycle_projection = LifecycleProjection {
        condition_id: lifecycle.condition().to_string(),
        market_id: lifecycle.market().to_string(),
        active: live.active(),
        closed: live.closed(),
        archived: live.archived(),
        accepting_orders: live.accepting_orders(),
        order_book_enabled: live.order_book_enabled(),
    };
    let mut tokens = trading
        .tokens()
        .iter()
        .map(|token| TokenProjection {
            token_id: token.token().units().to_string(),
            outcome: token.label().to_string(),
        })
        .collect::<Vec<_>>();
    tokens.sort_by(|left, right| left.token_id.cmp(&right.token_id));
    let configured = trading.configured_outcome();
    let trading_projection = TradingProjection {
        requested_condition_id: trading.requested_condition().to_string(),
        reported_condition_id: trading.reported_condition().map(|value| value.to_string()),
        tokens,
        configured_token_id: configured.token().units().to_string(),
        configured_outcome: configured.label().to_string(),
        tick: trading.tick().to_string(),
        minimum_order_size: trading.minimum_order_size().to_string(),
        negative_risk: trading.negative_risk(),
    };
    let lifecycle_fingerprint_sha256 = hash_json(LIFECYCLE_DOMAIN, &lifecycle_projection)?;
    let trading_fingerprint_sha256 = hash_json(TRADING_DOMAIN, &trading_projection)?;
    let joined_fingerprint_sha256 = hash_json(
        JOINED_METADATA_DOMAIN,
        &(&lifecycle_fingerprint_sha256, &trading_fingerprint_sha256),
    )?;

    Ok(PmReadOnlyMetadataEvidence {
        market_body_base64: BASE64_STANDARD.encode(market_body),
        market_body_bytes: market_body.len() as u64,
        market_body_sha256: sha256_hex(market_body),
        clob_body_base64: BASE64_STANDARD.encode(clob_body),
        clob_body_bytes: clob_body.len() as u64,
        clob_body_sha256: sha256_hex(clob_body),
        condition_id: lifecycle_projection.condition_id,
        market_id: lifecycle_projection.market_id,
        token_id: trading_projection.configured_token_id,
        outcome: trading_projection.configured_outcome,
        tick: trading_projection.tick,
        minimum_order_size: trading_projection.minimum_order_size,
        negative_risk: trading_projection.negative_risk,
        active: lifecycle_projection.active,
        closed: lifecycle_projection.closed,
        archived: lifecycle_projection.archived,
        accepting_orders: lifecycle_projection.accepting_orders,
        order_book_enabled: lifecycle_projection.order_book_enabled,
        token_count: trading_projection.tokens.len() as u64,
        lifecycle_fingerprint_sha256,
        trading_fingerprint_sha256,
        joined_fingerprint_sha256,
    })
}

fn validate_metadata(
    evidence: &PmReadOnlyMetadataEvidence,
    config: &PmReadOnlySmokeConfig,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    let market = decode_public_body(
        &evidence.market_body_base64,
        evidence.market_body_bytes,
        &evidence.market_body_sha256,
    )?;
    let clob = decode_public_body(
        &evidence.clob_body_base64,
        evidence.clob_body_bytes,
        &evidence.clob_body_sha256,
    )?;
    if *evidence != derive_metadata(config, &market, &clob)? {
        return Err(invalid(
            "public metadata projection or fingerprint mismatch",
        ));
    }
    Ok(())
}

fn decode_public_body(
    encoded: &str,
    declared_bytes: u64,
    declared_sha256: &str,
) -> Result<Vec<u8>, PmReadOnlySmokeVerificationError> {
    if encoded.len() > MAX_PUBLIC_REST_BODY_BYTES.saturating_mul(2) || !is_sha256(declared_sha256) {
        return Err(invalid("public metadata body encoding or hash is invalid"));
    }
    let decoded = BASE64_STANDARD
        .decode(encoded)
        .map_err(|_| invalid("public metadata body is not canonical base64"))?;
    if BASE64_STANDARD.encode(&decoded) != encoded
        || decoded.len() > MAX_PUBLIC_REST_BODY_BYTES
        || decoded.len() as u64 != declared_bytes
        || sha256_hex(&decoded) != declared_sha256
    {
        return Err(invalid(
            "public metadata body size, encoding, or SHA-256 mismatch",
        ));
    }
    Ok(decoded)
}

fn canonicalize_account(
    account: &mut PmReadOnlyAccountEvidence,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    account
        .allowances
        .sort_by(|left, right| allowance_sort_key(left).cmp(&allowance_sort_key(right)));
    account.allowance_count = account.allowances.len() as u64;
    account.canonical_sha256 = account_fingerprint(account)?;
    Ok(())
}

fn validate_account(
    account: &PmReadOnlyAccountEvidence,
    config: &PmReadOnlySmokeConfig,
    metadata: &PmReadOnlyMetadataEvidence,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    if account.authenticated_response_count != 2
        || account.allowance_count != account.allowances.len() as u64
        || account.allowances.len() != 2
        || account.canonical_sha256 != account_fingerprint(account)?
    {
        return Err(invalid(
            "account response, allowance count, or digest is invalid",
        ));
    }
    canonical_u256(&account.collateral_balance, "collateral balance is invalid")?;
    canonical_u256(&account.outcome_balance, "outcome balance is invalid")?;
    canonical_u256(&account.position_balance, "position balance is invalid")?;
    let scope = config.wire_scope()?;
    if account.position_token_id != scope.token().units().to_string()
        || account.position_balance != account.outcome_balance
        || account.position_available != metadata_is_tradeable(metadata)
    {
        return Err(invalid(
            "position projection is not derived from the configured token, balance, and lifecycle",
        ));
    }

    let expected = expected_allowances(config)?;
    for (actual, expected) in account.allowances.iter().zip(expected.iter()) {
        validate_allowance(actual, expected)?;
    }
    if account
        .allowances
        .windows(2)
        .any(|pair| allowance_sort_key(&pair[0]) >= allowance_sort_key(&pair[1]))
    {
        return Err(invalid("allowance rows are not canonical and unique"));
    }
    Ok(())
}

#[derive(Debug)]
struct ExpectedAllowance {
    asset_kind: &'static str,
    asset_contract: String,
    token_id: Option<String>,
    spender_address: String,
}

fn expected_allowances(
    config: &PmReadOnlySmokeConfig,
) -> Result<[ExpectedAllowance; 2], PmReadOnlySmokeVerificationError> {
    let domain = PmGoalFTradingDomain::from_metadata(config.expected_metadata()?)
        .map_err(|_| invalid("configured Goal-F account domain is invalid"))?;
    let mut rows = Vec::with_capacity(2);
    for requirement in domain.required_spenders() {
        let (asset_kind, asset_contract, token_id) = match requirement.asset() {
            PmAssetId::Collateral { contract } => ("collateral", contract.to_string(), None),
            PmAssetId::Outcome { contract, token } => (
                "outcome",
                contract.to_string(),
                Some(token.units().to_string()),
            ),
        };
        rows.push(ExpectedAllowance {
            asset_kind,
            asset_contract,
            token_id,
            spender_address: requirement.spender().to_string(),
        });
    }
    rows.sort_by(|left, right| left.asset_kind.cmp(right.asset_kind));
    rows.try_into()
        .map_err(|_| invalid("fixed Goal-F allowance set does not contain two rows"))
}

fn validate_allowance(
    actual: &PmReadOnlyAllowanceEvidence,
    expected: &ExpectedAllowance,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    canonical_u256(&actual.amount, "allowance amount is invalid")?;
    if !actual.present
        || actual.unscoped_scalar_present
        || actual.asset_kind != expected.asset_kind
        || actual.asset_contract != expected.asset_contract
        || actual.token_id != expected.token_id
        || actual.spender_address != expected.spender_address
    {
        return Err(invalid(
            "required allowance row is absent, unscoped, foreign, or non-canonical",
        ));
    }
    Ok(())
}

fn account_fingerprint(
    account: &PmReadOnlyAccountEvidence,
) -> Result<String, PmReadOnlySmokeVerificationError> {
    hash_json(
        ACCOUNT_DOMAIN,
        &(
            account.authenticated_response_count,
            &account.collateral_balance,
            &account.outcome_balance,
            &account.position_token_id,
            &account.position_balance,
            account.position_available,
            &account.allowances,
        ),
    )
}

fn allowance_sort_key(allowance: &PmReadOnlyAllowanceEvidence) -> (&str, &str, Option<&str>) {
    (
        allowance.asset_kind.as_str(),
        allowance.asset_contract.as_str(),
        allowance.token_id.as_deref(),
    )
}

fn canonicalize_reconciliation(
    evidence: &mut PmReadOnlyReconciliationEvidence,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    evidence
        .open_orders
        .sort_by(|left, right| left.order_id.cmp(&right.order_id));
    for trade in &mut evidence.trades {
        trade
            .maker_orders
            .sort_by(|left, right| left.order_id.cmp(&right.order_id));
    }
    evidence
        .trades
        .sort_by(|left, right| left.trade_id.cmp(&right.trade_id));
    evidence.open_orders_sha256 = hash_json(OPEN_ORDERS_DOMAIN, &evidence.open_orders)?;
    evidence.trades_sha256 = hash_json(TRADES_DOMAIN, &evidence.trades)?;
    Ok(())
}

fn validate_reconciliation(
    evidence: &PmReadOnlyReconciliationEvidence,
    config: &PmReadOnlySmokeConfig,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    if evidence.open_order_page_count == 0
        || evidence.open_order_page_count > MAX_PM_AUTHENTICATED_CUT_PAGES as u64
        || evidence.trade_page_count == 0
        || evidence.trade_page_count > MAX_PM_AUTHENTICATED_CUT_PAGES as u64
        || !evidence.open_order_terminal_cursor_seen
        || !evidence.trade_terminal_cursor_seen
        || evidence.open_orders.len() > MAX_PM_AUTHENTICATED_ORDER_ROWS
        || evidence.trades.len() > MAX_PM_AUTHENTICATED_TRADE_ROWS
        || evidence.open_order_count > MAX_PM_AUTHENTICATED_ORDER_ROWS as u64
        || evidence.trade_count > MAX_PM_AUTHENTICATED_TRADE_ROWS as u64
        || evidence.open_orders.len() as u64 > evidence.open_order_count
        || evidence.trades.len() as u64 > evidence.trade_count
        || evidence.open_order_count
            > evidence.open_order_page_count * MAX_PM_LIVE_PAGE_ITEMS as u64
        || evidence.trade_count > evidence.trade_page_count * MAX_PM_LIVE_PAGE_ITEMS as u64
    {
        return Err(invalid(
            "reconciliation pagination, terminal, or row bounds are invalid",
        ));
    }
    let retained_order_count = evidence.open_orders.len() as u64;
    let retained_trade_count = evidence.trades.len() as u64;
    let order_scope_total = evidence
        .open_order_scope_bound_count
        .checked_add(evidence.open_order_scope_mismatch_count)
        .ok_or(invalid("open-order scope-classification count overflows"))?;
    let trade_scope_total = evidence
        .trade_scope_bound_count
        .checked_add(evidence.trade_scope_mismatch_count)
        .ok_or(invalid("trade scope-classification count overflows"))?;
    if evidence.open_order_owner_bound_count != evidence.open_order_count
        || evidence.trade_owner_bound_count != evidence.trade_count
        || evidence.open_order_owner_mismatch_count != 0
        || evidence.trade_owner_mismatch_count != 0
        || evidence.open_order_scope_bound_count != retained_order_count
        || evidence.trade_scope_bound_count != retained_trade_count
        || order_scope_total != evidence.open_order_count
        || trade_scope_total != evidence.trade_count
    {
        return Err(invalid(
            "redacted reconciliation rows lack exact owner or scope binding",
        ));
    }
    if evidence.open_orders_sha256 != hash_json(OPEN_ORDERS_DOMAIN, &evidence.open_orders)?
        || evidence.trades_sha256 != hash_json(TRADES_DOMAIN, &evidence.trades)?
    {
        return Err(invalid("reconciliation row digest mismatch"));
    }

    let mut order_ids = BTreeSet::new();
    let mut previous = None;
    for order in &evidence.open_orders {
        validate_order(order, config)?;
        if !order_ids.insert(order.order_id.as_str())
            || previous.is_some_and(|value: &str| value >= order.order_id.as_str())
        {
            return Err(invalid("open-order rows are not canonical and unique"));
        }
        previous = Some(order.order_id.as_str());
    }
    let mut trade_ids = BTreeSet::new();
    let mut previous = None;
    for trade in &evidence.trades {
        validate_trade(trade, config)?;
        if !trade_ids.insert(trade.trade_id.as_str())
            || previous.is_some_and(|value: &str| value >= trade.trade_id.as_str())
        {
            return Err(invalid("trade rows are not canonical and unique"));
        }
        previous = Some(trade.trade_id.as_str());
    }
    Ok(())
}

fn validate_order(
    order: &PmReadOnlyOrderEvidence,
    config: &PmReadOnlySmokeConfig,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    PmVenueOrderId::new(&order.order_id).map_err(|_| invalid("open-order ID is invalid"))?;
    validate_scope(&order.condition_id, &order.token_id, config)?;
    validate_side(&order.side)?;
    canonical_quantity(
        &order.original_size,
        false,
        "open-order original size is invalid",
    )?;
    canonical_quantity(
        &order.size_matched,
        true,
        "open-order matched size is invalid",
    )?;
    validate_price(&order.price, config)?;
    validate_visible_ascii(
        &order.status,
        MAX_STATUS_FIELD_BYTES,
        "open-order status is invalid",
    )?;
    canonical_address(&order.maker_address, "open-order maker address is invalid")?;
    if order.maker_address != config.signer()?.to_string() {
        return Err(invalid(
            "retained open-order maker is not the configured signer",
        ));
    }
    if order.created_at == 0 {
        return Err(invalid("open-order creation timestamp is zero"));
    }
    validate_optional_visible(
        &order.outcome,
        MAX_SHORT_FIELD_BYTES,
        "open-order outcome is invalid",
    )?;
    validate_optional_visible(
        &order.order_type,
        MAX_SHORT_FIELD_BYTES,
        "open-order type is invalid",
    )?;
    if order
        .outcome
        .as_deref()
        .is_some_and(|value| value != config.outcome)
    {
        return Err(invalid(
            "open-order outcome conflicts with configured outcome",
        ));
    }
    Ok(())
}

fn validate_trade(
    trade: &PmReadOnlyTradeEvidence,
    config: &PmReadOnlySmokeConfig,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    PmFillId::new(&trade.trade_id).map_err(|_| invalid("trade ID is invalid"))?;
    let scope = config.wire_scope()?;
    if trade.condition_id != scope.condition().to_string() {
        return Err(invalid("retained trade condition is out of scope"));
    }
    let top_level_token = canonical_u256(&trade.token_id, "trade token is invalid")?;
    if top_level_token.is_zero() {
        return Err(invalid("trade token is zero"));
    }
    validate_side(&trade.side)?;
    canonical_quantity(&trade.size, false, "trade size is invalid")?;
    validate_price(&trade.price, config)?;
    validate_visible_ascii(
        &trade.status,
        MAX_STATUS_FIELD_BYTES,
        "trade status is invalid",
    )?;
    validate_optional_order_id(&trade.order_id)?;
    validate_optional_order_id(&trade.taker_order_id)?;
    validate_optional_visible(
        &trade.trader_side,
        MAX_SHORT_FIELD_BYTES,
        "trade trader-side is invalid",
    )?;
    if let Some(hash) = &trade.transaction_hash {
        validate_prefixed_hash(hash, "trade transaction hash is invalid")?;
    }
    validate_optional_u256(&trade.fee_rate_bps, "trade fee rate is invalid")?;
    if let Some(address) = &trade.maker_address {
        canonical_address(address, "trade maker address is invalid")?;
    }
    if trade.maker_orders.len() > MAX_MAKER_ROWS_PER_TRADE {
        return Err(invalid("trade maker-order row bound is exceeded"));
    }
    let mut previous = None;
    let mut seen = BTreeSet::new();
    for maker in &trade.maker_orders {
        validate_trade_maker(maker, config)?;
        if !seen.insert(maker.order_id.as_str())
            || previous.is_some_and(|value: &str| value >= maker.order_id.as_str())
        {
            return Err(invalid("trade maker rows are not canonical and unique"));
        }
        previous = Some(maker.order_id.as_str());
    }
    if trade.token_id != scope.token().units().to_string() && trade.maker_orders.is_empty() {
        return Err(invalid(
            "retained trade lacks a configured top-level token or configured-token maker leg",
        ));
    }
    Ok(())
}

fn validate_trade_maker(
    maker: &PmReadOnlyTradeMakerEvidence,
    config: &PmReadOnlySmokeConfig,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    PmVenueOrderId::new(&maker.order_id).map_err(|_| invalid("maker-order ID is invalid"))?;
    let scope = config.wire_scope()?;
    if maker.token_id != scope.token().units().to_string() {
        return Err(invalid("maker-order token is out of scope"));
    }
    validate_side(&maker.side)?;
    validate_price(&maker.price, config)?;
    canonical_quantity(
        &maker.matched_amount,
        false,
        "maker matched amount is invalid",
    )?;
    validate_optional_u256(&maker.fee_rate_bps, "maker fee rate is invalid")?;
    canonical_address(&maker.maker_address, "maker address is invalid")?;
    if maker.maker_address != config.signer()?.to_string() {
        return Err(invalid(
            "retained maker-order address is not the configured signer",
        ));
    }
    Ok(())
}

fn validate_user_stream(
    stream: &PmReadOnlyUserStreamEvidence,
    config: &PmReadOnlySmokeConfig,
    artifact: &PmReadOnlySmokeArtifact,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    let maximum_attempt_count = stream
        .reconnect_attempt_count
        .checked_add(1)
        .ok_or(invalid("user-stream connection-attempt count overflows"))?;
    let derived_event_count = stream
        .order_event_count
        .checked_add(stream.trade_event_count)
        .ok_or(invalid("user-stream event count overflows"))?;
    let derived_bound_count = stream
        .correlated_pong_count
        .checked_add(stream.scope_bound_event_count)
        .ok_or(invalid("user-stream bound-observation count overflows"))?;
    let owner_classified_count = stream
        .owner_bound_event_count
        .checked_add(stream.owner_mismatch_count)
        .ok_or(invalid("user-stream owner-classification count overflows"))?;
    let scope_classified_count = stream
        .scope_bound_event_count
        .checked_add(stream.scope_mismatch_count)
        .ok_or(invalid("user-stream scope-classification count overflows"))?;
    if stream.connection_attempt_count == 0
        || stream.connection_attempt_count > maximum_attempt_count
        || stream.connection_attempt_count < stream.reconnect_attempt_count
        || stream.connection_open_count > stream.connection_attempt_count
        || stream.subscription_count > stream.connection_open_count
        || stream.retirement_count > stream.connection_attempt_count
        || stream.retry_exhausted_count > 1
        || stream.correlated_pong_count > stream.ping_count
        || stream.event_count != derived_event_count
        || stream.owner_bound_event_count > stream.event_count
        || stream.scope_bound_event_count > stream.event_count
        || stream.owner_mismatch_count > stream.event_count
        || stream.scope_mismatch_count > stream.event_count
        || owner_classified_count != stream.event_count
        || scope_classified_count != stream.event_count
        || stream.bound_observation_count != derived_bound_count
        || stream.reconnect_attempt_count > u64::from(config.user_stream_max_reconnect_attempts)
        || stream.dwell_ms > config.user_stream_dwell_ms.saturating_add(60_000)
        || stream.dwell_ms > artifact.finished_unix_ms - artifact.started_unix_ms
        || stream.shutdown_event_count > 1
        || (stream.run_completed_without_transport_error && stream.shutdown_event_count != 1)
        || stream.lifecycle_fingerprint_sha256 != user_stream_fingerprint(stream)?
    {
        return Err(invalid(
            "user-stream lifecycle counts, bounds, or fingerprint are invalid",
        ));
    }
    Ok(())
}

fn user_stream_passes(
    stream: &PmReadOnlyUserStreamEvidence,
    config: &PmReadOnlySmokeConfig,
) -> bool {
    stream.connection_attempt_count == 1
        && stream.connection_open_count == 1
        && stream.subscription_count == 1
        && stream.reconnect_attempt_count == 0
        && stream.retirement_count == 0
        && stream.retry_exhausted_count == 0
        && stream.ping_count >= 1
        && stream.correlated_pong_count >= 1
        && stream.frame_count >= 1
        && stream.event_count >= 1
        && stream.bound_observation_count >= 2
        && stream.owner_bound_event_count >= 1
        && stream.scope_bound_event_count >= 1
        && stream.owner_bound_event_count == stream.event_count
        && stream.scope_bound_event_count == stream.event_count
        && stream.owner_mismatch_count == 0
        && stream.scope_mismatch_count == 0
        && stream.dwell_ms >= config.user_stream_dwell_ms
        && stream.shutdown_event_count == 1
        && stream.run_completed_without_transport_error
}

fn user_stream_fingerprint(
    stream: &PmReadOnlyUserStreamEvidence,
) -> Result<String, PmReadOnlySmokeVerificationError> {
    #[derive(Serialize)]
    struct Projection {
        connection_attempt_count: u64,
        connection_open_count: u64,
        subscription_count: u64,
        reconnect_attempt_count: u64,
        retirement_count: u64,
        retry_exhausted_count: u64,
        ping_count: u64,
        correlated_pong_count: u64,
        frame_count: u64,
        event_count: u64,
        order_event_count: u64,
        trade_event_count: u64,
        owner_bound_event_count: u64,
        scope_bound_event_count: u64,
        owner_mismatch_count: u64,
        scope_mismatch_count: u64,
        bound_observation_count: u64,
        dwell_ms: u64,
        shutdown_event_count: u64,
        run_completed_without_transport_error: bool,
    }
    hash_json(
        USER_STREAM_DOMAIN,
        &Projection {
            connection_attempt_count: stream.connection_attempt_count,
            connection_open_count: stream.connection_open_count,
            subscription_count: stream.subscription_count,
            reconnect_attempt_count: stream.reconnect_attempt_count,
            retirement_count: stream.retirement_count,
            retry_exhausted_count: stream.retry_exhausted_count,
            ping_count: stream.ping_count,
            correlated_pong_count: stream.correlated_pong_count,
            frame_count: stream.frame_count,
            event_count: stream.event_count,
            order_event_count: stream.order_event_count,
            trade_event_count: stream.trade_event_count,
            owner_bound_event_count: stream.owner_bound_event_count,
            scope_bound_event_count: stream.scope_bound_event_count,
            owner_mismatch_count: stream.owner_mismatch_count,
            scope_mismatch_count: stream.scope_mismatch_count,
            bound_observation_count: stream.bound_observation_count,
            dwell_ms: stream.dwell_ms,
            shutdown_event_count: stream.shutdown_event_count,
            run_completed_without_transport_error: stream.run_completed_without_transport_error,
        },
    )
}

fn validate_teardown(
    teardown: &PmReadOnlyTeardownEvidence,
    failure: Option<&PmReadOnlyCollectionFailureEvidence>,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    let user_coherent = if teardown.user_stream_task_started {
        teardown.user_stream_shutdown_requested
            && teardown.user_stream_task_joined
            && teardown.user_stream_task_completed_cleanly
            && !teardown.user_stream_abort_requested
    } else {
        !teardown.user_stream_shutdown_requested
            && !teardown.user_stream_abort_requested
            && !teardown.user_stream_task_joined
            && !teardown.user_stream_task_completed_cleanly
    };
    let authority_coherent = if teardown.credential_authority_task_started {
        teardown.credential_authority_shutdown_requested
            && teardown.credential_authority_task_joined
    } else {
        !teardown.credential_authority_shutdown_requested
            && !teardown.credential_authority_abort_requested
            && !teardown.credential_authority_task_joined
            && !teardown.credential_authority_task_completed_cleanly
    };
    let credentials_clean = if teardown.credentials_loaded {
        teardown.credentials_dropped_before_return
    } else {
        !teardown.credentials_dropped_before_return
    };
    let all_tasks_joined = (!teardown.user_stream_task_started || teardown.user_stream_task_joined)
        && (!teardown.credential_authority_task_started
            || teardown.credential_authority_task_joined);
    let partial_lifecycle = teardown.user_stream_task_started
        && (teardown.user_stream_abort_requested
            || !teardown.user_stream_task_joined
            || !teardown.user_stream_task_completed_cleanly)
        || teardown.credential_authority_task_started
            && (teardown.credential_authority_abort_requested
                || !teardown.credential_authority_task_joined
                || !teardown.credential_authority_task_completed_cleanly)
        || teardown.credentials_loaded && !teardown.credentials_dropped_before_return;
    let partial_is_shutdown_failure = !partial_lifecycle
        || failure.is_some_and(|failure| failure.stage == "teardown" && failure.kind == "shutdown");
    if !user_coherent
        || !authority_coherent
        || !credentials_clean
        || !partial_is_shutdown_failure
        || !teardown.all_tasks_joined
        || teardown.user_stream_task_started && !teardown.credential_authority_task_started
        || teardown.credential_authority_task_started && !teardown.credentials_loaded
        || teardown.all_tasks_joined != all_tasks_joined
        || teardown.mutation_roles_constructed
        || teardown.mutation_requests != 0
    {
        return Err(invalid(
            "teardown is partial, contradictory, or mutation-bearing",
        ));
    }
    Ok(())
}

fn teardown_passes(teardown: &PmReadOnlyTeardownEvidence) -> bool {
    teardown.user_stream_task_started
        && teardown.user_stream_shutdown_requested
        && !teardown.user_stream_abort_requested
        && teardown.user_stream_task_joined
        && teardown.user_stream_task_completed_cleanly
        && teardown.credential_authority_task_started
        && teardown.credential_authority_shutdown_requested
        && !teardown.credential_authority_abort_requested
        && teardown.credential_authority_task_joined
        && teardown.credential_authority_task_completed_cleanly
        && teardown.credentials_loaded
        && teardown.credentials_dropped_before_return
        && teardown.all_tasks_joined
        && !teardown.mutation_roles_constructed
        && teardown.mutation_requests == 0
}

fn derive_summary(
    artifact: &PmReadOnlySmokeArtifact,
    config: &PmReadOnlySmokeConfig,
) -> Result<PmReadOnlySmokeSummary, PmReadOnlySmokeVerificationError> {
    let metadata_valid = artifact
        .metadata
        .as_ref()
        .is_some_and(|metadata| metadata_matches_config(metadata, config));
    let account_present = artifact.account.is_some();
    let reconciliation = artifact.reconciliation.as_ref();
    let open_orders_complete = reconciliation.is_some_and(|value| {
        value.open_order_page_count > 0 && value.open_order_terminal_cursor_seen
    });
    let trades_complete = reconciliation
        .is_some_and(|value| value.trade_page_count > 0 && value.trade_terminal_cursor_seen);
    let owner_evidence_non_vacuous = reconciliation
        .map_or(0, |value| {
            value
                .open_order_owner_bound_count
                .saturating_add(value.trade_owner_bound_count)
        })
        .saturating_add(
            artifact
                .user_stream
                .as_ref()
                .map_or(0, |value| value.owner_bound_event_count),
        )
        > 0;
    let user_stream_authenticated = artifact
        .user_stream
        .as_ref()
        .is_some_and(|value| user_stream_passes(value, config));
    let authorization_closed = !artifact.production_order_entry_authorized
        && !artifact.mutation_roles_constructed
        && artifact.mutation_requests == 0
        && !artifact.teardown.mutation_roles_constructed
        && artifact.teardown.mutation_requests == 0;
    let limitations_explicit = artifact.limitations == expected_limitations();
    let teardown_complete = teardown_passes(&artifact.teardown);
    let mut summary = PmReadOnlySmokeSummary {
        provenance_valid: true,
        config_valid: true,
        authorization_closed,
        public_metadata_valid: metadata_valid,
        account_balances_observed: account_present,
        position_observed: account_present,
        required_allowances_complete: account_present,
        open_orders_complete,
        trades_complete,
        owner_evidence_non_vacuous,
        user_stream_authenticated,
        teardown_complete,
        limitations_explicit,
        passed: false,
    };
    summary.passed = summary.provenance_valid
        && summary.config_valid
        && summary.authorization_closed
        && summary.public_metadata_valid
        && summary.account_balances_observed
        && summary.position_observed
        && summary.required_allowances_complete
        && summary.open_orders_complete
        && summary.trades_complete
        && summary.owner_evidence_non_vacuous
        && summary.user_stream_authenticated
        && summary.teardown_complete
        && summary.limitations_explicit
        && artifact.collection_failure.is_none();
    Ok(summary)
}

fn metadata_matches_config(
    metadata: &PmReadOnlyMetadataEvidence,
    config: &PmReadOnlySmokeConfig,
) -> bool {
    let Ok(scope) = config.wire_scope() else {
        return false;
    };
    let Ok(expected_metadata) = config.expected_metadata() else {
        return false;
    };
    metadata.condition_id == scope.condition().to_string()
        && metadata.market_id == scope.market().to_string()
        && metadata.token_id == scope.token().units().to_string()
        && metadata.outcome == config.outcome
        && metadata.tick == expected_metadata.tick().to_string()
        && metadata.minimum_order_size == expected_metadata.minimum_order_size().to_string()
        && metadata.negative_risk == config.negative_risk
        && metadata.token_count > 0
        && metadata_is_tradeable(metadata)
}

fn metadata_is_tradeable(metadata: &PmReadOnlyMetadataEvidence) -> bool {
    metadata.active
        && !metadata.closed
        && !metadata.archived
        && metadata.accepting_orders
        && metadata.order_book_enabled
}

fn binary_fingerprint(
    artifact: &PmReadOnlySmokeArtifact,
) -> Result<String, PmReadOnlySmokeVerificationError> {
    hash_json(
        BINARY_DOMAIN,
        &(
            &artifact.binary_name,
            &artifact.binary_version,
            &artifact.binary_sha256,
        ),
    )
}

fn host_fingerprint(
    artifact: &PmReadOnlySmokeArtifact,
) -> Result<String, PmReadOnlySmokeVerificationError> {
    hash_json(
        HOST_DOMAIN,
        &(&artifact.host_name, &artifact.host_os, &artifact.host_arch),
    )
}

fn slot_fingerprint(
    config: &PmReadOnlySmokeConfig,
) -> Result<String, PmReadOnlySmokeVerificationError> {
    let signer = config.signer()?.to_string();
    let funder = config.funder()?.to_string();
    hash_json(
        SLOT_DOMAIN,
        &(
            &config.credential_slot_id,
            signer,
            funder,
            config.chain_id,
            config.signature_type,
        ),
    )
}

fn artifact_fingerprint(
    artifact: &PmReadOnlySmokeArtifact,
) -> Result<String, PmReadOnlySmokeVerificationError> {
    let mut value = serde_json::to_value(artifact)
        .map_err(|_| invalid("artifact could not be canonicalized"))?;
    value
        .as_object_mut()
        .ok_or(invalid("artifact canonical projection is not an object"))?
        .remove("evidence_fingerprint_sha256");
    hash_json(ARTIFACT_DOMAIN, &value)
}

fn validate_scope(
    condition_id: &str,
    token_id: &str,
    config: &PmReadOnlySmokeConfig,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    let scope = config.wire_scope()?;
    if condition_id != scope.condition().to_string()
        || token_id != scope.token().units().to_string()
    {
        return Err(invalid("private row is outside the exact configured scope"));
    }
    Ok(())
}

fn validate_side(side: &str) -> Result<(), PmReadOnlySmokeVerificationError> {
    if matches!(side, "buy" | "sell") {
        Ok(())
    } else {
        Err(invalid("private row side is not canonical"))
    }
}

fn validate_price(
    value: &str,
    config: &PmReadOnlySmokeConfig,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    let price =
        PmPrice::parse_decimal(value).map_err(|_| invalid("private row price is invalid"))?;
    let tick = config.expected_metadata()?.tick();
    price
        .validate_tick(tick)
        .map_err(|_| invalid("private row price is off the configured tick"))?;
    if price.to_string() != value {
        return Err(invalid("private row price is not canonical decimal"));
    }
    Ok(())
}

fn canonical_quantity(
    value: &str,
    zero_allowed: bool,
    error: &'static str,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    if zero_allowed {
        let parsed = PmBookQuantity::parse_decimal(value).map_err(|_| invalid(error))?;
        let canonical = match parsed {
            PmBookQuantity::Delete => "0".to_string(),
            PmBookQuantity::Quantity(quantity) => quantity.to_string(),
        };
        if canonical != value {
            return Err(invalid(error));
        }
    } else {
        let parsed = PmQuantity::parse_decimal(value).map_err(|_| invalid(error))?;
        if parsed.to_string() != value {
            return Err(invalid(error));
        }
    }
    Ok(())
}

fn canonical_u256(
    value: &str,
    error: &'static str,
) -> Result<U256, PmReadOnlySmokeVerificationError> {
    let parsed = U256::from_str(value).map_err(|_| invalid(error))?;
    if parsed.to_string() != value {
        return Err(invalid(error));
    }
    Ok(parsed)
}

fn canonical_address(
    value: &str,
    error: &'static str,
) -> Result<EvmAddress, PmReadOnlySmokeVerificationError> {
    let parsed = EvmAddress::parse(value).map_err(|_| invalid(error))?;
    if parsed.to_string() != value {
        return Err(invalid(error));
    }
    Ok(parsed)
}

fn validate_optional_u256(
    value: &Option<String>,
    error: &'static str,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    if let Some(value) = value {
        canonical_u256(value, error)?;
    }
    Ok(())
}

fn validate_optional_order_id(
    value: &Option<String>,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    if let Some(value) = value {
        PmVenueOrderId::new(value).map_err(|_| invalid("optional venue-order ID is invalid"))?;
    }
    Ok(())
}

fn validate_optional_visible(
    value: &Option<String>,
    capacity: usize,
    error: &'static str,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    if let Some(value) = value {
        validate_visible_ascii(value, capacity, error)?;
    }
    Ok(())
}

fn validate_visible_ascii(
    value: &str,
    capacity: usize,
    error: &'static str,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    if value.is_empty()
        || value.len() > capacity
        || !value.is_ascii()
        || value.trim_ascii() != value
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == 0x7f)
    {
        Err(invalid(error))
    } else {
        Ok(())
    }
}

fn validate_prefixed_hash(
    value: &str,
    error: &'static str,
) -> Result<(), PmReadOnlySmokeVerificationError> {
    let Some(hex) = value.strip_prefix("0x") else {
        return Err(invalid(error));
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(error));
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn hash_json<T: Serialize>(
    domain: &[u8],
    value: &T,
) -> Result<String, PmReadOnlySmokeVerificationError> {
    let canonical = serde_json::to_vec(value)
        .map_err(|_| invalid("evidence projection could not be canonicalized"))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(canonical);
    Ok(hex_lower(&hasher.finalize()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing hex to String cannot fail");
    }
    output
}

fn expected_limitations() -> Vec<String> {
    PM_READ_ONLY_LIMITATIONS
        .iter()
        .map(|value| (*value).to_string())
        .collect()
}

fn empty_summary() -> PmReadOnlySmokeSummary {
    PmReadOnlySmokeSummary {
        provenance_valid: false,
        config_valid: false,
        authorization_closed: false,
        public_metadata_valid: false,
        account_balances_observed: false,
        position_observed: false,
        required_allowances_complete: false,
        open_orders_complete: false,
        trades_complete: false,
        owner_evidence_non_vacuous: false,
        user_stream_authenticated: false,
        teardown_complete: false,
        limitations_explicit: false,
        passed: false,
    }
}

fn invalid(message: &'static str) -> PmReadOnlySmokeVerificationError {
    PmReadOnlySmokeVerificationError::Invalid(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PmReadOnlyConfigEvidence;

    fn config() -> PmReadOnlySmokeConfig {
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
            request_timeout_ms: 10_000,
            user_stream_dwell_ms: 12_000,
            user_stream_idle_timeout_ms: 30_000,
            user_stream_pong_timeout_ms: 5_000,
            user_stream_max_reconnect_attempts: 2,
            user_stream_reconnect_backoff_ms: 500,
            user_stream_event_channel_capacity: 64,
            api_key_file: "api-key".into(),
            secret_file: "secret".into(),
            passphrase_file: "passphrase".into(),
        }
    }

    fn config_evidence(config: &PmReadOnlySmokeConfig) -> PmReadOnlyConfigEvidence {
        let toml = toml::to_string(config).unwrap();
        PmReadOnlyConfigEvidence {
            canonical_bytes: toml.len() as u64,
            canonical_sha256: sha256_hex(toml.as_bytes()),
            canonical_toml: toml,
        }
    }

    fn metadata(config: &PmReadOnlySmokeConfig) -> PmReadOnlyMetadataEvidence {
        let market = format!(
            r#"{{"condition_id":"{}","question_id":"{}","active":true,"closed":false,"archived":false,"accepting_orders":true,"enable_order_book":true}}"#,
            config.condition_id, config.market_id
        );
        let clob = format!(
            r#"{{"c":"{}","t":[{{"t":"1234","o":"Yes"}},{{"t":"5678","o":"No"}}],"mts":0.01,"mos":5,"nr":false}}"#,
            config.condition_id,
        );
        PmReadOnlyMetadataEvidence::from_public_bodies(config, market.as_bytes(), clob.as_bytes())
            .unwrap()
    }

    fn account(config: &PmReadOnlySmokeConfig) -> PmReadOnlyAccountEvidence {
        let allowances = expected_allowances(config)
            .unwrap()
            .into_iter()
            .map(|expected| PmReadOnlyAllowanceEvidence {
                asset_kind: expected.asset_kind.into(),
                asset_contract: expected.asset_contract,
                token_id: expected.token_id,
                spender_address: expected.spender_address,
                amount: "1".into(),
                present: true,
                unscoped_scalar_present: false,
            })
            .collect();
        PmReadOnlyAccountEvidence {
            authenticated_response_count: 0,
            collateral_balance: "1000000".into(),
            outcome_balance: "5000000".into(),
            position_token_id: String::new(),
            position_balance: String::new(),
            position_available: false,
            allowances,
            allowance_count: 0,
            canonical_sha256: String::new(),
        }
    }

    fn reconciliation(config: &PmReadOnlySmokeConfig) -> PmReadOnlyReconciliationEvidence {
        PmReadOnlyReconciliationEvidence {
            open_order_page_count: 1,
            open_order_terminal_cursor_seen: true,
            open_order_count: 1,
            open_order_owner_bound_count: 1,
            open_order_scope_bound_count: 1,
            open_order_owner_mismatch_count: 0,
            open_order_scope_mismatch_count: 0,
            open_orders_sha256: String::new(),
            open_orders: vec![PmReadOnlyOrderEvidence {
                order_id: "order-1".into(),
                condition_id: config.condition_id.clone(),
                token_id: config.token_id.clone(),
                side: "buy".into(),
                original_size: "5".into(),
                size_matched: "0".into(),
                price: "0.5".into(),
                status: "live".into(),
                maker_address: config.signer_address.clone(),
                created_at: 1,
                expiration: 0,
                outcome: Some(config.outcome.clone()),
                order_type: Some("GTC".into()),
            }],
            trade_page_count: 1,
            trade_terminal_cursor_seen: true,
            trade_count: 0,
            trade_owner_bound_count: 0,
            trade_scope_bound_count: 0,
            trade_owner_mismatch_count: 0,
            trade_scope_mismatch_count: 0,
            trades_sha256: String::new(),
            trades: Vec::new(),
        }
    }

    fn user_stream() -> PmReadOnlyUserStreamEvidence {
        PmReadOnlyUserStreamEvidence {
            connection_attempt_count: 1,
            connection_open_count: 1,
            subscription_count: 1,
            reconnect_attempt_count: 0,
            retirement_count: 0,
            retry_exhausted_count: 0,
            ping_count: 1,
            correlated_pong_count: 1,
            frame_count: 1,
            event_count: 1,
            order_event_count: 1,
            trade_event_count: 0,
            owner_bound_event_count: 1,
            scope_bound_event_count: 1,
            owner_mismatch_count: 0,
            scope_mismatch_count: 0,
            bound_observation_count: 2,
            dwell_ms: 12_000,
            shutdown_event_count: 1,
            run_completed_without_transport_error: true,
            lifecycle_fingerprint_sha256: String::new(),
        }
    }

    fn teardown() -> PmReadOnlyTeardownEvidence {
        PmReadOnlyTeardownEvidence {
            user_stream_task_started: true,
            user_stream_shutdown_requested: true,
            user_stream_abort_requested: false,
            user_stream_task_joined: true,
            user_stream_task_completed_cleanly: true,
            credential_authority_task_started: true,
            credential_authority_shutdown_requested: true,
            credential_authority_abort_requested: false,
            credential_authority_task_joined: true,
            credential_authority_task_completed_cleanly: true,
            credentials_loaded: true,
            credentials_dropped_before_return: true,
            all_tasks_joined: true,
            mutation_roles_constructed: false,
            mutation_requests: 0,
        }
    }

    fn passing_artifact() -> PmReadOnlySmokeArtifact {
        let config = config();
        PmReadOnlySmokeArtifact::from_collected(
            "aa".repeat(32),
            "pm-host".into(),
            "linux".into(),
            "x86_64".into(),
            &config,
            config_evidence(&config),
            1_800_000_000_000,
            1_800_000_020_000,
            None,
            Some(metadata(&config)),
            Some(account(&config)),
            Some(reconciliation(&config)),
            Some(user_stream()),
            teardown(),
        )
        .unwrap()
    }

    #[test]
    fn passing_artifact_reparses_every_public_byte_and_derived_gate() {
        let artifact = passing_artifact();
        assert!(artifact.summary.passed);
        let bytes = serde_json::to_vec(&artifact).unwrap();
        let verified = verify_pm_read_only_smoke_artifact_bytes(&bytes).unwrap();
        require_pm_read_only_smoke_pass(&verified).unwrap();
    }

    #[test]
    fn account_wide_totals_omit_foreign_rows_and_keep_configured_maker_legs() {
        let config = config();
        let mut reconciliation = reconciliation(&config);
        reconciliation.open_order_count = 2;
        reconciliation.open_order_owner_bound_count = 2;
        reconciliation.open_order_scope_bound_count = 1;
        reconciliation.open_order_scope_mismatch_count = 1;
        reconciliation.trade_count = 2;
        reconciliation.trade_owner_bound_count = 2;
        reconciliation.trade_scope_bound_count = 1;
        reconciliation.trade_scope_mismatch_count = 1;
        reconciliation.trades.push(PmReadOnlyTradeEvidence {
            trade_id: "trade-1".into(),
            condition_id: config.condition_id.clone(),
            token_id: "5678".into(),
            side: "buy".into(),
            size: "5".into(),
            price: "0.5".into(),
            status: "matched".into(),
            order_id: None,
            taker_order_id: None,
            trader_side: None,
            transaction_hash: None,
            fee_rate_bps: None,
            maker_orders: vec![PmReadOnlyTradeMakerEvidence {
                order_id: "maker-order-1".into(),
                token_id: config.token_id.clone(),
                side: "sell".into(),
                price: "0.5".into(),
                matched_amount: "1".into(),
                fee_rate_bps: None,
                maker_address: config.signer_address.clone(),
            }],
            maker_address: None,
            timestamp: None,
            match_time: None,
            last_update: None,
        });
        let artifact = PmReadOnlySmokeArtifact::from_collected(
            "aa".repeat(32),
            "pm-host".into(),
            "linux".into(),
            "x86_64".into(),
            &config,
            config_evidence(&config),
            1_800_000_000_000,
            1_800_000_020_000,
            None,
            Some(metadata(&config)),
            Some(account(&config)),
            Some(reconciliation),
            Some(user_stream()),
            teardown(),
        )
        .unwrap();
        let evidence = artifact.reconciliation.as_ref().unwrap();
        assert_eq!(evidence.open_order_count, 2);
        assert_eq!(evidence.open_orders.len(), 1);
        assert_eq!(evidence.trade_count, 2);
        assert_eq!(evidence.trades.len(), 1);
        assert!(artifact.summary.passed);
        verify_pm_read_only_smoke_artifact_bytes(&serde_json::to_vec(&artifact).unwrap()).unwrap();
    }

    #[test]
    fn retained_order_and_maker_leg_addresses_must_be_the_configured_signer() {
        let mut order = passing_artifact();
        order.reconciliation.as_mut().unwrap().open_orders[0].maker_address =
            "0x1000000000000000000000000000000000000001".into();
        let open_orders_sha256 = hash_json(
            OPEN_ORDERS_DOMAIN,
            &order.reconciliation.as_ref().unwrap().open_orders,
        )
        .unwrap();
        order.reconciliation.as_mut().unwrap().open_orders_sha256 = open_orders_sha256;
        order.evidence_fingerprint_sha256 = artifact_fingerprint(&order).unwrap();
        assert!(
            verify_pm_read_only_smoke_artifact_bytes(&serde_json::to_vec(&order).unwrap()).is_err()
        );

        let config = config();
        let mut reconciliation = reconciliation(&config);
        reconciliation.trade_count = 1;
        reconciliation.trade_owner_bound_count = 1;
        reconciliation.trade_scope_bound_count = 1;
        reconciliation.trades.push(PmReadOnlyTradeEvidence {
            trade_id: "trade-foreign-maker".into(),
            condition_id: config.condition_id.clone(),
            token_id: "5678".into(),
            side: "buy".into(),
            size: "5".into(),
            price: "0.5".into(),
            status: "matched".into(),
            order_id: None,
            taker_order_id: None,
            trader_side: None,
            transaction_hash: None,
            fee_rate_bps: None,
            maker_orders: vec![PmReadOnlyTradeMakerEvidence {
                order_id: "foreign-maker".into(),
                token_id: config.token_id.clone(),
                side: "sell".into(),
                price: "0.5".into(),
                matched_amount: "1".into(),
                fee_rate_bps: None,
                maker_address: "0x1000000000000000000000000000000000000001".into(),
            }],
            maker_address: None,
            timestamp: None,
            match_time: None,
            last_update: None,
        });
        assert!(
            PmReadOnlySmokeArtifact::from_collected(
                "aa".repeat(32),
                "pm-host".into(),
                "linux".into(),
                "x86_64".into(),
                &config,
                config_evidence(&config),
                1_800_000_000_000,
                1_800_000_020_000,
                None,
                Some(metadata(&config)),
                Some(account(&config)),
                Some(reconciliation),
                Some(user_stream()),
                teardown(),
            )
            .is_err()
        );
    }

    #[test]
    fn foreign_top_level_trade_without_a_configured_maker_leg_is_not_retained_scope() {
        let config = config();
        let mut reconciliation = reconciliation(&config);
        reconciliation.trade_count = 1;
        reconciliation.trade_owner_bound_count = 1;
        reconciliation.trade_scope_bound_count = 1;
        reconciliation.trades.push(PmReadOnlyTradeEvidence {
            trade_id: "trade-without-our-leg".into(),
            condition_id: config.condition_id.clone(),
            token_id: "5678".into(),
            side: "buy".into(),
            size: "5".into(),
            price: "0.5".into(),
            status: "matched".into(),
            order_id: None,
            taker_order_id: None,
            trader_side: None,
            transaction_hash: None,
            fee_rate_bps: None,
            maker_orders: Vec::new(),
            maker_address: None,
            timestamp: None,
            match_time: None,
            last_update: None,
        });
        assert!(
            PmReadOnlySmokeArtifact::from_collected(
                "aa".repeat(32),
                "pm-host".into(),
                "linux".into(),
                "x86_64".into(),
                &config,
                config_evidence(&config),
                1_800_000_000_000,
                1_800_000_020_000,
                None,
                Some(metadata(&config)),
                Some(account(&config)),
                Some(reconciliation),
                Some(user_stream()),
                teardown(),
            )
            .is_err()
        );
    }

    #[test]
    fn authorization_hash_count_summary_owner_and_teardown_tampering_are_rejected() {
        let original = serde_json::to_value(passing_artifact()).unwrap();
        let mutations: [fn(&mut serde_json::Value); 7] = [
            |value| value["production_order_entry_authorized"] = true.into(),
            |value| value["mutation_requests"] = 1.into(),
            |value| value["metadata"]["market_body_sha256"] = "00".repeat(32).into(),
            |value| value["reconciliation"]["open_order_count"] = 2.into(),
            |value| value["reconciliation"]["open_order_owner_bound_count"] = 0.into(),
            |value| value["summary"]["passed"] = false.into(),
            |value| value["teardown"]["user_stream_task_joined"] = false.into(),
        ];
        for mutate in mutations {
            let mut value = original.clone();
            mutate(&mut value);
            let bytes = serde_json::to_vec(&value).unwrap();
            assert!(verify_pm_read_only_smoke_artifact_bytes(&bytes).is_err());
        }
    }

    #[test]
    fn finalization_rejects_instead_of_sanitizing_mutation_evidence() {
        let mut artifact = passing_artifact();
        artifact.teardown.mutation_roles_constructed = true;
        assert!(artifact.finalize(&config()).is_err());

        let mut artifact = passing_artifact();
        artifact.mutation_requests = 1;
        assert!(artifact.finalize(&config()).is_err());
    }

    #[test]
    fn unknown_fields_and_oversized_input_are_rejected() {
        let mut value = serde_json::to_value(passing_artifact()).unwrap();
        value["private_response_body"] = "forbidden".into();
        assert!(
            verify_pm_read_only_smoke_artifact_bytes(&serde_json::to_vec(&value).unwrap()).is_err()
        );
        assert!(matches!(
            verify_pm_read_only_smoke_artifact_bytes(&vec![
                b' ';
                MAX_PM_READ_ONLY_ARTIFACT_BYTES as usize
                    + 1
            ]),
            Err(PmReadOnlySmokeVerificationError::ArtifactTooLarge)
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn path_verification_requires_owner_only_single_link_artifacts() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let directory = tempfile::tempdir().unwrap();
        let artifact_path = directory.path().join("artifact.json");
        std::fs::write(
            &artifact_path,
            serde_json::to_vec(&passing_artifact()).unwrap(),
        )
        .unwrap();
        std::fs::set_permissions(&artifact_path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(verify_pm_read_only_smoke_path(&artifact_path).is_err());

        std::fs::set_permissions(&artifact_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        verify_pm_read_only_smoke_path(&artifact_path).unwrap();

        let symlink_path = directory.path().join("artifact-symlink.json");
        symlink(&artifact_path, &symlink_path).unwrap();
        assert!(verify_pm_read_only_smoke_path(&symlink_path).is_err());

        let hardlink = directory.path().join("artifact-hardlink.json");
        std::fs::hard_link(&artifact_path, &hardlink).unwrap();
        assert!(verify_pm_read_only_smoke_path(&artifact_path).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn anchored_path_verification_checks_reviewed_config_and_declared_binary() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("reviewed.toml");
        let artifact_path = directory.path().join("artifact.json");
        let reviewed = config();
        std::fs::write(&config_path, toml::to_string(&reviewed).unwrap()).unwrap();

        let mut artifact = passing_artifact();
        artifact.binary_sha256 = current_executable_sha256().unwrap();
        artifact.finalize(&reviewed).unwrap();
        std::fs::write(&artifact_path, serde_json::to_vec(&artifact).unwrap()).unwrap();
        std::fs::set_permissions(&artifact_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            verify_pm_read_only_smoke_path_with_anchors(&artifact_path, &config_path).unwrap(),
            artifact
        );

        let mut other_config = reviewed.clone();
        other_config.credential_slot_id = "different-reviewed-slot".into();
        std::fs::write(&config_path, toml::to_string(&other_config).unwrap()).unwrap();
        assert!(matches!(
            verify_pm_read_only_smoke_path_with_anchors(&artifact_path, &config_path),
            Err(PmReadOnlySmokeVerificationError::ReviewedConfigMismatch)
        ));

        std::fs::write(&config_path, toml::to_string(&reviewed).unwrap()).unwrap();
        let different_binary = passing_artifact();
        std::fs::write(
            &artifact_path,
            serde_json::to_vec(&different_binary).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            verify_pm_read_only_smoke_path_with_anchors(&artifact_path, &config_path),
            Err(PmReadOnlySmokeVerificationError::DeclaredBinaryMismatch)
        ));
    }

    #[test]
    fn verifier_uses_the_live_adapters_distinct_reconciliation_limits() {
        let config = config();
        let mut too_many_orders = reconciliation(&config);
        too_many_orders.open_order_count = MAX_PM_AUTHENTICATED_ORDER_ROWS as u64 + 1;
        assert!(validate_reconciliation(&too_many_orders, &config).is_err());

        let mut too_many_trades = reconciliation(&config);
        too_many_trades.trade_count = MAX_PM_AUTHENTICATED_TRADE_ROWS as u64 + 1;
        assert!(validate_reconciliation(&too_many_trades, &config).is_err());

        let mut too_many_pages = reconciliation(&config);
        too_many_pages.open_order_page_count = MAX_PM_AUTHENTICATED_CUT_PAGES as u64 + 1;
        assert!(validate_reconciliation(&too_many_pages, &config).is_err());
    }

    #[test]
    fn coherent_public_metadata_failure_verifies_but_does_not_pass() {
        let config = config();
        let failure = PmReadOnlyCollectionFailureEvidence::new(
            "public_metadata",
            "transport",
            1_800_000_001_000,
        )
        .unwrap();
        let artifact = PmReadOnlySmokeArtifact::from_collected(
            "aa".repeat(32),
            "pm-host".into(),
            "linux".into(),
            "x86_64".into(),
            &config,
            config_evidence(&config),
            1_800_000_000_000,
            1_800_000_002_000,
            Some(failure),
            None,
            None,
            None,
            None,
            PmReadOnlyTeardownEvidence {
                user_stream_task_started: false,
                user_stream_shutdown_requested: false,
                user_stream_abort_requested: false,
                user_stream_task_joined: false,
                user_stream_task_completed_cleanly: false,
                credential_authority_task_started: false,
                credential_authority_shutdown_requested: false,
                credential_authority_abort_requested: false,
                credential_authority_task_joined: false,
                credential_authority_task_completed_cleanly: false,
                credentials_loaded: true,
                credentials_dropped_before_return: true,
                all_tasks_joined: true,
                mutation_roles_constructed: false,
                mutation_requests: 0,
            },
        )
        .unwrap();
        let verified =
            verify_pm_read_only_smoke_artifact_bytes(&serde_json::to_vec(&artifact).unwrap())
                .unwrap();
        assert!(!verified.summary.passed);
        assert!(matches!(
            require_pm_read_only_smoke_pass(&verified),
            Err(PmReadOnlySmokeVerificationError::NotPassing)
        ));
    }

    #[test]
    fn failure_stage_kind_and_named_gate_must_match_collector_reality() {
        for (stage, kind) in [
            ("credential_loading", "io"),
            ("teardown", "transport"),
            ("user_stream", "io"),
        ] {
            assert!(PmReadOnlyCollectionFailureEvidence::new(stage, kind, 1).is_err());
        }

        let config = config();
        let mut clean = passing_artifact();
        clean.collection_failure = Some(
            PmReadOnlyCollectionFailureEvidence::new(
                "teardown",
                "shutdown",
                clean.finished_unix_ms,
            )
            .unwrap(),
        );
        assert!(clean.finalize(&config).is_err());
    }

    #[test]
    fn pre_open_connection_retirement_is_a_coherent_verified_nonpass() {
        let config = config();
        let mut stream = user_stream();
        stream.connection_open_count = 0;
        stream.subscription_count = 0;
        stream.retirement_count = 1;
        stream.retry_exhausted_count = 1;
        stream.ping_count = 0;
        stream.correlated_pong_count = 0;
        stream.frame_count = 0;
        stream.event_count = 0;
        stream.order_event_count = 0;
        stream.owner_bound_event_count = 0;
        stream.scope_bound_event_count = 0;
        stream.bound_observation_count = 0;
        stream.dwell_ms = 0;
        stream.shutdown_event_count = 0;
        stream.run_completed_without_transport_error = false;
        let failure = PmReadOnlyCollectionFailureEvidence::new(
            "user_stream",
            "reconnect_exhausted",
            1_800_000_020_000,
        )
        .unwrap();
        let artifact = PmReadOnlySmokeArtifact::from_collected(
            "aa".repeat(32),
            "pm-host".into(),
            "linux".into(),
            "x86_64".into(),
            &config,
            config_evidence(&config),
            1_800_000_000_000,
            1_800_000_020_000,
            Some(failure),
            Some(metadata(&config)),
            Some(account(&config)),
            Some(reconciliation(&config)),
            Some(stream),
            teardown(),
        )
        .unwrap();
        let verified =
            verify_pm_read_only_smoke_artifact_bytes(&serde_json::to_vec(&artifact).unwrap())
                .unwrap();
        assert!(!verified.summary.passed);
        assert!(require_pm_read_only_smoke_pass(&verified).is_err());
    }

    #[test]
    fn bounded_credential_abort_join_is_a_verifiable_nonpass_not_a_false_clean_join() {
        let config = config();
        let failure =
            PmReadOnlyCollectionFailureEvidence::new("teardown", "shutdown", 1_800_000_020_000)
                .unwrap();
        let partial_teardown = PmReadOnlyTeardownEvidence {
            user_stream_task_started: true,
            user_stream_shutdown_requested: true,
            user_stream_abort_requested: false,
            user_stream_task_joined: true,
            user_stream_task_completed_cleanly: true,
            credential_authority_task_started: true,
            credential_authority_shutdown_requested: true,
            credential_authority_abort_requested: true,
            credential_authority_task_joined: true,
            credential_authority_task_completed_cleanly: false,
            credentials_loaded: true,
            credentials_dropped_before_return: true,
            all_tasks_joined: true,
            mutation_roles_constructed: false,
            mutation_requests: 0,
        };
        let artifact = PmReadOnlySmokeArtifact::from_collected(
            "aa".repeat(32),
            "pm-host".into(),
            "linux".into(),
            "x86_64".into(),
            &config,
            config_evidence(&config),
            1_800_000_000_000,
            1_800_000_020_000,
            Some(failure),
            Some(metadata(&config)),
            Some(account(&config)),
            Some(reconciliation(&config)),
            Some(user_stream()),
            partial_teardown.clone(),
        )
        .unwrap();

        let verified =
            verify_pm_read_only_smoke_artifact_bytes(&serde_json::to_vec(&artifact).unwrap())
                .unwrap();
        assert!(!verified.summary.teardown_complete);
        assert!(!verified.summary.passed);
        assert!(require_pm_read_only_smoke_pass(&verified).is_err());

        let mut contradictory = partial_teardown;
        contradictory.credential_authority_task_joined = false;
        contradictory.all_tasks_joined = false;
        assert!(validate_teardown(&contradictory, artifact.collection_failure.as_ref(),).is_err());
    }
}
