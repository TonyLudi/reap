use std::collections::BTreeSet;
use std::fs::{DirBuilder, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reap_core::PINNED_JAVA_REVISION;
use reap_venue::okx::{
    HttpTransport, OkxAccountConfig, OkxAccountLevel, OkxFillPagination, OkxPositionMode,
    OkxRestClient, OkxSigner, ReqwestTransport, RestError, parse_okx_fill_page_response_json,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::provenance::{
    current_executable_sha256, host_identity_sha256, okx_account_identity_sha256,
};
use crate::{LiveConfig, LiveConfigError, TradingEnvironment};

pub const FILL_COLLECTION_SCHEMA_VERSION: u32 = 1;
pub const FILL_COLLECTION_MANIFEST_NAME: &str = "manifest.json";
pub const MAX_FILL_COLLECTION_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_FILL_COLLECTION_CONFIG_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_FILL_COLLECTION_PAGES: usize = 1_000;
pub const MAX_FILL_COLLECTION_PAGE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_FILL_COLLECTION_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
pub const OKX_RECENT_FILLS_RETENTION_MS: u64 = 72 * 60 * 60 * 1_000;
pub const MAX_FILL_COLLECTION_WINDOW_AGE_MS: u64 = 70 * 60 * 60 * 1_000;
pub const MIN_FILL_COLLECTION_PAGE_INTERVAL_MS: u64 = 200;
pub const MAX_FILL_COLLECTION_PAGE_INTERVAL_MS: u64 = 60_000;
pub const MAX_FILL_COLLECTION_CLOSE_DELAY_MS: u64 = 10 * 60 * 1_000;

#[derive(Debug, Clone)]
pub struct FillCollectionOptions {
    pub account_id: String,
    pub begin_ms: u64,
    pub end_ms: u64,
    pub max_pages: usize,
    pub page_interval_ms: u64,
    pub minimum_window_close_delay_ms: u64,
}

impl FillCollectionOptions {
    fn validate(&self) -> Result<(), FillCollectionError> {
        if self.account_id.is_empty() || self.account_id.trim() != self.account_id {
            return Err(FillCollectionError::InvalidOptions(
                "account id must be non-empty and contain no surrounding whitespace".to_string(),
            ));
        }
        if self.account_id.len() > 128 {
            return Err(FillCollectionError::InvalidOptions(
                "account id exceeds 128 bytes".to_string(),
            ));
        }
        if self.begin_ms > self.end_ms {
            return Err(FillCollectionError::InvalidOptions(
                "begin-ms must be less than or equal to end-ms".to_string(),
            ));
        }
        if self.max_pages == 0 || self.max_pages > MAX_FILL_COLLECTION_PAGES {
            return Err(FillCollectionError::InvalidOptions(format!(
                "max-pages must be in 1..={MAX_FILL_COLLECTION_PAGES}"
            )));
        }
        if !(MIN_FILL_COLLECTION_PAGE_INTERVAL_MS..=MAX_FILL_COLLECTION_PAGE_INTERVAL_MS)
            .contains(&self.page_interval_ms)
        {
            return Err(FillCollectionError::InvalidOptions(format!(
                "page-interval-ms must be in {MIN_FILL_COLLECTION_PAGE_INTERVAL_MS}..={MAX_FILL_COLLECTION_PAGE_INTERVAL_MS}"
            )));
        }
        if self.minimum_window_close_delay_ms == 0
            || self.minimum_window_close_delay_ms > MAX_FILL_COLLECTION_CLOSE_DELAY_MS
        {
            return Err(FillCollectionError::InvalidOptions(format!(
                "minimum-window-close-delay-ms must be in 1..={MAX_FILL_COLLECTION_CLOSE_DELAY_MS}"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillCollectionCoverage {
    CompleteOkxRecentFills,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FillCollectionFileEvidence {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FillCollectionWindow {
    pub begin_ms: u64,
    pub end_ms: u64,
    pub endpoints_inclusive: bool,
    pub minimum_close_delay_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FillCollectionClockEvidence {
    pub local_midpoint_ms: u64,
    pub server_ms: u64,
    pub absolute_skew_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FillCollectionPageEvidence {
    pub page_index: u64,
    pub request_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_after: Option<String>,
    pub rows: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_fill_time_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_fill_time_ms: Option<u64>,
    pub response: FillCollectionFileEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FillCollectionManifest {
    pub schema_version: u32,
    pub coverage: FillCollectionCoverage,
    pub java_reference_revision: String,
    pub reap_version: String,
    pub executable_sha256: String,
    pub host_identity_sha256: String,
    pub config_file: FillCollectionFileEvidence,
    pub config_fingerprint: String,
    pub environment: TradingEnvironment,
    pub account_id: String,
    pub account_identity_sha256: String,
    pub account_level: OkxAccountLevel,
    pub position_mode: OkxPositionMode,
    pub endpoint: String,
    pub retention_ms: u64,
    pub maximum_window_age_ms: u64,
    pub window: FillCollectionWindow,
    pub max_pages: u64,
    pub page_interval_ms: u64,
    pub start_clock: FillCollectionClockEvidence,
    pub finish_clock: FillCollectionClockEvidence,
    pub pages: Vec<FillCollectionPageEvidence>,
    pub total_rows: u64,
    pub window_rows: u64,
    pub total_response_bytes: u64,
    pub account_identity_sampled_before_and_after: bool,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VerifiedFillCollection {
    pub manifest_file: FillCollectionFileEvidence,
    pub manifest: FillCollectionManifest,
    pub page_paths: Vec<PathBuf>,
}

#[derive(Debug, Error)]
pub enum FillCollectionError {
    #[error("invalid fill-collection options: {0}")]
    InvalidOptions(String),
    #[error("failed to reserve fill-collection directory {path}: {source}")]
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
    #[error("live config failed validation: {0}")]
    Config(#[from] LiveConfigError),
    #[error("configured account {0} does not exist")]
    UnknownAccount(String),
    #[error("failed to fingerprint collector provenance: {0}")]
    Provenance(String),
    #[error("failed to initialize OKX transport: {0}")]
    Transport(#[source] RestError),
    #[error("OKX fill collection failed: {0}")]
    Rest(#[from] RestError),
    #[error("exchange clock evidence is invalid: {0}")]
    Clock(String),
    #[error("exchange account identity evidence is invalid: {0}")]
    AccountIdentity(String),
    #[error("fill response page {page} is {actual} bytes; limit is {limit}")]
    PageTooLarge { page: u64, actual: u64, limit: u64 },
    #[error("fill response pages total {actual} bytes; aggregate limit is {limit}")]
    PagesTooLarge { actual: u64, limit: u64 },
    #[error("failed to write fill-collection file {path}: {source}")]
    WriteOutput {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize fill-collection manifest: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("invalid fill-collection evidence: {0}")]
    InvalidEvidence(String),
    #[error("invalid {label} path {path}: {message}")]
    InvalidEvidencePath {
        label: &'static str,
        path: PathBuf,
        message: String,
    },
    #[error("failed to read {label} {path}: {source}")]
    ReadEvidence {
        label: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{label} {path} is {actual} bytes; limit is {limit}")]
    EvidenceTooLarge {
        label: &'static str,
        path: PathBuf,
        actual: u64,
        limit: u64,
    },
    #[error("failed to parse fill-collection manifest {path}: {source}")]
    ParseManifest {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to parse collected fill page {path}: {source}")]
    ParsePage {
        path: PathBuf,
        #[source]
        source: RestError,
    },
}

#[derive(Debug, Clone)]
struct CollectionProvenance {
    executable_sha256: String,
    host_identity_sha256: String,
}

/// Collects complete recent fills for one configured account without order entry.
///
/// The output directory is reserved before config parsing, credentials, or
/// network access. A failed collection intentionally leaves a partial directory
/// without a complete manifest, which must never be accepted as evidence.
pub async fn collect_recent_okx_fills_paths(
    config_path: impl AsRef<Path>,
    output_directory: impl AsRef<Path>,
    options: FillCollectionOptions,
) -> Result<FillCollectionManifest, FillCollectionError> {
    options.validate()?;
    let output_directory = reserve_output_directory(output_directory.as_ref())?;
    let (config_file, config_bytes) = read_regular_file(config_path.as_ref(), "live config")?;
    let config_text = std::str::from_utf8(&config_bytes).map_err(|error| {
        FillCollectionError::InvalidConfigPath {
            path: PathBuf::from(&config_file.path),
            message: format!("config is not valid UTF-8: {error}"),
        }
    })?;
    let config = LiveConfig::from_toml(config_text)?;
    let account = config
        .account(&options.account_id)
        .ok_or_else(|| FillCollectionError::UnknownAccount(options.account_id.clone()))?;
    let credentials = account.credentials_from_env()?;
    let provenance = CollectionProvenance {
        executable_sha256: current_executable_sha256().map_err(FillCollectionError::Provenance)?,
        host_identity_sha256: host_identity_sha256().map_err(FillCollectionError::Provenance)?,
    };
    let transport = ReqwestTransport::with_timeouts(
        &config.venue.rest_url,
        Duration::from_millis(config.runtime.rest_connect_timeout_ms),
        Duration::from_millis(config.runtime.rest_request_timeout_ms),
    )
    .map_err(FillCollectionError::Transport)?;
    let signer = OkxSigner::new(credentials, config.venue.environment.is_demo());
    let client = OkxRestClient::new(transport, signer);
    let manifest = collect_recent_okx_fills_with_client(
        &client,
        &config,
        config_file,
        &output_directory,
        &options,
        provenance,
    )
    .await?;
    write_manifest(&output_directory, &manifest)?;
    sync_directory(&output_directory)?;
    Ok(manifest)
}

/// Verifies a complete authenticated recent-fill collection without credentials.
///
/// Every referenced file is reopened, bounded, hashed, and parsed. Pagination is
/// replayed from the raw responses so a manifest cannot silently omit a full
/// page, reorder pages, or substitute a different cursor chain.
pub fn verify_fill_collection_manifest_path(
    manifest_path: impl AsRef<Path>,
) -> Result<VerifiedFillCollection, FillCollectionError> {
    let manifest_path = manifest_path.as_ref();
    let (manifest_file, manifest_bytes, canonical_manifest_path) = read_evidence_file(
        manifest_path,
        "fill-collection manifest",
        MAX_FILL_COLLECTION_MANIFEST_BYTES,
    )?;
    let manifest: FillCollectionManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|source| {
            FillCollectionError::ParseManifest {
                path: canonical_manifest_path.clone(),
                source,
            }
        })?;
    validate_manifest_header(&manifest)?;

    let config_path = PathBuf::from(&manifest.config_file.path);
    let (config_file, config_bytes, canonical_config_path) = read_evidence_file(
        &config_path,
        "referenced live config",
        MAX_FILL_COLLECTION_CONFIG_BYTES,
    )?;
    if canonical_config_path == canonical_manifest_path {
        return invalid_evidence("manifest resolves to its referenced live config");
    }
    if config_file != manifest.config_file {
        return invalid_evidence(format!(
            "referenced live config evidence changed: expected {:?}, observed {:?}",
            manifest.config_file, config_file
        ));
    }
    let config_text = std::str::from_utf8(&config_bytes).map_err(|error| {
        FillCollectionError::InvalidEvidence(format!(
            "referenced live config is not valid UTF-8: {error}"
        ))
    })?;
    let config = LiveConfig::from_toml(config_text)?;
    if config.fingerprint()? != manifest.config_fingerprint {
        return invalid_evidence("referenced live config fingerprint does not match the manifest");
    }
    if config.venue.environment != manifest.environment {
        return invalid_evidence("referenced live config environment does not match the manifest");
    }
    let account = config
        .account(&manifest.account_id)
        .ok_or_else(|| FillCollectionError::UnknownAccount(manifest.account_id.clone()))?;
    if account.expected_account_level != manifest.account_level {
        return invalid_evidence(
            "referenced live config account level does not match the manifest",
        );
    }
    if account.expected_position_mode != manifest.position_mode {
        return invalid_evidence(
            "referenced live config position mode does not match the manifest",
        );
    }
    validate_clock_evidence(
        "start",
        &manifest.start_clock,
        config.runtime.max_exchange_clock_skew_ms,
    )?;
    validate_clock_evidence(
        "finish",
        &manifest.finish_clock,
        config.runtime.max_exchange_clock_skew_ms,
    )?;
    if manifest.finish_clock.server_ms < manifest.start_clock.server_ms {
        return invalid_evidence("exchange server time regressed during collection");
    }
    if manifest.finish_clock.local_midpoint_ms < manifest.start_clock.local_midpoint_ms {
        return invalid_evidence("local midpoint time regressed during collection");
    }
    let latest_allowed_end = manifest
        .start_clock
        .server_ms
        .saturating_sub(manifest.window.minimum_close_delay_ms);
    if manifest.window.end_ms > latest_allowed_end {
        return invalid_evidence("collection window was not closed before collection began");
    }
    let oldest_allowed_begin = manifest
        .finish_clock
        .server_ms
        .saturating_sub(manifest.maximum_window_age_ms);
    if manifest.window.begin_ms < oldest_allowed_begin {
        return invalid_evidence("collection window exceeds the conservative recent-fill age");
    }

    let max_pages = usize::try_from(manifest.max_pages)
        .map_err(|_| FillCollectionError::InvalidEvidence("max_pages exceeds usize".to_string()))?;
    if manifest.pages.len() > max_pages {
        return invalid_evidence(format!(
            "manifest contains {} pages but max_pages is {}",
            manifest.pages.len(),
            max_pages
        ));
    }
    let mut pagination = OkxFillPagination::new(max_pages)?;
    let mut page_paths = Vec::with_capacity(manifest.pages.len());
    let mut seen_paths = BTreeSet::new();
    seen_paths.insert(canonical_manifest_path.clone());
    seen_paths.insert(canonical_config_path);
    let mut total_response_bytes = 0_u64;

    for (offset, evidence) in manifest.pages.iter().enumerate() {
        let expected_index = offset as u64 + 1;
        if evidence.page_index != expected_index {
            return invalid_evidence(format!(
                "page at offset {offset} has index {}; expected {expected_index}",
                evidence.page_index
            ));
        }
        let expected_after = pagination.after();
        if evidence.requested_after.as_deref() != expected_after {
            return invalid_evidence(format!(
                "page {expected_index} requested_after does not match the derived cursor"
            ));
        }
        let expected_request_path = recent_fills_request_path(expected_after);
        if evidence.request_path != expected_request_path {
            return invalid_evidence(format!(
                "page {expected_index} request path is {:?}; expected {:?}",
                evidence.request_path, expected_request_path
            ));
        }

        let response_path = PathBuf::from(&evidence.response.path);
        let (observed, bytes, canonical_response_path) = read_evidence_file(
            &response_path,
            "collected fill page",
            MAX_FILL_COLLECTION_PAGE_BYTES,
        )?;
        if !seen_paths.insert(canonical_response_path.clone()) {
            return invalid_evidence(format!(
                "page {expected_index} resolves to a duplicate evidence path {}",
                canonical_response_path.display()
            ));
        }
        if observed != evidence.response {
            return invalid_evidence(format!(
                "page {expected_index} response evidence changed: expected {:?}, observed {:?}",
                evidence.response, observed
            ));
        }
        total_response_bytes = total_response_bytes
            .checked_add(observed.bytes)
            .ok_or_else(|| {
                FillCollectionError::InvalidEvidence(
                    "aggregate response byte count overflowed".to_string(),
                )
            })?;
        if total_response_bytes > MAX_FILL_COLLECTION_TOTAL_BYTES {
            return Err(FillCollectionError::PagesTooLarge {
                actual: total_response_bytes,
                limit: MAX_FILL_COLLECTION_TOTAL_BYTES,
            });
        }
        let page = parse_okx_fill_page_response_json(&bytes).map_err(|source| {
            FillCollectionError::ParsePage {
                path: canonical_response_path.clone(),
                source,
            }
        })?;
        let rows = page.fills.len() as u64;
        let minimum_fill_time_ms = page.fills.iter().map(|fill| fill.ts_ms).min();
        let maximum_fill_time_ms = page.fills.iter().map(|fill| fill.ts_ms).max();
        if rows != evidence.rows
            || page.next_after != evidence.next_after
            || minimum_fill_time_ms != evidence.minimum_fill_time_ms
            || maximum_fill_time_ms != evidence.maximum_fill_time_ms
        {
            return invalid_evidence(format!(
                "page {expected_index} parsed row, cursor, or timestamp evidence does not match"
            ));
        }
        let terminal = pagination.accept(page)?;
        let final_page = offset + 1 == manifest.pages.len();
        if terminal != final_page {
            return invalid_evidence(if terminal {
                format!("page {expected_index} is short but is not the final page")
            } else {
                format!("final page {expected_index} is full; pagination is incomplete")
            });
        }
        page_paths.push(canonical_response_path);
    }

    let fills = pagination.into_fills();
    let total_rows = fills.len() as u64;
    let window_rows = fills
        .iter()
        .filter(|fill| (manifest.window.begin_ms..=manifest.window.end_ms).contains(&fill.ts_ms))
        .count() as u64;
    if total_rows != manifest.total_rows
        || window_rows != manifest.window_rows
        || total_response_bytes != manifest.total_response_bytes
    {
        return invalid_evidence(
            "manifest aggregate row, window-row, or response-byte evidence does not match",
        );
    }

    Ok(VerifiedFillCollection {
        manifest_file,
        manifest,
        page_paths,
    })
}

fn validate_manifest_header(manifest: &FillCollectionManifest) -> Result<(), FillCollectionError> {
    if manifest.schema_version != FILL_COLLECTION_SCHEMA_VERSION {
        return invalid_evidence(format!(
            "schema version {} is unsupported; expected {FILL_COLLECTION_SCHEMA_VERSION}",
            manifest.schema_version
        ));
    }
    if manifest.java_reference_revision != PINNED_JAVA_REVISION {
        return invalid_evidence("pinned Java revision does not match this verifier");
    }
    if manifest.reap_version.trim().is_empty() {
        return invalid_evidence("collector Reap version is empty");
    }
    for (label, digest) in [
        ("collector executable", manifest.executable_sha256.as_str()),
        (
            "collector host identity",
            manifest.host_identity_sha256.as_str(),
        ),
        ("config fingerprint", manifest.config_fingerprint.as_str()),
        (
            "account identity",
            manifest.account_identity_sha256.as_str(),
        ),
    ] {
        if !is_lower_sha256(digest) {
            return invalid_evidence(format!("{label} SHA-256 is not lowercase hexadecimal"));
        }
    }
    if manifest.account_id.is_empty() || manifest.account_id.trim() != manifest.account_id {
        return invalid_evidence("account id is empty or has surrounding whitespace");
    }
    if manifest.account_id.len() > 128 {
        return invalid_evidence("account id exceeds 128 bytes");
    }
    if manifest.endpoint != "/api/v5/trade/fills" {
        return invalid_evidence("manifest endpoint is not the authenticated recent-fill endpoint");
    }
    if manifest.retention_ms != OKX_RECENT_FILLS_RETENTION_MS
        || manifest.maximum_window_age_ms != MAX_FILL_COLLECTION_WINDOW_AGE_MS
    {
        return invalid_evidence("manifest retention bounds do not match this verifier");
    }
    if manifest.window.begin_ms > manifest.window.end_ms || !manifest.window.endpoints_inclusive {
        return invalid_evidence("manifest window is invalid or is not inclusive");
    }
    if manifest.window.minimum_close_delay_ms == 0
        || manifest.window.minimum_close_delay_ms > MAX_FILL_COLLECTION_CLOSE_DELAY_MS
    {
        return invalid_evidence("manifest window close delay is outside supported bounds");
    }
    if manifest.max_pages == 0 || manifest.max_pages > MAX_FILL_COLLECTION_PAGES as u64 {
        return invalid_evidence("manifest max_pages is outside supported bounds");
    }
    if !(MIN_FILL_COLLECTION_PAGE_INTERVAL_MS..=MAX_FILL_COLLECTION_PAGE_INTERVAL_MS)
        .contains(&manifest.page_interval_ms)
    {
        return invalid_evidence("manifest page interval is outside supported bounds");
    }
    if manifest.pages.is_empty() {
        return invalid_evidence("manifest contains no response pages");
    }
    if !manifest.account_identity_sampled_before_and_after {
        return invalid_evidence("account identity was not sampled before and after collection");
    }
    if !manifest.complete {
        return invalid_evidence("collection is not marked complete");
    }
    Ok(())
}

fn validate_clock_evidence(
    label: &str,
    clock: &FillCollectionClockEvidence,
    maximum_skew_ms: u64,
) -> Result<(), FillCollectionError> {
    let derived_skew = clock.local_midpoint_ms.abs_diff(clock.server_ms);
    if clock.absolute_skew_ms != derived_skew {
        return invalid_evidence(format!(
            "{label} clock absolute skew does not match its timestamps"
        ));
    }
    if derived_skew > maximum_skew_ms {
        return invalid_evidence(format!(
            "{label} clock skew {derived_skew} ms exceeds configured limit {maximum_skew_ms} ms"
        ));
    }
    Ok(())
}

fn recent_fills_request_path(after: Option<&str>) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    if let Some(after) = after {
        serializer.append_pair("after", after);
    }
    serializer.append_pair("limit", "100");
    format!("/api/v5/trade/fills?{}", serializer.finish())
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn invalid_evidence<T>(message: impl Into<String>) -> Result<T, FillCollectionError> {
    Err(FillCollectionError::InvalidEvidence(message.into()))
}

async fn collect_recent_okx_fills_with_client<T>(
    client: &OkxRestClient<T>,
    config: &LiveConfig,
    config_file: FillCollectionFileEvidence,
    output_directory: &Path,
    options: &FillCollectionOptions,
    provenance: CollectionProvenance,
) -> Result<FillCollectionManifest, FillCollectionError>
where
    T: HttpTransport,
{
    let account = config
        .account(&options.account_id)
        .ok_or_else(|| FillCollectionError::UnknownAccount(options.account_id.clone()))?;
    let start_clock = sample_clock(client, config.runtime.max_exchange_clock_skew_ms).await?;
    let latest_allowed_end = start_clock
        .server_ms
        .saturating_sub(options.minimum_window_close_delay_ms);
    if options.end_ms > latest_allowed_end {
        return Err(FillCollectionError::Clock(format!(
            "end-ms {} must be at or before {} after the required {} ms close delay",
            options.end_ms, latest_allowed_end, options.minimum_window_close_delay_ms
        )));
    }
    let oldest_allowed_begin = start_clock
        .server_ms
        .saturating_sub(MAX_FILL_COLLECTION_WINDOW_AGE_MS);
    if options.begin_ms < oldest_allowed_begin {
        return Err(FillCollectionError::Clock(format!(
            "begin-ms {} is older than conservative recent-fill boundary {}",
            options.begin_ms, oldest_allowed_begin
        )));
    }

    let account_before = client.account_config().await?;
    validate_account_config(account, &account_before)?;
    let account_identity_sha256 = account_identity(config, &options.account_id, &account_before)?;

    let mut pagination = OkxFillPagination::new(options.max_pages)?;
    let mut pages = Vec::new();
    let mut total_response_bytes = 0_u64;
    let mut page_index = 0_u64;
    loop {
        if page_index > 0 {
            tokio::time::sleep(Duration::from_millis(options.page_interval_ms)).await;
        }
        page_index += 1;
        let requested_after = pagination.after().map(str::to_string);
        let raw = client
            .fills_page_raw(None, None, pagination.after())
            .await?;
        let bytes = raw.response_body.as_bytes();
        let bytes_len = bytes.len() as u64;
        if bytes_len > MAX_FILL_COLLECTION_PAGE_BYTES {
            return Err(FillCollectionError::PageTooLarge {
                page: page_index,
                actual: bytes_len,
                limit: MAX_FILL_COLLECTION_PAGE_BYTES,
            });
        }
        total_response_bytes = total_response_bytes.saturating_add(bytes_len);
        if total_response_bytes > MAX_FILL_COLLECTION_TOTAL_BYTES {
            return Err(FillCollectionError::PagesTooLarge {
                actual: total_response_bytes,
                limit: MAX_FILL_COLLECTION_TOTAL_BYTES,
            });
        }
        let rows = raw.page.fills.len() as u64;
        let minimum_fill_time_ms = raw.page.fills.iter().map(|fill| fill.ts_ms).min();
        let maximum_fill_time_ms = raw.page.fills.iter().map(|fill| fill.ts_ms).max();
        let next_after = raw.page.next_after.clone();
        let response = write_page(output_directory, page_index, bytes)?;
        pages.push(FillCollectionPageEvidence {
            page_index,
            request_path: raw.request_path,
            requested_after,
            next_after,
            rows,
            minimum_fill_time_ms,
            maximum_fill_time_ms,
            response,
        });
        if pagination.accept(raw.page)? {
            break;
        }
    }
    let fills = pagination.into_fills();

    let account_after = client.account_config().await?;
    validate_account_config(account, &account_after)?;
    let account_identity_after = account_identity(config, &options.account_id, &account_after)?;
    if account_identity_after != account_identity_sha256 {
        return Err(FillCollectionError::AccountIdentity(
            "authenticated account identity changed during collection".to_string(),
        ));
    }
    let finish_clock = sample_clock(client, config.runtime.max_exchange_clock_skew_ms).await?;
    if finish_clock.server_ms < start_clock.server_ms {
        return Err(FillCollectionError::Clock(
            "exchange server time regressed during collection".to_string(),
        ));
    }
    let oldest_allowed_begin_at_finish = finish_clock
        .server_ms
        .saturating_sub(MAX_FILL_COLLECTION_WINDOW_AGE_MS);
    if options.begin_ms < oldest_allowed_begin_at_finish {
        return Err(FillCollectionError::Clock(format!(
            "begin-ms {} aged beyond the conservative recent-fill boundary {} during collection",
            options.begin_ms, oldest_allowed_begin_at_finish
        )));
    }

    let window_rows = fills
        .iter()
        .filter(|fill| (options.begin_ms..=options.end_ms).contains(&fill.ts_ms))
        .count() as u64;
    Ok(FillCollectionManifest {
        schema_version: FILL_COLLECTION_SCHEMA_VERSION,
        coverage: FillCollectionCoverage::CompleteOkxRecentFills,
        java_reference_revision: PINNED_JAVA_REVISION.to_string(),
        reap_version: env!("CARGO_PKG_VERSION").to_string(),
        executable_sha256: provenance.executable_sha256,
        host_identity_sha256: provenance.host_identity_sha256,
        config_file,
        config_fingerprint: config.fingerprint()?,
        environment: config.venue.environment,
        account_id: options.account_id.clone(),
        account_identity_sha256,
        account_level: account_before.account_level,
        position_mode: account_before.position_mode,
        endpoint: "/api/v5/trade/fills".to_string(),
        retention_ms: OKX_RECENT_FILLS_RETENTION_MS,
        maximum_window_age_ms: MAX_FILL_COLLECTION_WINDOW_AGE_MS,
        window: FillCollectionWindow {
            begin_ms: options.begin_ms,
            end_ms: options.end_ms,
            endpoints_inclusive: true,
            minimum_close_delay_ms: options.minimum_window_close_delay_ms,
        },
        max_pages: options.max_pages as u64,
        page_interval_ms: options.page_interval_ms,
        start_clock,
        finish_clock,
        pages,
        total_rows: fills.len() as u64,
        window_rows,
        total_response_bytes,
        account_identity_sampled_before_and_after: true,
        complete: true,
    })
}

async fn sample_clock<T>(
    client: &OkxRestClient<T>,
    maximum_skew_ms: u64,
) -> Result<FillCollectionClockEvidence, FillCollectionError>
where
    T: HttpTransport,
{
    let local_before = unix_time_ms()?;
    let server_ms = client.server_time_ms().await?;
    let local_after = unix_time_ms()?;
    let local_midpoint_ms = local_before + local_after.saturating_sub(local_before) / 2;
    let absolute_skew_ms = local_midpoint_ms.abs_diff(server_ms);
    if absolute_skew_ms > maximum_skew_ms {
        return Err(FillCollectionError::Clock(format!(
            "absolute local/exchange skew {absolute_skew_ms} ms exceeds configured limit {maximum_skew_ms} ms"
        )));
    }
    Ok(FillCollectionClockEvidence {
        local_midpoint_ms,
        server_ms,
        absolute_skew_ms,
    })
}

fn validate_account_config(
    expected: &crate::LiveAccountConfig,
    actual: &OkxAccountConfig,
) -> Result<(), FillCollectionError> {
    if actual.account_level != expected.expected_account_level {
        return Err(FillCollectionError::AccountIdentity(format!(
            "account level {:?} does not match configured {:?}",
            actual.account_level, expected.expected_account_level
        )));
    }
    if actual.position_mode != expected.expected_position_mode {
        return Err(FillCollectionError::AccountIdentity(format!(
            "position mode {:?} does not match configured {:?}",
            actual.position_mode, expected.expected_position_mode
        )));
    }
    if actual.user_id.trim().is_empty() || actual.main_user_id.trim().is_empty() {
        return Err(FillCollectionError::AccountIdentity(
            "exchange account identity response was empty".to_string(),
        ));
    }
    Ok(())
}

fn account_identity(
    config: &LiveConfig,
    account_id: &str,
    account: &OkxAccountConfig,
) -> Result<String, FillCollectionError> {
    if account.user_id.trim().is_empty() || account.main_user_id.trim().is_empty() {
        return Err(FillCollectionError::AccountIdentity(
            "exchange account identity response was empty".to_string(),
        ));
    }
    Ok(okx_account_identity_sha256(
        config.venue.environment,
        account_id,
        &account.user_id,
        &account.main_user_id,
    ))
}

fn reserve_output_directory(path: &Path) -> Result<PathBuf, FillCollectionError> {
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(|source| FillCollectionError::ReserveOutput {
            path: path.to_path_buf(),
            source,
        })?;
    std::fs::canonicalize(path).map_err(|source| FillCollectionError::ReserveOutput {
        path: path.to_path_buf(),
        source,
    })
}

fn read_regular_file(
    path: &Path,
    label: &'static str,
) -> Result<(FillCollectionFileEvidence, Vec<u8>), FillCollectionError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        FillCollectionError::InvalidConfigPath {
            path: path.to_path_buf(),
            message: format!("{label}: {error}"),
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(FillCollectionError::InvalidConfigPath {
            path: path.to_path_buf(),
            message: format!("{label} must be a regular file and not a symbolic link"),
        });
    }
    if metadata.len() > MAX_FILL_COLLECTION_CONFIG_BYTES {
        return Err(FillCollectionError::ConfigTooLarge {
            path: path.to_path_buf(),
            actual: metadata.len(),
            limit: MAX_FILL_COLLECTION_CONFIG_BYTES,
        });
    }
    let canonical =
        std::fs::canonicalize(path).map_err(|error| FillCollectionError::InvalidConfigPath {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    let bytes = std::fs::read(&canonical).map_err(|source| FillCollectionError::ReadConfig {
        path: canonical.clone(),
        source,
    })?;
    if bytes.len() as u64 > MAX_FILL_COLLECTION_CONFIG_BYTES {
        return Err(FillCollectionError::ConfigTooLarge {
            path: canonical,
            actual: bytes.len() as u64,
            limit: MAX_FILL_COLLECTION_CONFIG_BYTES,
        });
    }
    Ok((file_evidence(&canonical, &bytes)?, bytes))
}

fn read_evidence_file(
    path: &Path,
    label: &'static str,
    limit: u64,
) -> Result<(FillCollectionFileEvidence, Vec<u8>, PathBuf), FillCollectionError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        FillCollectionError::InvalidEvidencePath {
            label,
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(FillCollectionError::InvalidEvidencePath {
            label,
            path: path.to_path_buf(),
            message: "must be a regular file and not a symbolic link".to_string(),
        });
    }
    if metadata.len() > limit {
        return Err(FillCollectionError::EvidenceTooLarge {
            label,
            path: path.to_path_buf(),
            actual: metadata.len(),
            limit,
        });
    }
    let canonical =
        std::fs::canonicalize(path).map_err(|error| FillCollectionError::InvalidEvidencePath {
            label,
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    let bytes = std::fs::read(&canonical).map_err(|source| FillCollectionError::ReadEvidence {
        label,
        path: canonical.clone(),
        source,
    })?;
    if bytes.len() as u64 > limit {
        return Err(FillCollectionError::EvidenceTooLarge {
            label,
            path: canonical,
            actual: bytes.len() as u64,
            limit,
        });
    }
    let evidence = file_evidence(&canonical, &bytes)?;
    Ok((evidence, bytes, canonical))
}

fn write_page(
    output_directory: &Path,
    page_index: u64,
    bytes: &[u8],
) -> Result<FillCollectionFileEvidence, FillCollectionError> {
    let path = output_directory.join(format!("page-{page_index:04}.json"));
    write_create_new(&path, bytes)?;
    file_evidence(&path, bytes)
}

fn write_manifest(
    output_directory: &Path,
    manifest: &FillCollectionManifest,
) -> Result<(), FillCollectionError> {
    let mut bytes = serde_json::to_vec_pretty(manifest)?;
    bytes.push(b'\n');
    write_create_new(
        &output_directory.join(FILL_COLLECTION_MANIFEST_NAME),
        &bytes,
    )
}

fn write_create_new(path: &Path, bytes: &[u8]) -> Result<(), FillCollectionError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|source| FillCollectionError::WriteOutput {
            path: path.to_path_buf(),
            source,
        })?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|source| FillCollectionError::WriteOutput {
            path: path.to_path_buf(),
            source,
        })
}

fn file_evidence(
    path: &Path,
    bytes: &[u8],
) -> Result<FillCollectionFileEvidence, FillCollectionError> {
    let canonical =
        std::fs::canonicalize(path).map_err(|source| FillCollectionError::WriteOutput {
            path: path.to_path_buf(),
            source,
        })?;
    let path = canonical
        .to_str()
        .ok_or_else(|| FillCollectionError::InvalidConfigPath {
            path: canonical.clone(),
            message: "canonical path is not valid UTF-8".to_string(),
        })?
        .to_string();
    Ok(FillCollectionFileEvidence {
        path,
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
    })
}

fn sync_directory(path: &Path) -> Result<(), FillCollectionError> {
    let directory =
        std::fs::File::open(path).map_err(|source| FillCollectionError::WriteOutput {
            path: path.to_path_buf(),
            source,
        })?;
    directory
        .sync_all()
        .map_err(|source| FillCollectionError::WriteOutput {
            path: path.to_path_buf(),
            source,
        })
}

fn unix_time_ms() -> Result<u64, FillCollectionError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| FillCollectionError::Clock(error.to_string()))?
        .as_millis();
    u64::try_from(millis).map_err(|error| FillCollectionError::Clock(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use reap_venue::okx::{HttpResponse, OkxCredentials, SignedRequest};

    use super::*;

    #[derive(Clone)]
    struct MockTransport {
        responses: Arc<Mutex<VecDeque<Result<HttpResponse, RestError>>>>,
        requests: Arc<Mutex<Vec<SignedRequest>>>,
    }

    #[async_trait]
    impl HttpTransport for MockTransport {
        async fn execute(&self, request: SignedRequest) -> Result<HttpResponse, RestError> {
            self.requests.lock().unwrap().push(request);
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("missing mock response")
        }
    }

    fn response(body: String) -> Result<HttpResponse, RestError> {
        Ok(HttpResponse { status: 200, body })
    }

    fn time_response(timestamp_ms: u64) -> Result<HttpResponse, RestError> {
        response(format!(
            r#"{{"code":"0","msg":"","data":[{{"ts":"{timestamp_ms}"}}]}}"#
        ))
    }

    fn account_response(user_id: &str) -> Result<HttpResponse, RestError> {
        response(format!(
            r#"{{"code":"0","msg":"","data":[{{"acctLv":"2","posMode":"net_mode","acctStpMode":"cancel_maker","uid":"{user_id}","mainUid":"6"}}]}}"#
        ))
    }

    fn fill_response(fill_time_ms: u64) -> String {
        format!(
            r#"{{"code":"0","msg":"","data":[{{"billId":"bill-1","tradeId":"trade-1","ordId":"exchange-1","clOrdId":"client-1","instId":"BTC-USDT","side":"buy","fillPx":"50000","fillSz":"0.01","execType":"M","fee":"-0.00001","feeCcy":"BTC","fillTime":"{fill_time_ms}"}}]}}"#
        )
    }

    fn config() -> LiveConfig {
        LiveConfig::from_toml(include_str!("../../../examples/live-okx-demo.toml")).unwrap()
    }

    fn options(now_ms: u64) -> FillCollectionOptions {
        FillCollectionOptions {
            account_id: "main".to_string(),
            begin_ms: now_ms - 120_000,
            end_ms: now_ms - 60_000,
            max_pages: 3,
            page_interval_ms: MIN_FILL_COLLECTION_PAGE_INTERVAL_MS,
            minimum_window_close_delay_ms: 30_000,
        }
    }

    fn output_directory() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "reap-fill-collection-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        path
    }

    fn evidence(path: &Path) -> FillCollectionFileEvidence {
        FillCollectionFileEvidence {
            path: path.to_str().unwrap().to_string(),
            bytes: 1,
            sha256: "a".repeat(64),
        }
    }

    fn provenance() -> CollectionProvenance {
        CollectionProvenance {
            executable_sha256: "b".repeat(64),
            host_identity_sha256: "c".repeat(64),
        }
    }

    async fn complete_fixture(directory: &Path) -> FillCollectionManifest {
        let now_ms = unix_time_ms().unwrap();
        let config_path = directory.join("live.toml");
        write_create_new(
            &config_path,
            include_str!("../../../examples/live-okx-demo.toml").as_bytes(),
        )
        .unwrap();
        let (config_file, _) = read_regular_file(&config_path, "live config").unwrap();
        let client = OkxRestClient::new(
            MockTransport {
                responses: Arc::new(Mutex::new(VecDeque::from([
                    time_response(now_ms),
                    account_response("7"),
                    response(fill_response(now_ms - 90_000)),
                    account_response("7"),
                    time_response(now_ms + 1),
                ]))),
                requests: Arc::new(Mutex::new(Vec::new())),
            },
            OkxSigner::new(OkxCredentials::new("key", "secret", "pass"), true),
        );
        collect_recent_okx_fills_with_client(
            &client,
            &config(),
            config_file,
            directory,
            &options(now_ms),
            provenance(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn complete_recent_collection_binds_raw_page_account_and_window() {
        let now_ms = unix_time_ms().unwrap();
        let raw_fill = fill_response(now_ms - 90_000);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let client = OkxRestClient::new(
            MockTransport {
                responses: Arc::new(Mutex::new(VecDeque::from([
                    time_response(now_ms),
                    account_response("7"),
                    response(raw_fill.clone()),
                    account_response("7"),
                    time_response(now_ms + 1),
                ]))),
                requests: Arc::clone(&requests),
            },
            OkxSigner::new(OkxCredentials::new("key", "secret", "pass"), true),
        );
        let directory = output_directory();

        let manifest = collect_recent_okx_fills_with_client(
            &client,
            &config(),
            evidence(Path::new("/config")),
            &directory,
            &options(now_ms),
            provenance(),
        )
        .await
        .unwrap();

        assert!(manifest.complete);
        assert_eq!(manifest.total_rows, 1);
        assert_eq!(manifest.window_rows, 1);
        assert_eq!(manifest.pages.len(), 1);
        assert_eq!(manifest.pages[0].rows, 1);
        assert_eq!(manifest.pages[0].response.bytes, raw_fill.len() as u64);
        assert_eq!(manifest.pages[0].response.sha256.len(), 64);
        assert_eq!(manifest.account_identity_sha256.len(), 64);
        assert!(manifest.account_identity_sampled_before_and_after);
        assert_eq!(
            std::fs::read_to_string(&manifest.pages[0].response.path).unwrap(),
            raw_fill
        );
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 5);
        assert_eq!(requests[2].path, "/api/v5/trade/fills?limit=100");

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn account_identity_change_fails_after_preserving_raw_page() {
        let now_ms = unix_time_ms().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let client = OkxRestClient::new(
            MockTransport {
                responses: Arc::new(Mutex::new(VecDeque::from([
                    time_response(now_ms),
                    account_response("7"),
                    response(fill_response(now_ms - 90_000)),
                    account_response("8"),
                ]))),
                requests,
            },
            OkxSigner::new(OkxCredentials::new("key", "secret", "pass"), true),
        );
        let directory = output_directory();

        let error = collect_recent_okx_fills_with_client(
            &client,
            &config(),
            evidence(Path::new("/config")),
            &directory,
            &options(now_ms),
            provenance(),
        )
        .await
        .unwrap_err();

        assert!(matches!(error, FillCollectionError::AccountIdentity(_)));
        assert!(directory.join("page-0001.json").is_file());
        assert!(!directory.join(FILL_COLLECTION_MANIFEST_NAME).exists());

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn collection_options_reject_unbounded_or_unclosed_requests() {
        let now_ms = unix_time_ms().unwrap();
        let mut invalid = options(now_ms);
        invalid.max_pages = 0;
        assert!(invalid.validate().is_err());

        let mut invalid = options(now_ms);
        invalid.page_interval_ms = MIN_FILL_COLLECTION_PAGE_INTERVAL_MS - 1;
        assert!(invalid.validate().is_err());
    }

    #[tokio::test]
    async fn verifier_replays_complete_collection_from_exact_files() {
        let directory = output_directory();
        let manifest = complete_fixture(&directory).await;
        write_manifest(&directory, &manifest).unwrap();

        let verified =
            verify_fill_collection_manifest_path(directory.join(FILL_COLLECTION_MANIFEST_NAME))
                .unwrap();

        assert_eq!(verified.manifest, manifest);
        assert_eq!(verified.page_paths.len(), 1);
        assert_eq!(verified.manifest_file.sha256.len(), 64);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn verifier_rejects_response_changed_after_manifest() {
        let directory = output_directory();
        let manifest = complete_fixture(&directory).await;
        write_manifest(&directory, &manifest).unwrap();
        let mut page = OpenOptions::new()
            .append(true)
            .open(&manifest.pages[0].response.path)
            .unwrap();
        page.write_all(b"\n").unwrap();
        page.sync_all().unwrap();

        let error =
            verify_fill_collection_manifest_path(directory.join(FILL_COLLECTION_MANIFEST_NAME))
                .unwrap_err();

        assert!(matches!(error, FillCollectionError::InvalidEvidence(_)));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn verifier_rejects_manifest_request_path_not_derived_from_cursor() {
        let directory = output_directory();
        let mut manifest = complete_fixture(&directory).await;
        manifest.pages[0].request_path = "/api/v5/trade/fills?after=unproven&limit=100".to_string();
        write_manifest(&directory, &manifest).unwrap();

        let error =
            verify_fill_collection_manifest_path(directory.join(FILL_COLLECTION_MANIFEST_NAME))
                .unwrap_err();

        assert!(matches!(error, FillCollectionError::InvalidEvidence(_)));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn verifier_rejects_window_that_aged_out_during_collection() {
        let directory = output_directory();
        let mut manifest = complete_fixture(&directory).await;
        let late_finish = manifest.start_clock.server_ms + MAX_FILL_COLLECTION_WINDOW_AGE_MS + 1;
        manifest.finish_clock = FillCollectionClockEvidence {
            local_midpoint_ms: late_finish,
            server_ms: late_finish,
            absolute_skew_ms: 0,
        };
        write_manifest(&directory, &manifest).unwrap();

        let error =
            verify_fill_collection_manifest_path(directory.join(FILL_COLLECTION_MANIFEST_NAME))
                .unwrap_err();

        assert!(matches!(error, FillCollectionError::InvalidEvidence(_)));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
