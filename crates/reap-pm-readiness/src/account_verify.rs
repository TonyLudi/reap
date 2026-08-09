use std::{
    fs::OpenOptions,
    io::Read as _,
    path::{Path, PathBuf},
    str::FromStr as _,
};

use reap_pm_core::{PmAssetId, PmGoalFTradingDomain, U256};
use reap_telemetry::{current_executable_sha256, sha256_bytes};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    MAX_PM_READ_ONLY_CONFIG_BYTES, PmReadOnlyAccountConfig, PmReadOnlyAccountConfigEvidence,
    account_schema::{
        PM_READ_ONLY_ACCOUNT_ARTIFACT_SCHEMA_VERSION, PM_READ_ONLY_ACCOUNT_CONTRACT_VERSION,
        PM_READ_ONLY_ACCOUNT_COVERAGE, PM_READ_ONLY_ACCOUNT_PRODUCT, PmReadOnlyAccountArtifact,
        PmReadOnlyAccountSnapshotEvidence, PmReadOnlyAccountSummary,
        PmReadOnlyAccountTeardownEvidence,
    },
    config::opened_regular_file_metadata_is_stable,
    load_pm_read_only_account_config_path,
};

pub const MAX_PM_READ_ONLY_ACCOUNT_ARTIFACT_BYTES: u64 = 1024 * 1024;

const BINARY_NAME: &str = "reap-pm-readiness";
const MAX_ATTEMPT_MS: u64 = 5 * 60 * 1_000;
const MIN_SUPPORTED_UNIX_MS: u64 = 1_577_836_800_000;
const MAX_SUPPORTED_UNIX_MS: u64 = 32_503_680_000_000;
const BINARY_DOMAIN: &[u8] = b"reap.pm.read-only-account.binary.v1\0";
const HOST_DOMAIN: &[u8] = b"reap.pm.read-only-account.host.v1\0";
const SLOT_DOMAIN: &[u8] = b"reap.pm.read-only-account.credential-slot-nonsecret.v1\0";
const ACCOUNT_DOMAIN: &[u8] = b"reap.pm.read-only-account.snapshot.v1\0";
const ARTIFACT_DOMAIN: &[u8] = b"reap.pm.read-only-account.artifact.v1\0";

pub const PM_READ_ONLY_ACCOUNT_LIMITATIONS: [&str; 6] = [
    "the collateral and conditional responses are sequential reads, not one atomic venue snapshot",
    "raw authenticated HTTP responses are intentionally not retained",
    "the account-only workflow does not inspect public metadata, positions from the Data API, open orders, trades, or user WebSocket events",
    "the exact funder is operator-configured and is not independently returned or bound by the two balance responses",
    "offline verification checks public recomputable consistency and requires an independently trusted collection chain of custody",
    "this read-only artifact grants no mutation, strategy, risk, settlement, demo, or production authority",
];

#[derive(Debug, Error)]
pub enum PmReadOnlyAccountVerificationError {
    #[error("invalid account-only artifact path {path}: {message}")]
    InvalidPath { path: PathBuf, message: String },
    #[error("account-only artifact {path} is {actual} bytes; limit is {limit}")]
    TooLarge {
        path: PathBuf,
        actual: u64,
        limit: u64,
    },
    #[error("failed to read account-only artifact {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("account-only artifact exceeds its fixed byte bound")]
    ArtifactTooLarge,
    #[error("account-only artifact is malformed or has an unknown field")]
    MalformedArtifact,
    #[error("invalid account-only artifact: {0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Config(#[from] crate::PmReadOnlySmokeConfigError),
    #[error("account-only artifact is internally valid but did not pass every gate")]
    NotPassing,
    #[error("the artifact does not match the separately reviewed canonical config")]
    ReviewedConfigMismatch,
    #[error("the artifact's declared binary digest does not match the current verifier executable")]
    DeclaredBinaryMismatch,
    #[error("the current verifier executable digest is unavailable")]
    VerifierDigestUnavailable,
}

fn invalid(message: &'static str) -> PmReadOnlyAccountVerificationError {
    PmReadOnlyAccountVerificationError::Invalid(message)
}

impl PmReadOnlyAccountArtifact {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_collected(
        binary_sha256: String,
        host_name: String,
        host_os: String,
        host_arch: String,
        config: &PmReadOnlyAccountConfig,
        config_evidence: PmReadOnlyAccountConfigEvidence,
        started_unix_ms: u64,
        finished_unix_ms: u64,
        collection_failure: Option<crate::PmReadOnlyCollectionFailureEvidence>,
        account: Option<PmReadOnlyAccountSnapshotEvidence>,
        teardown: PmReadOnlyAccountTeardownEvidence,
    ) -> Result<Self, PmReadOnlyAccountVerificationError> {
        let smoke = config.smoke();
        let mut artifact = Self {
            schema_version: PM_READ_ONLY_ACCOUNT_ARTIFACT_SCHEMA_VERSION,
            product: PM_READ_ONLY_ACCOUNT_PRODUCT.to_owned(),
            coverage: PM_READ_ONLY_ACCOUNT_COVERAGE.to_owned(),
            contract_version: PM_READ_ONLY_ACCOUNT_CONTRACT_VERSION.to_owned(),
            binary_name: BINARY_NAME.to_owned(),
            binary_version: env!("CARGO_PKG_VERSION").to_owned(),
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
            signer_address: smoke.signer_address.clone(),
            funder_address: smoke.funder_address.clone(),
            signature_type: smoke.signature_type,
            production_order_entry_authorized: false,
            mutation_roles_constructed: false,
            mutation_requests: 0,
            collection_failure,
            account,
            teardown,
            limitations: PM_READ_ONLY_ACCOUNT_LIMITATIONS.map(str::to_owned).to_vec(),
            summary: empty_summary(),
            evidence_fingerprint_sha256: String::new(),
        };
        artifact.finalize(config)?;
        Ok(artifact)
    }

    pub(crate) fn finalize(
        &mut self,
        config: &PmReadOnlyAccountConfig,
    ) -> Result<(), PmReadOnlyAccountVerificationError> {
        config.validate()?;
        if self.production_order_entry_authorized
            || self.mutation_roles_constructed
            || self.mutation_requests != 0
            || self.teardown.mutation_roles_constructed
            || self.teardown.mutation_requests != 0
        {
            return Err(invalid(
                "account-only evidence contains mutation authority or activity",
            ));
        }
        if let Some(account) = &mut self.account {
            canonicalize_account(account, config)?;
        }
        self.binary_fingerprint_sha256 = binary_fingerprint(self)?;
        self.host_fingerprint_sha256 = host_fingerprint(self)?;
        self.config_fingerprint_sha256 = config.fingerprint()?;
        self.credential_slot_nonsecret_fingerprint_sha256 = slot_fingerprint(config)?;
        self.limitations = PM_READ_ONLY_ACCOUNT_LIMITATIONS.map(str::to_owned).to_vec();
        self.summary = derive_summary(self, config)?;
        validate_failure_consistency(self)?;
        self.evidence_fingerprint_sha256 = artifact_fingerprint(self)?;
        Ok(())
    }
}

pub fn verify_pm_read_only_account_artifact_bytes(
    bytes: &[u8],
) -> Result<PmReadOnlyAccountArtifact, PmReadOnlyAccountVerificationError> {
    if bytes.len() as u64 > MAX_PM_READ_ONLY_ACCOUNT_ARTIFACT_BYTES {
        return Err(PmReadOnlyAccountVerificationError::ArtifactTooLarge);
    }
    let artifact: PmReadOnlyAccountArtifact = serde_json::from_slice(bytes)
        .map_err(|_| PmReadOnlyAccountVerificationError::MalformedArtifact)?;
    verify_artifact(artifact)
}

pub fn verify_pm_read_only_account_path(
    path: impl AsRef<Path>,
) -> Result<PmReadOnlyAccountArtifact, PmReadOnlyAccountVerificationError> {
    let path = path.as_ref();
    let bytes = read_private_artifact(path)?;
    verify_pm_read_only_account_artifact_bytes(&bytes)
}

pub fn verify_pm_read_only_account_path_with_anchors(
    artifact_path: impl AsRef<Path>,
    reviewed_config_path: impl AsRef<Path>,
) -> Result<PmReadOnlyAccountArtifact, PmReadOnlyAccountVerificationError> {
    let (reviewed, evidence) = load_pm_read_only_account_config_path(reviewed_config_path)?;
    let artifact = verify_pm_read_only_account_path(artifact_path)?;
    if artifact.config != evidence
        || artifact.config_fingerprint_sha256 != reviewed.fingerprint()?
    {
        return Err(PmReadOnlyAccountVerificationError::ReviewedConfigMismatch);
    }
    let digest = current_executable_sha256()
        .map_err(|_| PmReadOnlyAccountVerificationError::VerifierDigestUnavailable)?;
    if artifact.binary_sha256 != digest {
        return Err(PmReadOnlyAccountVerificationError::DeclaredBinaryMismatch);
    }
    Ok(artifact)
}

pub fn require_pm_read_only_account_pass(
    artifact: &PmReadOnlyAccountArtifact,
) -> Result<(), PmReadOnlyAccountVerificationError> {
    if artifact.summary.passed {
        Ok(())
    } else {
        Err(PmReadOnlyAccountVerificationError::NotPassing)
    }
}

fn verify_artifact(
    artifact: PmReadOnlyAccountArtifact,
) -> Result<PmReadOnlyAccountArtifact, PmReadOnlyAccountVerificationError> {
    let config = parse_embedded_config(&artifact)?;
    validate_envelope(&artifact, &config)?;
    let expected = derive_summary(&artifact, &config)?;
    if artifact.summary != expected {
        return Err(invalid(
            "stored account-only summary differs from re-derivation",
        ));
    }
    validate_failure_consistency(&artifact)?;
    if artifact.evidence_fingerprint_sha256 != artifact_fingerprint(&artifact)? {
        return Err(invalid("account-only evidence fingerprint mismatch"));
    }
    Ok(artifact)
}

fn validate_envelope(
    artifact: &PmReadOnlyAccountArtifact,
    config: &PmReadOnlyAccountConfig,
) -> Result<(), PmReadOnlyAccountVerificationError> {
    if artifact.schema_version != PM_READ_ONLY_ACCOUNT_ARTIFACT_SCHEMA_VERSION
        || artifact.product != PM_READ_ONLY_ACCOUNT_PRODUCT
        || artifact.coverage != PM_READ_ONLY_ACCOUNT_COVERAGE
        || artifact.contract_version != PM_READ_ONLY_ACCOUNT_CONTRACT_VERSION
        || artifact.binary_name != BINARY_NAME
        || artifact.binary_version != env!("CARGO_PKG_VERSION")
        || !is_sha256(&artifact.binary_sha256)
        || artifact.binary_fingerprint_sha256 != binary_fingerprint(artifact)?
    {
        return Err(invalid(
            "account-only envelope or binary provenance is invalid",
        ));
    }
    validate_visible(&artifact.host_name, 255)?;
    validate_visible(&artifact.host_os, 128)?;
    validate_visible(&artifact.host_arch, 128)?;
    if artifact.host_fingerprint_sha256 != host_fingerprint(artifact)?
        || artifact.config_fingerprint_sha256 != config.fingerprint()?
        || artifact.credential_slot_nonsecret_fingerprint_sha256 != slot_fingerprint(config)?
        || artifact.started_unix_ms < MIN_SUPPORTED_UNIX_MS
        || artifact.finished_unix_ms < artifact.started_unix_ms
        || artifact.finished_unix_ms > MAX_SUPPORTED_UNIX_MS
        || artifact.finished_unix_ms - artifact.started_unix_ms > MAX_ATTEMPT_MS
    {
        return Err(invalid(
            "account-only provenance, config, or time bounds are invalid",
        ));
    }
    let smoke = config.smoke();
    if artifact.signer_address != smoke.signer_address
        || artifact.funder_address != smoke.funder_address
        || artifact.signature_type != smoke.signature_type
        || artifact.production_order_entry_authorized
        || artifact.mutation_roles_constructed
        || artifact.mutation_requests != 0
        || artifact.limitations != PM_READ_ONLY_ACCOUNT_LIMITATIONS.map(str::to_owned).to_vec()
    {
        return Err(invalid(
            "account-only identity, authorization, or limitations are invalid",
        ));
    }
    if let Some(account) = &artifact.account {
        let mut expected = account.clone();
        canonicalize_account(&mut expected, config)?;
        if &expected != account {
            return Err(invalid("account-only snapshot is not canonical"));
        }
    }
    validate_failure(artifact)?;
    Ok(())
}

fn parse_embedded_config(
    artifact: &PmReadOnlyAccountArtifact,
) -> Result<PmReadOnlyAccountConfig, PmReadOnlyAccountVerificationError> {
    if artifact.config.canonical_bytes != artifact.config.canonical_toml.len() as u64
        || artifact.config.canonical_bytes > MAX_PM_READ_ONLY_CONFIG_BYTES
        || artifact.config.canonical_sha256
            != sha256_bytes(artifact.config.canonical_toml.as_bytes())
    {
        return Err(invalid("embedded account-only config digest is invalid"));
    }
    let config: PmReadOnlyAccountConfig = toml::from_str(&artifact.config.canonical_toml)
        .map_err(|_| invalid("embedded account-only config is malformed"))?;
    config.validate()?;
    if toml::to_string(&config).map_err(|_| invalid("embedded account-only config is malformed"))?
        != artifact.config.canonical_toml
    {
        return Err(invalid("embedded account-only config is not canonical"));
    }
    Ok(config)
}

fn canonicalize_account(
    account: &mut PmReadOnlyAccountSnapshotEvidence,
    config: &PmReadOnlyAccountConfig,
) -> Result<(), PmReadOnlyAccountVerificationError> {
    canonical_u256(&account.collateral_balance)?;
    canonical_u256(&account.conditional_balance)?;
    let token = config.wire_scope()?.token().units().to_string();
    if account.token_id != token || account.authenticated_response_count != 2 {
        return Err(invalid(
            "account-only balance identity or response count is invalid",
        ));
    }
    let domain = PmGoalFTradingDomain::from_metadata(config.expected_metadata()?)
        .map_err(|_| invalid("account-only allowance domain is invalid"))?;
    if account.allowances.len() != domain.required_spenders().len() {
        return Err(invalid("account-only allowance row count is invalid"));
    }
    for (actual, required) in account.allowances.iter().zip(domain.required_spenders()) {
        let (kind, contract, token_id) = match required.asset() {
            PmAssetId::Collateral { contract } => ("collateral", contract, None),
            PmAssetId::Outcome { contract, token } => {
                ("outcome", contract, Some(token.units().to_string()))
            }
        };
        if actual.asset_kind != kind
            || actual.asset_contract != contract.to_string()
            || actual.token_id != token_id
            || actual.spender_address != required.spender().to_string()
        {
            return Err(invalid("account-only allowance scope is invalid"));
        }
        if actual.present {
            canonical_u256(&actual.amount)?;
        } else if !actual.amount.is_empty() {
            return Err(invalid("absent account-only allowance retained an amount"));
        }
    }
    account.allowance_count = account.allowances.len() as u64;
    account.canonical_sha256 = account_fingerprint(account)?;
    Ok(())
}

fn derive_summary(
    artifact: &PmReadOnlyAccountArtifact,
    config: &PmReadOnlyAccountConfig,
) -> Result<PmReadOnlyAccountSummary, PmReadOnlyAccountVerificationError> {
    let configured_profile_structurally_valid = config.validate().is_ok()
        && artifact.signer_address == config.smoke().signer_address
        && artifact.funder_address == config.smoke().funder_address
        && artifact.signature_type == config.signature_type();
    let authorization_closed = !artifact.production_order_entry_authorized
        && !artifact.mutation_roles_constructed
        && artifact.mutation_requests == 0
        && !artifact.teardown.mutation_roles_constructed
        && artifact.teardown.mutation_requests == 0;
    let balances_observed = artifact.account.as_ref().is_some_and(|account| {
        account.authenticated_response_count == 2
            && canonical_u256(&account.collateral_balance).is_ok()
            && canonical_u256(&account.conditional_balance).is_ok()
            && account.token_id == config.smoke().token_id
    });
    let required_allowances_complete = artifact.account.as_ref().is_some_and(|account| {
        account.allowances.iter().all(|allowance| {
            allowance.present
                && !allowance.unscoped_scalar_present
                && canonical_u256(&allowance.amount).is_ok()
        }) && account.allowance_count == account.allowances.len() as u64
            && account.allowances.len() == 2
    });
    let request_surface_exact = artifact.teardown.public_time_attempt_count == 2
        && artifact.teardown.authenticated_balance_attempt_count == 2
        && artifact.teardown.private_reconciliation_request_count == 0
        && artifact.teardown.user_stream_connection_count == 0;
    let teardown_complete = artifact.teardown.credential_authority_task_started
        && artifact.teardown.credential_authority_shutdown_requested
        && !artifact.teardown.credential_authority_abort_requested
        && artifact.teardown.credential_authority_task_joined
        && artifact
            .teardown
            .credential_authority_task_completed_cleanly
        && artifact.teardown.credentials_loaded
        && artifact.teardown.credentials_dropped_before_return
        && artifact.teardown.all_tasks_joined;
    let limitations_explicit =
        artifact.limitations == PM_READ_ONLY_ACCOUNT_LIMITATIONS.map(str::to_owned).to_vec();
    let mut summary = PmReadOnlyAccountSummary {
        provenance_valid: is_sha256(&artifact.binary_sha256)
            && artifact.binary_fingerprint_sha256 == binary_fingerprint(artifact)?
            && artifact.host_fingerprint_sha256 == host_fingerprint(artifact)?,
        config_valid: artifact.config_fingerprint_sha256 == config.fingerprint()?,
        configured_profile_structurally_valid,
        authorization_closed,
        public_time_observed: artifact.teardown.public_time_attempt_count == 2
            && artifact.account.is_some(),
        balances_observed,
        required_allowances_complete,
        request_surface_exact,
        teardown_complete,
        limitations_explicit,
        passed: false,
    };
    summary.passed = summary.provenance_valid
        && summary.config_valid
        && summary.configured_profile_structurally_valid
        && summary.authorization_closed
        && summary.public_time_observed
        && summary.balances_observed
        && summary.required_allowances_complete
        && summary.request_surface_exact
        && summary.teardown_complete
        && summary.limitations_explicit
        && artifact.collection_failure.is_none();
    Ok(summary)
}

fn validate_failure(
    artifact: &PmReadOnlyAccountArtifact,
) -> Result<(), PmReadOnlyAccountVerificationError> {
    if let Some(failure) = &artifact.collection_failure
        && (!(match failure.stage.as_str() {
            "authenticated_account" => matches!(
                failure.kind.as_str(),
                "timeout"
                    | "transport"
                    | "rejected_status"
                    | "malformed_response"
                    | "owner_mismatch"
                    | "insufficient_evidence"
            ),
            "teardown" => failure.kind == "shutdown",
            _ => false,
        }) || failure.occurred_unix_ms < artifact.started_unix_ms
            || failure.occurred_unix_ms > artifact.finished_unix_ms)
    {
        return Err(invalid("account-only failure classification is invalid"));
    }
    Ok(())
}

fn validate_failure_consistency(
    artifact: &PmReadOnlyAccountArtifact,
) -> Result<(), PmReadOnlyAccountVerificationError> {
    if artifact.collection_failure.is_none() != artifact.summary.passed {
        return Err(invalid(
            "account-only failure state contradicts the summary",
        ));
    }
    if let Some(failure) = &artifact.collection_failure {
        let coherent = match failure.stage.as_str() {
            "authenticated_account" => {
                !artifact.summary.public_time_observed
                    || !artifact.summary.balances_observed
                    || !artifact.summary.required_allowances_complete
                    || !artifact.summary.request_surface_exact
            }
            "teardown" => !artifact.summary.teardown_complete,
            _ => false,
        };
        if !coherent {
            return Err(invalid(
                "account-only failure stage contradicts retained evidence",
            ));
        }
    }
    Ok(())
}

fn empty_summary() -> PmReadOnlyAccountSummary {
    PmReadOnlyAccountSummary {
        provenance_valid: false,
        config_valid: false,
        configured_profile_structurally_valid: false,
        authorization_closed: false,
        public_time_observed: false,
        balances_observed: false,
        required_allowances_complete: false,
        request_surface_exact: false,
        teardown_complete: false,
        limitations_explicit: false,
        passed: false,
    }
}

fn binary_fingerprint(
    artifact: &PmReadOnlyAccountArtifact,
) -> Result<String, PmReadOnlyAccountVerificationError> {
    hash_serialized(
        BINARY_DOMAIN,
        &(
            &artifact.binary_name,
            &artifact.binary_version,
            &artifact.binary_sha256,
        ),
    )
}

fn host_fingerprint(
    artifact: &PmReadOnlyAccountArtifact,
) -> Result<String, PmReadOnlyAccountVerificationError> {
    hash_serialized(
        HOST_DOMAIN,
        &(&artifact.host_name, &artifact.host_os, &artifact.host_arch),
    )
}

fn slot_fingerprint(
    config: &PmReadOnlyAccountConfig,
) -> Result<String, PmReadOnlyAccountVerificationError> {
    hash_serialized(SLOT_DOMAIN, &config.smoke().credential_slot_id)
}

fn account_fingerprint(
    account: &PmReadOnlyAccountSnapshotEvidence,
) -> Result<String, PmReadOnlyAccountVerificationError> {
    let mut projected = account.clone();
    projected.canonical_sha256.clear();
    hash_serialized(ACCOUNT_DOMAIN, &projected)
}

fn artifact_fingerprint(
    artifact: &PmReadOnlyAccountArtifact,
) -> Result<String, PmReadOnlyAccountVerificationError> {
    let mut projected = artifact.clone();
    projected.evidence_fingerprint_sha256.clear();
    hash_serialized(ARTIFACT_DOMAIN, &projected)
}

fn hash_serialized<T: Serialize>(
    domain: &[u8],
    value: &T,
) -> Result<String, PmReadOnlyAccountVerificationError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| invalid("account-only evidence could not be canonicalized"))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    Ok(hex_lower(&hasher.finalize()))
}

fn canonical_u256(value: &str) -> Result<(), PmReadOnlyAccountVerificationError> {
    let parsed = U256::from_str(value).map_err(|_| invalid("account-only amount is invalid"))?;
    if parsed.to_string() != value {
        return Err(invalid("account-only amount is not canonical decimal"));
    }
    Ok(())
}

fn validate_visible(value: &str, max: usize) -> Result<(), PmReadOnlyAccountVerificationError> {
    if value.is_empty()
        || value.len() > max
        || !value.is_ascii()
        || value.trim_ascii() != value
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == 0x7f)
    {
        Err(invalid("account-only visible provenance field is invalid"))
    } else {
        Ok(())
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn read_private_artifact(path: &Path) -> Result<Vec<u8>, PmReadOnlyAccountVerificationError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    }
    let mut file =
        options
            .open(path)
            .map_err(|source| PmReadOnlyAccountVerificationError::Read {
                path: path.to_path_buf(),
                source,
            })?;
    let metadata = file
        .metadata()
        .map_err(|source| PmReadOnlyAccountVerificationError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    if !metadata.is_file() || !private_metadata(&metadata) {
        return Err(PmReadOnlyAccountVerificationError::InvalidPath {
            path: path.to_path_buf(),
            message: "must be an owner-owned, single-link, mode-0600 regular file".to_owned(),
        });
    }
    if metadata.len() > MAX_PM_READ_ONLY_ACCOUNT_ARTIFACT_BYTES {
        return Err(PmReadOnlyAccountVerificationError::TooLarge {
            path: path.to_path_buf(),
            actual: metadata.len(),
            limit: MAX_PM_READ_ONLY_ACCOUNT_ARTIFACT_BYTES,
        });
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.by_ref()
        .take(MAX_PM_READ_ONLY_ACCOUNT_ARTIFACT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| PmReadOnlyAccountVerificationError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    let after = file
        .metadata()
        .map_err(|source| PmReadOnlyAccountVerificationError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() as u64 > MAX_PM_READ_ONLY_ACCOUNT_ARTIFACT_BYTES
        || !opened_regular_file_metadata_is_stable(&metadata, &after, bytes.len() as u64)
        || !private_metadata(&after)
    {
        return Err(PmReadOnlyAccountVerificationError::InvalidPath {
            path: path.to_path_buf(),
            message: "changed while being read or exceeded its bound".to_owned(),
        });
    }
    Ok(bytes)
}

fn private_metadata(metadata: &std::fs::Metadata) -> bool {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt as _;
        metadata.mode() & 0o7777 == 0o600
            && metadata.nlink() == 1
            && effective_uid().is_some_and(|uid| uid == metadata.uid())
    }
    #[cfg(not(target_os = "linux"))]
    {
        metadata.is_file()
    }
}

#[cfg(target_os = "linux")]
fn effective_uid() -> Option<u32> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|line| line.starts_with("Uid:"))?
        .split_ascii_whitespace()
        .nth(2)?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PmReadOnlyAllowanceEvidence, PmReadOnlyConfigEvidence, PmReadOnlySmokeConfig,
        account_schema::PmReadOnlyAccountTeardownEvidence,
    };

    fn config() -> (PmReadOnlyAccountConfig, PmReadOnlyConfigEvidence) {
        let smoke = PmReadOnlySmokeConfig {
            schema_version: 1,
            credential_slot_id: "proxy-account-v1".into(),
            signer_address: "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266".into(),
            funder_address: "0x1000000000000000000000000000000000000001".into(),
            chain_id: 137,
            signature_type: 1,
            condition_id: format!("0x{}", "11".repeat(32)),
            market_id: format!("0x{}", "22".repeat(32)),
            token_id: "1234".into(),
            outcome: "Yes".into(),
            tick: "0.01".into(),
            minimum_order_size: "5".into(),
            negative_risk: false,
            connect_timeout_ms: 5_000,
            request_timeout_ms: 10_000,
            user_stream_dwell_ms: 15_000,
            user_stream_idle_timeout_ms: 30_000,
            user_stream_pong_timeout_ms: 5_000,
            user_stream_max_reconnect_attempts: 1,
            user_stream_reconnect_backoff_ms: 500,
            user_stream_event_channel_capacity: 64,
            api_key_file: "api-key".into(),
            secret_file: "secret".into(),
            passphrase_file: "passphrase".into(),
        };
        let canonical_toml = toml::to_string(&smoke).unwrap();
        let account: PmReadOnlyAccountConfig = toml::from_str(&canonical_toml).unwrap();
        account.validate().unwrap();
        let evidence = PmReadOnlyConfigEvidence {
            canonical_bytes: canonical_toml.len() as u64,
            canonical_sha256: sha256_bytes(canonical_toml.as_bytes()),
            canonical_toml,
        };
        (account, evidence)
    }

    fn passing_artifact() -> PmReadOnlyAccountArtifact {
        let (config, evidence) = config();
        let domain =
            PmGoalFTradingDomain::from_metadata(config.expected_metadata().unwrap()).unwrap();
        let allowances = domain
            .required_spenders()
            .iter()
            .map(|requirement| {
                let (asset_kind, asset_contract, token_id) = match requirement.asset() {
                    PmAssetId::Collateral { contract } => ("collateral", contract, None),
                    PmAssetId::Outcome { contract, token } => {
                        ("outcome", contract, Some(token.units().to_string()))
                    }
                };
                PmReadOnlyAllowanceEvidence {
                    asset_kind: asset_kind.into(),
                    asset_contract: asset_contract.to_string(),
                    token_id,
                    spender_address: requirement.spender().to_string(),
                    amount: "42".into(),
                    present: true,
                    unscoped_scalar_present: false,
                }
            })
            .collect();
        PmReadOnlyAccountArtifact::from_collected(
            "a".repeat(64),
            "test-host".into(),
            std::env::consts::OS.into(),
            std::env::consts::ARCH.into(),
            &config,
            evidence,
            1_800_000_000_000,
            1_800_000_000_100,
            None,
            Some(PmReadOnlyAccountSnapshotEvidence {
                authenticated_response_count: 2,
                collateral_balance: "100".into(),
                conditional_balance: "7".into(),
                token_id: "1234".into(),
                allowances,
                allowance_count: 0,
                canonical_sha256: String::new(),
            }),
            PmReadOnlyAccountTeardownEvidence {
                public_time_attempt_count: 2,
                authenticated_balance_attempt_count: 2,
                private_reconciliation_request_count: 0,
                user_stream_connection_count: 0,
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
            },
        )
        .unwrap()
    }

    #[test]
    fn proxy_account_artifact_round_trips_and_passes() {
        let artifact = passing_artifact();
        assert!(artifact.summary.passed);
        assert!(artifact.summary.configured_profile_structurally_valid);
        assert_eq!(artifact.teardown.public_time_attempt_count, 2);
        assert!(
            artifact
                .limitations
                .iter()
                .any(|value| { value.contains("exact funder is operator-configured") })
        );
        let bytes = serde_json::to_vec(&artifact).unwrap();
        assert_eq!(
            verify_pm_read_only_account_artifact_bytes(&bytes).unwrap(),
            artifact
        );
    }

    #[test]
    fn verifier_rejects_request_surface_and_funder_tampering() {
        let mut requests = passing_artifact();
        requests.teardown.public_time_attempt_count = 1;
        assert!(
            verify_pm_read_only_account_artifact_bytes(&serde_json::to_vec(&requests).unwrap())
                .is_err()
        );

        let mut identity = passing_artifact();
        identity.funder_address = "0x2000000000000000000000000000000000000002".into();
        assert!(
            verify_pm_read_only_account_artifact_bytes(&serde_json::to_vec(&identity).unwrap())
                .is_err()
        );
    }
}
