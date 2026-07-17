use std::collections::BTreeSet;
use std::fs::{DirBuilder, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reap_core::PINNED_JAVA_REVISION;
use reap_evidence_core::{
    EvidenceClientFactory, EvidenceClientFactoryError, EvidenceCredentialEnvironment,
    EvidenceHttpConfig, EvidenceReadError, EvidenceReadOnly,
};
use reap_venue::okx::{
    OkxAccountConfig, OkxAccountLevel, OkxBill, OkxBillPagination, OkxPositionMode, RestError,
    parse_okx_account_config_response_json, parse_okx_bill_page_response_json,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::fill_collection::{FillCollectionClockEvidence, FillCollectionFileEvidence};
use crate::provenance::{
    current_executable_sha256, host_identity_sha256, okx_account_identity_sha256,
};
use crate::{LiveAccountConfig, LiveConfig, LiveConfigError, TradingEnvironment};

pub const BILL_COLLECTION_SCHEMA_VERSION: u32 = 1;
pub const BILL_COLLECTION_MANIFEST_NAME: &str = "manifest.json";
pub const MAX_BILL_COLLECTION_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_BILL_COLLECTION_CONFIG_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_BILL_COLLECTION_PAGES: usize = 1_000;
pub const MAX_BILL_COLLECTION_PAGE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_BILL_COLLECTION_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
pub const OKX_ACCOUNT_BILLS_RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
/// Keep two hours of margin inside OKX's documented seven-day recent-bill window.
pub const MAX_BILL_COLLECTION_WINDOW_AGE_MS: u64 = 166 * 60 * 60 * 1_000;
/// The account-bills endpoint permits five requests per two seconds. A 500 ms
/// floor leaves scheduling margin instead of operating exactly at the limit.
pub const MIN_BILL_COLLECTION_PAGE_INTERVAL_MS: u64 = 500;
pub const MAX_BILL_COLLECTION_PAGE_INTERVAL_MS: u64 = 60_000;
pub const MAX_BILL_COLLECTION_CLOSE_DELAY_MS: u64 = 10 * 60 * 1_000;

#[derive(Debug, Clone)]
pub struct BillCollectionOptions {
    pub account_id: String,
    pub begin_ms: u64,
    pub end_ms: u64,
    pub max_pages: usize,
    pub page_interval_ms: u64,
    pub minimum_window_close_delay_ms: u64,
}

impl BillCollectionOptions {
    fn validate(&self) -> Result<(), BillCollectionError> {
        if self.account_id.is_empty() || self.account_id.trim() != self.account_id {
            return Err(BillCollectionError::InvalidOptions(
                "account id must be non-empty and contain no surrounding whitespace".to_string(),
            ));
        }
        if self.account_id.len() > 128 {
            return Err(BillCollectionError::InvalidOptions(
                "account id exceeds 128 bytes".to_string(),
            ));
        }
        if self.begin_ms == 0 || self.end_ms == 0 || self.begin_ms > self.end_ms {
            return Err(BillCollectionError::InvalidOptions(
                "begin-ms and end-ms must form a positive inclusive window".to_string(),
            ));
        }
        if self.max_pages == 0 || self.max_pages > MAX_BILL_COLLECTION_PAGES {
            return Err(BillCollectionError::InvalidOptions(format!(
                "max-pages must be in 1..={MAX_BILL_COLLECTION_PAGES}"
            )));
        }
        if !(MIN_BILL_COLLECTION_PAGE_INTERVAL_MS..=MAX_BILL_COLLECTION_PAGE_INTERVAL_MS)
            .contains(&self.page_interval_ms)
        {
            return Err(BillCollectionError::InvalidOptions(format!(
                "page-interval-ms must be in {MIN_BILL_COLLECTION_PAGE_INTERVAL_MS}..={MAX_BILL_COLLECTION_PAGE_INTERVAL_MS}"
            )));
        }
        if self.minimum_window_close_delay_ms == 0
            || self.minimum_window_close_delay_ms > MAX_BILL_COLLECTION_CLOSE_DELAY_MS
        {
            return Err(BillCollectionError::InvalidOptions(format!(
                "minimum-window-close-delay-ms must be in 1..={MAX_BILL_COLLECTION_CLOSE_DELAY_MS}"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillCollectionCoverage {
    CompleteOkxAccountBills,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BillCollectionWindow {
    pub begin_ms: u64,
    pub end_ms: u64,
    pub endpoints_inclusive: bool,
    pub minimum_close_delay_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BillCollectionPageEvidence {
    pub page_index: u64,
    pub request_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_after: Option<String>,
    pub rows: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_bill_time_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_bill_time_ms: Option<u64>,
    pub response: FillCollectionFileEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BillCollectionManifest {
    pub schema_version: u32,
    pub coverage: BillCollectionCoverage,
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
    pub window: BillCollectionWindow,
    pub max_pages: u64,
    pub page_interval_ms: u64,
    pub start_clock: FillCollectionClockEvidence,
    pub finish_clock: FillCollectionClockEvidence,
    pub pages: Vec<BillCollectionPageEvidence>,
    pub total_rows: u64,
    pub total_response_bytes: u64,
    pub account_identity_sampled_before_and_after: bool,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VerifiedBillCollection {
    pub manifest_file: FillCollectionFileEvidence,
    pub manifest: BillCollectionManifest,
    pub page_paths: Vec<PathBuf>,
    pub bills: Vec<OkxBill>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BillCollectionVerificationSummary {
    pub schema_version: u32,
    pub java_reference_revision: String,
    pub manifest_file: FillCollectionFileEvidence,
    pub config_file: FillCollectionFileEvidence,
    pub account_id: String,
    pub environment: TradingEnvironment,
    pub window: BillCollectionWindow,
    pub page_count: u64,
    pub total_rows: u64,
    pub total_response_bytes: u64,
    pub verification_passed: bool,
}

impl VerifiedBillCollection {
    pub fn summary(&self) -> BillCollectionVerificationSummary {
        BillCollectionVerificationSummary {
            schema_version: BILL_COLLECTION_SCHEMA_VERSION,
            java_reference_revision: PINNED_JAVA_REVISION.to_string(),
            manifest_file: self.manifest_file.clone(),
            config_file: self.manifest.config_file.clone(),
            account_id: self.manifest.account_id.clone(),
            environment: self.manifest.environment,
            window: self.manifest.window.clone(),
            page_count: self.page_paths.len() as u64,
            total_rows: self.bills.len() as u64,
            total_response_bytes: self.manifest.total_response_bytes,
            verification_passed: true,
        }
    }
}

#[derive(Debug, Error)]
pub enum BillCollectionError {
    #[error("invalid bill-collection options: {0}")]
    InvalidOptions(String),
    #[error("failed to reserve bill-collection directory {path}: {source}")]
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
    #[error("OKX bill collection failed: {0}")]
    Rest(#[from] RestError),
    #[error("OKX bill collection failed: {0}")]
    Evidence(#[from] EvidenceReadError),
    #[error("exchange clock evidence is invalid: {0}")]
    Clock(String),
    #[error("exchange account identity evidence is invalid: {0}")]
    AccountIdentity(String),
    #[error("bill response page {page} is {actual} bytes; limit is {limit}")]
    PageTooLarge { page: u64, actual: u64, limit: u64 },
    #[error("bill response pages total {actual} bytes; aggregate limit is {limit}")]
    PagesTooLarge { actual: u64, limit: u64 },
    #[error(
        "bill {bill_id} on page {page} has timestamp {timestamp_ms} outside inclusive window {begin_ms}..={end_ms}"
    )]
    BillOutsideWindow {
        page: u64,
        bill_id: String,
        timestamp_ms: u64,
        begin_ms: u64,
        end_ms: u64,
    },
    #[error("failed to write bill-collection file {path}: {source}")]
    WriteOutput {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize bill-collection manifest: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("invalid bill-collection evidence: {0}")]
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
    #[error("failed to parse bill-collection manifest {path}: {source}")]
    ParseManifest {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to parse collected bill page {path}: {source}")]
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

fn evidence_credential_environment(account: &LiveAccountConfig) -> EvidenceCredentialEnvironment {
    EvidenceCredentialEnvironment::new(
        &account.id,
        &account.api_key_env,
        &account.secret_key_env,
        &account.passphrase_env,
    )
}

fn map_factory_error(error: EvidenceClientFactoryError) -> BillCollectionError {
    match error {
        EvidenceClientFactoryError::MissingCredential { account_id, name } => {
            BillCollectionError::Config(LiveConfigError::MissingCredential { account_id, name })
        }
        EvidenceClientFactoryError::InvalidConfiguration(message) => {
            BillCollectionError::Transport(RestError::Transport(format!(
                "invalid evidence client configuration: {message}"
            )))
        }
        EvidenceClientFactoryError::Transport(message) => {
            BillCollectionError::Transport(RestError::Transport(message))
        }
    }
}

/// Collects complete account-wide OKX bills for a closed window without order entry.
///
/// The destination is reserved before config parsing, credentials, or network
/// access. A failure leaves raw diagnostic pages but no complete manifest.
pub async fn collect_okx_bills_paths<F>(
    config_path: impl AsRef<Path>,
    output_directory: impl AsRef<Path>,
    options: BillCollectionOptions,
    factory: &F,
) -> Result<BillCollectionManifest, BillCollectionError>
where
    F: EvidenceClientFactory,
    F::Client: EvidenceReadOnly<Error = EvidenceReadError>,
{
    options.validate()?;
    let output_directory = reserve_output_directory(output_directory.as_ref())?;
    let (config_file, config_bytes) = read_regular_file(config_path.as_ref(), "live config")?;
    let config_text = std::str::from_utf8(&config_bytes).map_err(|error| {
        BillCollectionError::InvalidConfigPath {
            path: PathBuf::from(&config_file.path),
            message: format!("config is not valid UTF-8: {error}"),
        }
    })?;
    let config = LiveConfig::from_toml(config_text)?;
    let account = config
        .account(&options.account_id)
        .ok_or_else(|| BillCollectionError::UnknownAccount(options.account_id.clone()))?;
    let prepared = factory
        .prepare_credentials(&evidence_credential_environment(account))
        .map_err(map_factory_error)?;
    let provenance = CollectionProvenance {
        executable_sha256: current_executable_sha256().map_err(BillCollectionError::Provenance)?,
        host_identity_sha256: host_identity_sha256().map_err(BillCollectionError::Provenance)?,
    };
    let client = factory
        .connect(
            prepared,
            &EvidenceHttpConfig::new(
                &config.venue.rest_url,
                config.venue.environment.is_demo(),
                Duration::from_millis(config.runtime.rest_connect_timeout_ms),
                Duration::from_millis(config.runtime.rest_request_timeout_ms),
            ),
        )
        .map_err(map_factory_error)?;
    let manifest = collect_okx_bills_with_client(
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

/// Independently verifies a bill collection without credentials or network access.
///
/// Every source is reopened, bounded, hashed, and parsed. The verifier rebuilds
/// the exact request path and cursor chain from raw pages and rejects a full
/// final page, duplicate bill, omitted page, or row outside the closed window.
pub fn verify_bill_collection_manifest_path(
    manifest_path: impl AsRef<Path>,
) -> Result<VerifiedBillCollection, BillCollectionError> {
    let manifest_path = manifest_path.as_ref();
    let (manifest_file, manifest_bytes, canonical_manifest_path) = read_evidence_file(
        manifest_path,
        "bill-collection manifest",
        MAX_BILL_COLLECTION_MANIFEST_BYTES,
    )?;
    let manifest: BillCollectionManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|source| {
            BillCollectionError::ParseManifest {
                path: canonical_manifest_path.clone(),
                source,
            }
        })?;
    validate_manifest_header(&manifest)?;

    let config_path = PathBuf::from(&manifest.config_file.path);
    let (config_file, config_bytes, canonical_config_path) = read_evidence_file(
        &config_path,
        "referenced live config",
        MAX_BILL_COLLECTION_CONFIG_BYTES,
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
        BillCollectionError::InvalidEvidence(format!(
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
        .ok_or_else(|| BillCollectionError::UnknownAccount(manifest.account_id.clone()))?;
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
        return invalid_evidence("collection window exceeds the conservative account-bill age");
    }

    let max_pages = usize::try_from(manifest.max_pages)
        .map_err(|_| BillCollectionError::InvalidEvidence("max_pages exceeds usize".to_string()))?;
    if manifest.pages.len() > max_pages {
        return invalid_evidence(format!(
            "manifest contains {} pages but max_pages is {}",
            manifest.pages.len(),
            max_pages
        ));
    }
    let mut pagination = OkxBillPagination::new(max_pages)?;
    let mut page_paths = Vec::with_capacity(manifest.pages.len());
    let mut seen_paths = BTreeSet::new();
    seen_paths.insert(canonical_manifest_path);
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
        let expected_request_path = bills_request_path(
            manifest.window.begin_ms,
            manifest.window.end_ms,
            expected_after,
        );
        if evidence.request_path != expected_request_path {
            return invalid_evidence(format!(
                "page {expected_index} request path is {:?}; expected {:?}",
                evidence.request_path, expected_request_path
            ));
        }

        let response_path = PathBuf::from(&evidence.response.path);
        let (observed, bytes, canonical_response_path) = read_evidence_file(
            &response_path,
            "collected bill page",
            MAX_BILL_COLLECTION_PAGE_BYTES,
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
                BillCollectionError::InvalidEvidence(
                    "aggregate response byte count overflowed".to_string(),
                )
            })?;
        if total_response_bytes > MAX_BILL_COLLECTION_TOTAL_BYTES {
            return Err(BillCollectionError::PagesTooLarge {
                actual: total_response_bytes,
                limit: MAX_BILL_COLLECTION_TOTAL_BYTES,
            });
        }
        let page = parse_okx_bill_page_response_json(&bytes).map_err(|source| {
            BillCollectionError::ParsePage {
                path: canonical_response_path.clone(),
                source,
            }
        })?;
        let rows = page.bills.len() as u64;
        let minimum_bill_time_ms = page.bills.iter().map(|bill| bill.timestamp_ms).min();
        let maximum_bill_time_ms = page.bills.iter().map(|bill| bill.timestamp_ms).max();
        if rows != evidence.rows
            || page.next_after != evidence.next_after
            || minimum_bill_time_ms != evidence.minimum_bill_time_ms
            || maximum_bill_time_ms != evidence.maximum_bill_time_ms
        {
            return invalid_evidence(format!(
                "page {expected_index} parsed row, cursor, or timestamp evidence does not match"
            ));
        }
        for bill in &page.bills {
            validate_bill_window(
                expected_index,
                bill,
                manifest.window.begin_ms,
                manifest.window.end_ms,
            )?;
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

    let bills = pagination.into_bills();
    if bills.len() as u64 != manifest.total_rows
        || total_response_bytes != manifest.total_response_bytes
    {
        return invalid_evidence("manifest aggregate row or response-byte evidence does not match");
    }

    Ok(VerifiedBillCollection {
        manifest_file,
        manifest,
        page_paths,
        bills,
    })
}

fn validate_manifest_header(manifest: &BillCollectionManifest) -> Result<(), BillCollectionError> {
    if manifest.schema_version != BILL_COLLECTION_SCHEMA_VERSION {
        return invalid_evidence(format!(
            "schema version {} is unsupported; expected {BILL_COLLECTION_SCHEMA_VERSION}",
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
    if manifest.endpoint != "/api/v5/account/bills" {
        return invalid_evidence(
            "manifest endpoint is not the authenticated account-bills endpoint",
        );
    }
    if manifest.retention_ms != OKX_ACCOUNT_BILLS_RETENTION_MS
        || manifest.maximum_window_age_ms != MAX_BILL_COLLECTION_WINDOW_AGE_MS
    {
        return invalid_evidence("manifest retention bounds do not match this verifier");
    }
    if manifest.window.begin_ms == 0
        || manifest.window.end_ms == 0
        || manifest.window.begin_ms > manifest.window.end_ms
        || !manifest.window.endpoints_inclusive
    {
        return invalid_evidence("manifest window is invalid or is not inclusive");
    }
    if manifest.window.minimum_close_delay_ms == 0
        || manifest.window.minimum_close_delay_ms > MAX_BILL_COLLECTION_CLOSE_DELAY_MS
    {
        return invalid_evidence("manifest window close delay is outside supported bounds");
    }
    if manifest.max_pages == 0 || manifest.max_pages > MAX_BILL_COLLECTION_PAGES as u64 {
        return invalid_evidence("manifest max_pages is outside supported bounds");
    }
    if !(MIN_BILL_COLLECTION_PAGE_INTERVAL_MS..=MAX_BILL_COLLECTION_PAGE_INTERVAL_MS)
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
) -> Result<(), BillCollectionError> {
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

fn bills_request_path(begin_ms: u64, end_ms: u64, after: Option<&str>) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.append_pair("begin", &begin_ms.to_string());
    serializer.append_pair("end", &end_ms.to_string());
    if let Some(after) = after {
        serializer.append_pair("after", after);
    }
    serializer.append_pair("limit", "100");
    format!("/api/v5/account/bills?{}", serializer.finish())
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn invalid_evidence<T>(message: impl Into<String>) -> Result<T, BillCollectionError> {
    Err(BillCollectionError::InvalidEvidence(message.into()))
}

async fn collect_okx_bills_with_client<C>(
    client: &C,
    config: &LiveConfig,
    config_file: FillCollectionFileEvidence,
    output_directory: &Path,
    options: &BillCollectionOptions,
    provenance: CollectionProvenance,
) -> Result<BillCollectionManifest, BillCollectionError>
where
    C: EvidenceReadOnly<Error = EvidenceReadError> + ?Sized,
{
    let account = config
        .account(&options.account_id)
        .ok_or_else(|| BillCollectionError::UnknownAccount(options.account_id.clone()))?;
    let start_clock = sample_clock(client, config.runtime.max_exchange_clock_skew_ms).await?;
    let latest_allowed_end = start_clock
        .server_ms
        .saturating_sub(options.minimum_window_close_delay_ms);
    if options.end_ms > latest_allowed_end {
        return Err(BillCollectionError::Clock(format!(
            "end-ms {} must be at or before {} after the required {} ms close delay",
            options.end_ms, latest_allowed_end, options.minimum_window_close_delay_ms
        )));
    }
    let oldest_allowed_begin = start_clock
        .server_ms
        .saturating_sub(MAX_BILL_COLLECTION_WINDOW_AGE_MS);
    if options.begin_ms < oldest_allowed_begin {
        return Err(BillCollectionError::Clock(format!(
            "begin-ms {} is older than conservative account-bill boundary {}",
            options.begin_ms, oldest_allowed_begin
        )));
    }

    let account_before_response = client.account_config().await?;
    let account_before =
        parse_okx_account_config_response_json(account_before_response.response_body().as_bytes())?;
    validate_account_config(account, &account_before)?;
    let account_identity_sha256 = account_identity(config, &options.account_id, &account_before)?;

    let mut pagination = OkxBillPagination::new(options.max_pages)?;
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
            .account_bills_page(options.begin_ms, options.end_ms, pagination.after())
            .await?;
        let page = parse_okx_bill_page_response_json(raw.response_body().as_bytes())?;
        let (request_path, response_body) = raw.into_parts();
        let bytes = response_body.as_bytes();
        let bytes_len = bytes.len() as u64;
        if bytes_len > MAX_BILL_COLLECTION_PAGE_BYTES {
            return Err(BillCollectionError::PageTooLarge {
                page: page_index,
                actual: bytes_len,
                limit: MAX_BILL_COLLECTION_PAGE_BYTES,
            });
        }
        total_response_bytes = total_response_bytes.checked_add(bytes_len).ok_or({
            BillCollectionError::PagesTooLarge {
                actual: u64::MAX,
                limit: MAX_BILL_COLLECTION_TOTAL_BYTES,
            }
        })?;
        if total_response_bytes > MAX_BILL_COLLECTION_TOTAL_BYTES {
            return Err(BillCollectionError::PagesTooLarge {
                actual: total_response_bytes,
                limit: MAX_BILL_COLLECTION_TOTAL_BYTES,
            });
        }
        let rows = page.bills.len() as u64;
        let minimum_bill_time_ms = page.bills.iter().map(|bill| bill.timestamp_ms).min();
        let maximum_bill_time_ms = page.bills.iter().map(|bill| bill.timestamp_ms).max();
        let next_after = page.next_after.clone();
        let response = write_page(output_directory, page_index, bytes)?;
        pages.push(BillCollectionPageEvidence {
            page_index,
            request_path,
            requested_after,
            next_after,
            rows,
            minimum_bill_time_ms,
            maximum_bill_time_ms,
            response,
        });
        for bill in &page.bills {
            validate_bill_window(page_index, bill, options.begin_ms, options.end_ms)?;
        }
        if pagination.accept(page)? {
            break;
        }
    }
    let bills = pagination.into_bills();

    let account_after_response = client.account_config().await?;
    let account_after =
        parse_okx_account_config_response_json(account_after_response.response_body().as_bytes())?;
    validate_account_config(account, &account_after)?;
    let account_identity_after = account_identity(config, &options.account_id, &account_after)?;
    if account_identity_after != account_identity_sha256 {
        return Err(BillCollectionError::AccountIdentity(
            "authenticated account identity changed during collection".to_string(),
        ));
    }
    let finish_clock = sample_clock(client, config.runtime.max_exchange_clock_skew_ms).await?;
    if finish_clock.server_ms < start_clock.server_ms {
        return Err(BillCollectionError::Clock(
            "exchange server time regressed during collection".to_string(),
        ));
    }
    let oldest_allowed_begin_at_finish = finish_clock
        .server_ms
        .saturating_sub(MAX_BILL_COLLECTION_WINDOW_AGE_MS);
    if options.begin_ms < oldest_allowed_begin_at_finish {
        return Err(BillCollectionError::Clock(format!(
            "begin-ms {} aged beyond the conservative account-bill boundary {} during collection",
            options.begin_ms, oldest_allowed_begin_at_finish
        )));
    }

    Ok(BillCollectionManifest {
        schema_version: BILL_COLLECTION_SCHEMA_VERSION,
        coverage: BillCollectionCoverage::CompleteOkxAccountBills,
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
        endpoint: "/api/v5/account/bills".to_string(),
        retention_ms: OKX_ACCOUNT_BILLS_RETENTION_MS,
        maximum_window_age_ms: MAX_BILL_COLLECTION_WINDOW_AGE_MS,
        window: BillCollectionWindow {
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
        total_rows: bills.len() as u64,
        total_response_bytes,
        account_identity_sampled_before_and_after: true,
        complete: true,
    })
}

fn validate_bill_window(
    page: u64,
    bill: &OkxBill,
    begin_ms: u64,
    end_ms: u64,
) -> Result<(), BillCollectionError> {
    if !(begin_ms..=end_ms).contains(&bill.timestamp_ms) {
        return Err(BillCollectionError::BillOutsideWindow {
            page,
            bill_id: bill.bill_id.clone(),
            timestamp_ms: bill.timestamp_ms,
            begin_ms,
            end_ms,
        });
    }
    Ok(())
}

async fn sample_clock<C>(
    client: &C,
    maximum_skew_ms: u64,
) -> Result<FillCollectionClockEvidence, BillCollectionError>
where
    C: EvidenceReadOnly<Error = EvidenceReadError> + ?Sized,
{
    let local_before = unix_time_ms()?;
    let server_ms = client.server_time_ms().await?;
    let local_after = unix_time_ms()?;
    let local_midpoint_ms = local_before + local_after.saturating_sub(local_before) / 2;
    let absolute_skew_ms = local_midpoint_ms.abs_diff(server_ms);
    if absolute_skew_ms > maximum_skew_ms {
        return Err(BillCollectionError::Clock(format!(
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
    expected: &LiveAccountConfig,
    actual: &OkxAccountConfig,
) -> Result<(), BillCollectionError> {
    if actual.account_level != expected.expected_account_level {
        return Err(BillCollectionError::AccountIdentity(format!(
            "account level {:?} does not match configured {:?}",
            actual.account_level, expected.expected_account_level
        )));
    }
    if actual.position_mode != expected.expected_position_mode {
        return Err(BillCollectionError::AccountIdentity(format!(
            "position mode {:?} does not match configured {:?}",
            actual.position_mode, expected.expected_position_mode
        )));
    }
    if actual.user_id.trim().is_empty() || actual.main_user_id.trim().is_empty() {
        return Err(BillCollectionError::AccountIdentity(
            "exchange account identity response was empty".to_string(),
        ));
    }
    Ok(())
}

fn account_identity(
    config: &LiveConfig,
    account_id: &str,
    account: &OkxAccountConfig,
) -> Result<String, BillCollectionError> {
    if account.user_id.trim().is_empty() || account.main_user_id.trim().is_empty() {
        return Err(BillCollectionError::AccountIdentity(
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

fn reserve_output_directory(path: &Path) -> Result<PathBuf, BillCollectionError> {
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(|source| BillCollectionError::ReserveOutput {
            path: path.to_path_buf(),
            source,
        })?;
    std::fs::canonicalize(path).map_err(|source| BillCollectionError::ReserveOutput {
        path: path.to_path_buf(),
        source,
    })
}

fn read_regular_file(
    path: &Path,
    label: &'static str,
) -> Result<(FillCollectionFileEvidence, Vec<u8>), BillCollectionError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        BillCollectionError::InvalidConfigPath {
            path: path.to_path_buf(),
            message: format!("{label}: {error}"),
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(BillCollectionError::InvalidConfigPath {
            path: path.to_path_buf(),
            message: format!("{label} must be a regular file and not a symbolic link"),
        });
    }
    if metadata.len() > MAX_BILL_COLLECTION_CONFIG_BYTES {
        return Err(BillCollectionError::ConfigTooLarge {
            path: path.to_path_buf(),
            actual: metadata.len(),
            limit: MAX_BILL_COLLECTION_CONFIG_BYTES,
        });
    }
    let canonical =
        std::fs::canonicalize(path).map_err(|error| BillCollectionError::InvalidConfigPath {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    let bytes = std::fs::read(&canonical).map_err(|source| BillCollectionError::ReadConfig {
        path: canonical.clone(),
        source,
    })?;
    if bytes.len() as u64 > MAX_BILL_COLLECTION_CONFIG_BYTES {
        return Err(BillCollectionError::ConfigTooLarge {
            path: canonical,
            actual: bytes.len() as u64,
            limit: MAX_BILL_COLLECTION_CONFIG_BYTES,
        });
    }
    Ok((file_evidence(&canonical, &bytes)?, bytes))
}

fn read_evidence_file(
    path: &Path,
    label: &'static str,
    limit: u64,
) -> Result<(FillCollectionFileEvidence, Vec<u8>, PathBuf), BillCollectionError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        BillCollectionError::InvalidEvidencePath {
            label,
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(BillCollectionError::InvalidEvidencePath {
            label,
            path: path.to_path_buf(),
            message: "must be a regular file and not a symbolic link".to_string(),
        });
    }
    if metadata.len() > limit {
        return Err(BillCollectionError::EvidenceTooLarge {
            label,
            path: path.to_path_buf(),
            actual: metadata.len(),
            limit,
        });
    }
    let canonical =
        std::fs::canonicalize(path).map_err(|error| BillCollectionError::InvalidEvidencePath {
            label,
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    let bytes = std::fs::read(&canonical).map_err(|source| BillCollectionError::ReadEvidence {
        label,
        path: canonical.clone(),
        source,
    })?;
    if bytes.len() as u64 > limit {
        return Err(BillCollectionError::EvidenceTooLarge {
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
) -> Result<FillCollectionFileEvidence, BillCollectionError> {
    let path = output_directory.join(format!("page-{page_index:04}.json"));
    write_create_new(&path, bytes)?;
    file_evidence(&path, bytes)
}

fn write_manifest(
    output_directory: &Path,
    manifest: &BillCollectionManifest,
) -> Result<(), BillCollectionError> {
    let mut bytes = serde_json::to_vec_pretty(manifest)?;
    bytes.push(b'\n');
    write_create_new(
        &output_directory.join(BILL_COLLECTION_MANIFEST_NAME),
        &bytes,
    )
}

fn write_create_new(path: &Path, bytes: &[u8]) -> Result<(), BillCollectionError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|source| BillCollectionError::WriteOutput {
            path: path.to_path_buf(),
            source,
        })?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|source| BillCollectionError::WriteOutput {
            path: path.to_path_buf(),
            source,
        })
}

fn file_evidence(
    path: &Path,
    bytes: &[u8],
) -> Result<FillCollectionFileEvidence, BillCollectionError> {
    let canonical =
        std::fs::canonicalize(path).map_err(|source| BillCollectionError::WriteOutput {
            path: path.to_path_buf(),
            source,
        })?;
    let path = canonical
        .to_str()
        .ok_or_else(|| BillCollectionError::InvalidConfigPath {
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

fn sync_directory(path: &Path) -> Result<(), BillCollectionError> {
    let directory =
        std::fs::File::open(path).map_err(|source| BillCollectionError::WriteOutput {
            path: path.to_path_buf(),
            source,
        })?;
    directory
        .sync_all()
        .map_err(|source| BillCollectionError::WriteOutput {
            path: path.to_path_buf(),
            source,
        })
}

fn unix_time_ms() -> Result<u64, BillCollectionError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| BillCollectionError::Clock(error.to_string()))?
        .as_millis();
    u64::try_from(millis).map_err(|error| BillCollectionError::Clock(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::account_certification::NarrowEvidenceFake;

    fn response(body: String) -> Result<String, EvidenceReadError> {
        Ok(body)
    }

    fn time_response(timestamp_ms: u64) -> Result<String, EvidenceReadError> {
        response(format!(
            r#"{{"code":"0","msg":"","data":[{{"ts":"{timestamp_ms}"}}]}}"#
        ))
    }

    fn account_response(user_id: &str) -> Result<String, EvidenceReadError> {
        response(format!(
            r#"{{"code":"0","msg":"","data":[{{"acctLv":"2","posMode":"net_mode","acctStpMode":"cancel_maker","uid":"{user_id}","mainUid":"6"}}]}}"#
        ))
    }

    fn bill_response(timestamp_ms: u64) -> String {
        bill_page(timestamp_ms, 1)
    }

    fn bill_page(timestamp_ms: u64, count: usize) -> String {
        let data = (0..count)
            .map(|offset| {
                format!(
                    r#"{{"billId":"bill-{offset:03}","type":"8","subType":"174","ts":"{timestamp_ms}","ccy":"USDT","balChg":"1","pnl":"1","instType":"SWAP","instId":"BTC-USDT-SWAP","mgnMode":"cross","sz":"1","px":"50000"}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(r#"{{"code":"0","msg":"","data":[{data}]}}"#)
    }

    fn config() -> LiveConfig {
        LiveConfig::from_toml(include_str!("../../../examples/live-okx-demo.toml")).unwrap()
    }

    fn options(now_ms: u64) -> BillCollectionOptions {
        BillCollectionOptions {
            account_id: "main".to_string(),
            begin_ms: now_ms - 120_000,
            end_ms: now_ms - 60_000,
            max_pages: 3,
            page_interval_ms: MIN_BILL_COLLECTION_PAGE_INTERVAL_MS,
            minimum_window_close_delay_ms: 30_000,
        }
    }

    fn output_directory() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "reap-bill-collection-{}-{nonce}",
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

    async fn complete_fixture(directory: &Path) -> BillCollectionManifest {
        let now_ms = unix_time_ms().unwrap();
        let config_path = directory.join("live.toml");
        write_create_new(
            &config_path,
            include_str!("../../../examples/live-okx-demo.toml").as_bytes(),
        )
        .unwrap();
        let (config_file, _) = read_regular_file(&config_path, "live config").unwrap();
        let client = NarrowEvidenceFake::new(
            VecDeque::from([
                time_response(now_ms),
                account_response("7"),
                response(bill_response(now_ms - 90_000)),
                account_response("7"),
                time_response(now_ms + 1),
            ]),
            Arc::new(Mutex::new(Vec::new())),
        );
        collect_okx_bills_with_client(
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

    #[test]
    fn collection_options_require_bounded_paced_closed_window() {
        let now_ms = unix_time_ms().unwrap();
        let mut invalid = options(now_ms);
        invalid.begin_ms = 0;
        assert!(invalid.validate().is_err());

        let mut invalid = options(now_ms);
        invalid.max_pages = 0;
        assert!(invalid.validate().is_err());

        let mut invalid = options(now_ms);
        invalid.page_interval_ms = MIN_BILL_COLLECTION_PAGE_INTERVAL_MS - 1;
        assert!(invalid.validate().is_err());
    }

    #[tokio::test]
    async fn complete_collection_binds_exact_raw_page_account_window_and_permissions() {
        let now_ms = unix_time_ms().unwrap();
        let raw_bill = bill_response(now_ms - 90_000);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let client = NarrowEvidenceFake::new(
            VecDeque::from([
                time_response(now_ms),
                account_response("7"),
                response(raw_bill.clone()),
                account_response("7"),
                time_response(now_ms + 1),
            ]),
            Arc::clone(&requests),
        );
        let directory = output_directory();
        let collection_options = options(now_ms);

        let manifest = collect_okx_bills_with_client(
            &client,
            &config(),
            evidence(Path::new("/config")),
            &directory,
            &collection_options,
            provenance(),
        )
        .await
        .unwrap();

        assert!(manifest.complete);
        assert_eq!(manifest.total_rows, 1);
        assert_eq!(manifest.pages.len(), 1);
        assert_eq!(manifest.pages[0].response.bytes, raw_bill.len() as u64);
        assert_eq!(manifest.pages[0].response.sha256.len(), 64);
        assert_eq!(manifest.account_identity_sha256.len(), 64);
        assert_eq!(
            std::fs::read_to_string(&manifest.pages[0].response.path).unwrap(),
            raw_bill
        );
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 5);
        assert_eq!(
            requests[2],
            bills_request_path(collection_options.begin_ms, collection_options.end_ms, None)
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&manifest.pages[0].response.path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn out_of_window_bill_fails_after_preserving_raw_page() {
        let now_ms = unix_time_ms().unwrap();
        let client = NarrowEvidenceFake::new(
            VecDeque::from([
                time_response(now_ms),
                account_response("7"),
                response(bill_response(now_ms - 30_000)),
            ]),
            Arc::new(Mutex::new(Vec::new())),
        );
        let directory = output_directory();

        let error = collect_okx_bills_with_client(
            &client,
            &config(),
            evidence(Path::new("/config")),
            &directory,
            &options(now_ms),
            provenance(),
        )
        .await
        .unwrap_err();

        assert!(matches!(
            error,
            BillCollectionError::BillOutsideWindow { .. }
        ));
        assert!(directory.join("page-0001.json").is_file());
        assert!(!directory.join(BILL_COLLECTION_MANIFEST_NAME).exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn account_identity_change_fails_without_complete_manifest() {
        let now_ms = unix_time_ms().unwrap();
        let client = NarrowEvidenceFake::new(
            VecDeque::from([
                time_response(now_ms),
                account_response("7"),
                response(bill_response(now_ms - 90_000)),
                account_response("8"),
            ]),
            Arc::new(Mutex::new(Vec::new())),
        );
        let directory = output_directory();

        let error = collect_okx_bills_with_client(
            &client,
            &config(),
            evidence(Path::new("/config")),
            &directory,
            &options(now_ms),
            provenance(),
        )
        .await
        .unwrap_err();

        assert!(matches!(error, BillCollectionError::AccountIdentity(_)));
        assert!(directory.join("page-0001.json").is_file());
        assert!(!directory.join(BILL_COLLECTION_MANIFEST_NAME).exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn verifier_rebuilds_complete_collection_without_credentials() {
        let directory = output_directory();
        let manifest = complete_fixture(&directory).await;
        write_manifest(&directory, &manifest).unwrap();

        let verified =
            verify_bill_collection_manifest_path(directory.join(BILL_COLLECTION_MANIFEST_NAME))
                .unwrap();

        assert_eq!(verified.manifest, manifest);
        assert_eq!(verified.page_paths.len(), 1);
        assert_eq!(verified.bills.len(), 1);
        assert!(verified.summary().verification_passed);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn verifier_rejects_changed_response_and_unproven_request_path() {
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
            verify_bill_collection_manifest_path(directory.join(BILL_COLLECTION_MANIFEST_NAME))
                .unwrap_err();
        assert!(matches!(error, BillCollectionError::InvalidEvidence(_)));
        std::fs::remove_dir_all(directory).unwrap();

        let directory = output_directory();
        let mut manifest = complete_fixture(&directory).await;
        manifest.pages[0].request_path = format!(
            "/api/v5/account/bills?begin={}&end={}&after=unproven&limit=100",
            manifest.window.begin_ms, manifest.window.end_ms
        );
        write_manifest(&directory, &manifest).unwrap();

        let error =
            verify_bill_collection_manifest_path(directory.join(BILL_COLLECTION_MANIFEST_NAME))
                .unwrap_err();
        assert!(matches!(error, BillCollectionError::InvalidEvidence(_)));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn verifier_rejects_a_full_final_page_as_incomplete() {
        let directory = output_directory();
        let mut manifest = complete_fixture(&directory).await;
        let full_page = bill_page(manifest.window.begin_ms, 100);
        let page_path = PathBuf::from(&manifest.pages[0].response.path);
        std::fs::remove_file(&page_path).unwrap();
        write_create_new(&page_path, full_page.as_bytes()).unwrap();
        manifest.pages[0].response = file_evidence(&page_path, full_page.as_bytes()).unwrap();
        manifest.pages[0].rows = 100;
        manifest.pages[0].minimum_bill_time_ms = Some(manifest.window.begin_ms);
        manifest.pages[0].maximum_bill_time_ms = Some(manifest.window.begin_ms);
        manifest.pages[0].next_after = Some("bill-099".to_string());
        manifest.total_rows = 100;
        manifest.total_response_bytes = full_page.len() as u64;
        write_manifest(&directory, &manifest).unwrap();

        let error =
            verify_bill_collection_manifest_path(directory.join(BILL_COLLECTION_MANIFEST_NAME))
                .unwrap_err();

        assert!(matches!(error, BillCollectionError::InvalidEvidence(_)));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
