use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reap_core::{OrderStatus, PINNED_JAVA_REVISION, Side};
use reap_evidence_core::{
    EvidenceClientFactory, EvidenceClientFactoryError, EvidenceCredentialEnvironment,
    EvidenceHttpConfig, EvidenceReadError, EvidenceReadOnly,
};
use reap_storage::{RecoveredStorage, StorageError, acquire_storage_lease, recover_jsonl_bytes};
use reap_venue::PrivateOrderState;
use reap_venue::okx::{
    OkxAccountConfig, OkxOrderDetails, RestError, parse_okx_account_config_response_json,
    parse_okx_open_orders_response_json, parse_okx_order_details_response_json,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::form_urlencoded;

use crate::account_certification::{
    AccountCertificationClockEvidence, AccountCertificationConfigEvidence,
};
use crate::provenance::{
    current_executable_sha256, host_identity_sha256, okx_account_identity_sha256, sha256_bytes,
};
use crate::{LiveConfig, LiveConfigError, TradingEnvironment};

pub const DEADMAN_EXPIRY_CERTIFICATION_SCHEMA_VERSION: u32 = 1;
pub const OKX_DEADMAN_CANCEL_SOURCE: &str = "20";
pub const MAX_DEADMAN_CERTIFICATION_CONFIG_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_DEADMAN_CERTIFICATION_JOURNAL_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_DEADMAN_CERTIFICATION_RESPONSE_BYTES: u64 = 1024 * 1024;
pub const MAX_DEADMAN_CERTIFICATION_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_DEADMAN_CERTIFICATION_ORDERS: usize = 100;
pub const MAX_DEADMAN_CERTIFICATION_SPAN_MS: u64 = 120_000;

const ACCOUNT_CONFIG_ENDPOINT: &str = "/api/v5/account/config";
const ORDER_DETAILS_ENDPOINT: &str = "/api/v5/trade/order";
const OPEN_ORDERS_ENDPOINT: &str = "/api/v5/trade/orders-pending";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeadmanExpiryCertificationOptions {
    pub account_id: String,
    pub order_producers_stopped_attested: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeadmanExpiryCertificationCoverage {
    RecoveredRegularOrdersAndAccountWidePendingOrders,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeadmanCertificationResponseEvidence {
    pub request_path: String,
    pub bytes: u64,
    pub sha256: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeadmanBootstrapEvidence {
    pub account_id: String,
    pub strategy_name: String,
    pub config_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeadmanJournalEvidence {
    pub collector_path: String,
    pub bytes: u64,
    pub sha256: String,
    pub recovered_records: u64,
    pub recovered_last_ts_ms: u64,
    pub ignored_truncated_tail: bool,
    pub exclusive_lease_held_while_collecting: bool,
    pub bootstraps: Vec<DeadmanBootstrapEvidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeadmanRecoveredOrderEvidence {
    pub client_order_id: String,
    pub exchange_order_id: Option<String>,
    pub symbol: String,
    pub side: Side,
    pub status: OrderStatus,
    pub update_time_ms: u64,
    pub price: f64,
    pub qty: f64,
    pub filled_qty: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeadmanOrderDetailEvidence {
    pub client_order_id: String,
    pub exchange_order_id: String,
    pub symbol: String,
    pub response: DeadmanCertificationResponseEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum DeadmanCertificationFailure {
    OrderProducersStoppedNotAttested,
    JournalLeaseNotHeld,
    JournalIdentityMismatch,
    JournalTruncatedTail,
    UnmappedNonterminalOrders {
        count: usize,
    },
    PendingNewOrdersRecovered {
        count: usize,
    },
    NoLiveOrdersRecovered,
    MissingExchangeBinding {
        client_order_id: String,
    },
    MissingOrderDetail {
        client_order_id: String,
    },
    OrderIdentityMismatch {
        client_order_id: String,
    },
    OrderStateRegression {
        client_order_id: String,
    },
    OrderNotCancelled {
        client_order_id: String,
        state: PrivateOrderState,
    },
    CancelSourceMismatch {
        client_order_id: String,
        actual: String,
    },
    RegularOpenOrdersRemain {
        count: usize,
    },
    AccountIdentityUnstable,
    AccountSettingsChanged,
    AccountSettingsMismatch,
    ClockEvidenceInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeadmanExpiryCertificationSummary {
    pub coverage: DeadmanExpiryCertificationCoverage,
    pub environment: TradingEnvironment,
    pub account_id: String,
    pub account_identity_sha256: String,
    pub order_producers_stopped_attested: bool,
    pub journal_identity_matches: bool,
    pub journal_tail_complete: bool,
    pub account_identity_stable: bool,
    pub account_settings_stable: bool,
    pub account_settings_match: bool,
    pub clock_evidence_valid: bool,
    pub recovered_live_orders: usize,
    pub recovered_pending_new_orders: usize,
    pub deadman_cancelled_orders: usize,
    pub orders_all_deadman_cancelled: bool,
    pub regular_open_orders: usize,
    pub regular_open_orders_zero: bool,
    pub evidence_complete: bool,
    pub passed: bool,
    pub failures: Vec<DeadmanCertificationFailure>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeadmanExpiryCertificationArtifact {
    pub schema_version: u32,
    pub java_reference_revision: String,
    pub reap_version: String,
    pub executable_sha256: String,
    pub host_identity_sha256: String,
    pub config: AccountCertificationConfigEvidence,
    pub config_fingerprint: String,
    pub journal: DeadmanJournalEvidence,
    pub recovered_orders: Vec<DeadmanRecoveredOrderEvidence>,
    pub unmapped_nonterminal_order_ids: Vec<String>,
    pub start_clock: AccountCertificationClockEvidence,
    pub finish_clock: AccountCertificationClockEvidence,
    pub account_config_before: DeadmanCertificationResponseEvidence,
    pub order_details: Vec<DeadmanOrderDetailEvidence>,
    pub open_orders: DeadmanCertificationResponseEvidence,
    pub account_config_after: DeadmanCertificationResponseEvidence,
    pub summary: DeadmanExpiryCertificationSummary,
}

#[derive(Debug, Error)]
pub enum DeadmanExpiryCertificationError {
    #[error("invalid deadman-expiry certification options: {0}")]
    InvalidOptions(String),
    #[error("failed to reserve deadman-expiry output {path}: {source}")]
    ReserveOutput {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("deadman-expiry output resolves to the canonical journal {0}")]
    OutputIsJournal(PathBuf),
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
    #[error("live configuration is invalid: {0}")]
    Config(#[from] LiveConfigError),
    #[error("configured account {0} does not exist")]
    UnknownAccount(String),
    #[error("failed to fingerprint deadman-expiry provenance: {0}")]
    Provenance(String),
    #[error("failed to lease or recover the canonical journal: {0}")]
    Storage(#[from] StorageError),
    #[error("failed to read canonical journal {path}: {source}")]
    ReadJournal {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("canonical journal {path} is {actual} bytes; limit is {limit}")]
    JournalTooLarge {
        path: PathBuf,
        actual: u64,
        limit: u64,
    },
    #[error("journal contains {actual} selected nonterminal orders; limit is {limit}")]
    TooManyOrders { actual: usize, limit: usize },
    #[error("failed to initialize OKX transport: {0}")]
    Transport(#[source] RestError),
    #[error("OKX deadman-expiry certification failed: {0}")]
    Rest(#[from] RestError),
    #[error("OKX deadman-expiry certification failed: {0}")]
    Evidence(#[from] EvidenceReadError),
    #[error("deadman response for {request_path} is {actual} bytes; limit is {limit}")]
    ResponseTooLarge {
        request_path: String,
        actual: u64,
        limit: u64,
    },
    #[error("exchange clock evidence is invalid: {0}")]
    Clock(String),
    #[error("failed to serialize deadman-expiry artifact: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("failed to parse deadman-expiry artifact {path}: {source}")]
    ParseArtifact {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("deadman-expiry artifact is {actual} bytes; limit is {limit}")]
    ArtifactTooLarge { actual: u64, limit: u64 },
    #[error("failed to write deadman-expiry output {path}: {source}")]
    WriteOutput {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid deadman-expiry artifact path {path}: {message}")]
    InvalidArtifactPath { path: PathBuf, message: String },
    #[error("failed to read deadman-expiry artifact {path}: {source}")]
    ReadArtifact {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid deadman-expiry evidence: {0}")]
    InvalidEvidence(String),
}

fn evidence_credential_environment(
    account: &crate::LiveAccountConfig,
) -> EvidenceCredentialEnvironment {
    EvidenceCredentialEnvironment::new(
        &account.id,
        &account.api_key_env,
        &account.secret_key_env,
        &account.passphrase_env,
    )
}

fn map_factory_error(error: EvidenceClientFactoryError) -> DeadmanExpiryCertificationError {
    match error {
        EvidenceClientFactoryError::MissingCredential { account_id, name } => {
            DeadmanExpiryCertificationError::Config(LiveConfigError::MissingCredential {
                account_id,
                name,
            })
        }
        EvidenceClientFactoryError::InvalidConfiguration(message) => {
            DeadmanExpiryCertificationError::Transport(RestError::Transport(format!(
                "invalid evidence client configuration: {message}"
            )))
        }
        EvidenceClientFactoryError::Transport(message) => {
            DeadmanExpiryCertificationError::Transport(RestError::Transport(message))
        }
    }
}

/// Collects read-only proof that the exchange deadman cancelled the stopped
/// runtime's durable regular orders. The journal lease is acquired before
/// credentials are loaded or any network request is made.
pub async fn collect_deadman_expiry_certification_path<F>(
    config_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    options: DeadmanExpiryCertificationOptions,
    factory: &F,
) -> Result<DeadmanExpiryCertificationSummary, DeadmanExpiryCertificationError>
where
    F: EvidenceClientFactory,
    F::Client: EvidenceReadOnly<Error = EvidenceReadError>,
{
    validate_options(&options)?;
    let config_evidence = read_config(config_path.as_ref())?;
    let config = LiveConfig::from_toml(&config_evidence.toml)?;
    let account = config.account(&options.account_id).ok_or_else(|| {
        DeadmanExpiryCertificationError::UnknownAccount(options.account_id.clone())
    })?;
    let config_fingerprint = config.fingerprint()?;

    let lease = acquire_storage_lease(&config.storage.path)?;
    let journal_path = lease.journal_path().to_path_buf();
    let journal_bytes = read_journal(&journal_path)?;
    let recovered = recover_jsonl_bytes(&journal_bytes)?;
    let journal = journal_evidence(&journal_path, &journal_bytes, &recovered)?;
    let (recovered_orders, unmapped_nonterminal_order_ids) =
        select_recovered_orders(&config, &options.account_id, &recovered)?;

    let output_path = output_path.as_ref();
    let mut output = reserve_output(output_path)?;
    let canonical_output = std::fs::canonicalize(output_path).map_err(|source| {
        DeadmanExpiryCertificationError::ReserveOutput {
            path: output_path.to_path_buf(),
            source,
        }
    })?;
    if canonical_output == journal_path {
        return Err(DeadmanExpiryCertificationError::OutputIsJournal(
            journal_path,
        ));
    }

    let prepared = factory
        .prepare_credentials(&evidence_credential_environment(account))
        .map_err(map_factory_error)?;
    let executable_sha256 =
        current_executable_sha256().map_err(DeadmanExpiryCertificationError::Provenance)?;
    let host_identity_sha256 =
        host_identity_sha256().map_err(DeadmanExpiryCertificationError::Provenance)?;
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
    let artifact = collect_with_client(
        &client,
        &config,
        config_evidence,
        config_fingerprint,
        journal,
        recovered_orders,
        unmapped_nonterminal_order_ids,
        &options,
        executable_sha256,
        host_identity_sha256,
    )
    .await?;

    let mut bytes = serde_json::to_vec_pretty(&artifact)?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_DEADMAN_CERTIFICATION_ARTIFACT_BYTES {
        return Err(DeadmanExpiryCertificationError::ArtifactTooLarge {
            actual: bytes.len() as u64,
            limit: MAX_DEADMAN_CERTIFICATION_ARTIFACT_BYTES,
        });
    }
    output
        .write_all(&bytes)
        .and_then(|()| output.sync_all())
        .map_err(|source| DeadmanExpiryCertificationError::WriteOutput {
            path: output_path.to_path_buf(),
            source,
        })?;
    sync_parent(output_path)?;
    drop(lease);
    Ok(artifact.summary)
}

#[allow(clippy::too_many_arguments)]
async fn collect_with_client<C>(
    client: &C,
    config: &LiveConfig,
    config_evidence: AccountCertificationConfigEvidence,
    config_fingerprint: String,
    journal: DeadmanJournalEvidence,
    recovered_orders: Vec<DeadmanRecoveredOrderEvidence>,
    unmapped_nonterminal_order_ids: Vec<String>,
    options: &DeadmanExpiryCertificationOptions,
    executable_sha256: String,
    host_identity_sha256: String,
) -> Result<DeadmanExpiryCertificationArtifact, DeadmanExpiryCertificationError>
where
    C: EvidenceReadOnly<Error = EvidenceReadError> + ?Sized,
{
    let start_clock = sample_clock(client, config.runtime.max_exchange_clock_skew_ms).await?;
    let config_before_response = client.account_config().await?;
    let config_before =
        parse_okx_account_config_response_json(config_before_response.response_body().as_bytes())?;
    let (config_before_path, config_before_body) = config_before_response.into_parts();
    let account_config_before = response_evidence(
        ACCOUNT_CONFIG_ENDPOINT,
        &config_before_path,
        config_before_body,
    )?;

    let mut order_details = Vec::new();
    for recovered in recovered_orders.iter().filter(|order| {
        matches!(
            order.status,
            OrderStatus::Live | OrderStatus::PartiallyFilled
        )
    }) {
        let Some(exchange_order_id) = recovered.exchange_order_id.as_deref() else {
            continue;
        };
        let raw = client
            .regular_order_details(
                &recovered.symbol,
                exchange_order_id,
                &recovered.client_order_id,
            )
            .await?;
        parse_okx_order_details_response_json(raw.response_body().as_bytes())?;
        let expected_path = order_details_path(
            &recovered.symbol,
            exchange_order_id,
            &recovered.client_order_id,
        );
        let (request_path, response_body) = raw.into_parts();
        order_details.push(DeadmanOrderDetailEvidence {
            client_order_id: recovered.client_order_id.clone(),
            exchange_order_id: exchange_order_id.to_string(),
            symbol: recovered.symbol.clone(),
            response: response_evidence(&expected_path, &request_path, response_body)?,
        });
    }

    let open_response = client.regular_open_orders().await?;
    let open = parse_okx_open_orders_response_json(open_response.response_body().as_bytes())?;
    let (open_path, open_body) = open_response.into_parts();
    let open_orders = response_evidence(OPEN_ORDERS_ENDPOINT, &open_path, open_body)?;
    let config_after_response = client.account_config().await?;
    let config_after =
        parse_okx_account_config_response_json(config_after_response.response_body().as_bytes())?;
    let (config_after_path, config_after_body) = config_after_response.into_parts();
    let account_config_after = response_evidence(
        ACCOUNT_CONFIG_ENDPOINT,
        &config_after_path,
        config_after_body,
    )?;
    let finish_clock = sample_clock(client, config.runtime.max_exchange_clock_skew_ms).await?;
    let parsed_details = order_details
        .iter()
        .map(|evidence| {
            parse_okx_order_details_response_json(evidence.response.body.as_bytes())
                .map(|details| (evidence.client_order_id.clone(), details))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let summary = derive_summary(
        config,
        options,
        &journal,
        &recovered_orders,
        &unmapped_nonterminal_order_ids,
        &config_before,
        &parsed_details,
        &open,
        &config_after,
        &start_clock,
        &finish_clock,
    );

    Ok(DeadmanExpiryCertificationArtifact {
        schema_version: DEADMAN_EXPIRY_CERTIFICATION_SCHEMA_VERSION,
        java_reference_revision: PINNED_JAVA_REVISION.to_string(),
        reap_version: env!("CARGO_PKG_VERSION").to_string(),
        executable_sha256,
        host_identity_sha256,
        config: config_evidence,
        config_fingerprint,
        journal,
        recovered_orders,
        unmapped_nonterminal_order_ids,
        start_clock,
        finish_clock,
        account_config_before,
        order_details,
        open_orders,
        account_config_after,
        summary,
    })
}

/// Re-derives an artifact without credentials. The exact journal is supplied
/// separately so its hash and recovered order state are independently checked.
pub fn verify_deadman_expiry_certification_path(
    artifact_path: impl AsRef<Path>,
    journal_path: impl AsRef<Path>,
) -> Result<DeadmanExpiryCertificationSummary, DeadmanExpiryCertificationError> {
    Ok(verify_deadman_expiry_certification_artifact_path(artifact_path, journal_path)?.summary)
}

/// Re-derives an artifact and returns the exact validated artifact so callers
/// can bind its provenance without reopening an unchecked copy.
pub fn verify_deadman_expiry_certification_artifact_path(
    artifact_path: impl AsRef<Path>,
    journal_path: impl AsRef<Path>,
) -> Result<DeadmanExpiryCertificationArtifact, DeadmanExpiryCertificationError> {
    let artifact_path = artifact_path.as_ref();
    let bytes = read_artifact(artifact_path)?;
    let artifact: DeadmanExpiryCertificationArtifact =
        serde_json::from_slice(&bytes).map_err(|source| {
            DeadmanExpiryCertificationError::ParseArtifact {
                path: artifact_path.to_path_buf(),
                source,
            }
        })?;
    validate_artifact_header(&artifact)?;

    let config_bytes = artifact.config.toml.as_bytes();
    if artifact.config.bytes != config_bytes.len() as u64
        || artifact.config.sha256 != sha256_bytes(config_bytes)
    {
        return invalid_evidence("embedded live config byte count or SHA-256 does not match");
    }
    if artifact.config.bytes > MAX_DEADMAN_CERTIFICATION_CONFIG_BYTES {
        return Err(DeadmanExpiryCertificationError::ConfigTooLarge {
            path: PathBuf::from(&artifact.config.source_path),
            actual: artifact.config.bytes,
            limit: MAX_DEADMAN_CERTIFICATION_CONFIG_BYTES,
        });
    }
    let config = LiveConfig::from_toml(&artifact.config.toml)?;
    if config.fingerprint()? != artifact.config_fingerprint {
        return invalid_evidence("embedded live config fingerprint does not match");
    }
    if config.venue.environment != artifact.summary.environment {
        return invalid_evidence("embedded live config environment does not match summary");
    }
    if config.account(&artifact.summary.account_id).is_none() {
        return invalid_evidence("summary account does not exist in embedded live config");
    }

    let journal_path = canonical_regular_file(journal_path.as_ref(), "journal")?;
    let verification_lease = acquire_storage_lease(&journal_path)?;
    let journal_path = verification_lease.journal_path().to_path_buf();
    let journal_bytes = read_journal(&journal_path)?;
    let recovered = recover_jsonl_bytes(&journal_bytes)?;
    let derived_journal = journal_evidence(&journal_path, &journal_bytes, &recovered)?;
    if !journal_evidence_matches(&artifact.journal, &derived_journal) {
        return invalid_evidence("supplied journal does not match collected journal evidence");
    }
    let (recovered_orders, unmapped_nonterminal_order_ids) =
        select_recovered_orders(&config, &artifact.summary.account_id, &recovered)?;
    if artifact.recovered_orders != recovered_orders
        || artifact.unmapped_nonterminal_order_ids != unmapped_nonterminal_order_ids
    {
        return invalid_evidence("stored recovered orders do not match the supplied journal");
    }

    validate_response_evidence(&artifact.account_config_before, ACCOUNT_CONFIG_ENDPOINT)?;
    validate_response_evidence(&artifact.open_orders, OPEN_ORDERS_ENDPOINT)?;
    validate_response_evidence(&artifact.account_config_after, ACCOUNT_CONFIG_ENDPOINT)?;
    let config_before =
        parse_okx_account_config_response_json(artifact.account_config_before.body.as_bytes())?;
    let open_orders = parse_okx_open_orders_response_json(artifact.open_orders.body.as_bytes())?;
    let config_after =
        parse_okx_account_config_response_json(artifact.account_config_after.body.as_bytes())?;

    let expected_details = recovered_orders
        .iter()
        .filter(|order| {
            matches!(
                order.status,
                OrderStatus::Live | OrderStatus::PartiallyFilled
            ) && order.exchange_order_id.is_some()
        })
        .map(|order| (order.client_order_id.as_str(), order))
        .collect::<BTreeMap<_, _>>();
    let mut seen_details = BTreeSet::new();
    let mut parsed_details = Vec::with_capacity(artifact.order_details.len());
    for evidence in &artifact.order_details {
        if !seen_details.insert(evidence.client_order_id.as_str()) {
            return invalid_evidence(format!(
                "duplicate order-detail evidence for {}",
                evidence.client_order_id
            ));
        }
        let recovered = expected_details
            .get(evidence.client_order_id.as_str())
            .ok_or_else(|| {
                DeadmanExpiryCertificationError::InvalidEvidence(format!(
                    "order-detail evidence {} is not a bound recovered live order",
                    evidence.client_order_id
                ))
            })?;
        if recovered.exchange_order_id.as_deref() != Some(&evidence.exchange_order_id)
            || recovered.symbol != evidence.symbol
        {
            return invalid_evidence(format!(
                "order-detail query identity does not match recovered order {}",
                evidence.client_order_id
            ));
        }
        let expected_path = order_details_path(
            &evidence.symbol,
            &evidence.exchange_order_id,
            &evidence.client_order_id,
        );
        validate_response_evidence(&evidence.response, &expected_path)?;
        parsed_details.push((
            evidence.client_order_id.clone(),
            parse_okx_order_details_response_json(evidence.response.body.as_bytes())?,
        ));
    }

    let options = DeadmanExpiryCertificationOptions {
        account_id: artifact.summary.account_id.clone(),
        order_producers_stopped_attested: artifact.summary.order_producers_stopped_attested,
    };
    let derived = derive_summary(
        &config,
        &options,
        &artifact.journal,
        &recovered_orders,
        &unmapped_nonterminal_order_ids,
        &config_before,
        &parsed_details,
        &open_orders,
        &config_after,
        &artifact.start_clock,
        &artifact.finish_clock,
    );
    if artifact.summary != derived {
        return invalid_evidence("stored deadman-expiry summary does not match raw evidence");
    }
    Ok(artifact)
}

#[allow(clippy::too_many_arguments)]
fn derive_summary(
    config: &LiveConfig,
    options: &DeadmanExpiryCertificationOptions,
    journal: &DeadmanJournalEvidence,
    recovered_orders: &[DeadmanRecoveredOrderEvidence],
    unmapped_nonterminal_order_ids: &[String],
    config_before: &OkxAccountConfig,
    order_details: &[(String, OkxOrderDetails)],
    open_orders: &[reap_venue::RemoteOrder],
    config_after: &OkxAccountConfig,
    start_clock: &AccountCertificationClockEvidence,
    finish_clock: &AccountCertificationClockEvidence,
) -> DeadmanExpiryCertificationSummary {
    let mut failures = Vec::new();
    if !options.order_producers_stopped_attested {
        failures.push(DeadmanCertificationFailure::OrderProducersStoppedNotAttested);
    }
    if !journal.exclusive_lease_held_while_collecting {
        failures.push(DeadmanCertificationFailure::JournalLeaseNotHeld);
    }
    let journal_identity_matches = journal_identity_matches(config, journal, &options.account_id);
    if !journal_identity_matches {
        failures.push(DeadmanCertificationFailure::JournalIdentityMismatch);
    }
    let journal_tail_complete = !journal.ignored_truncated_tail;
    if !journal_tail_complete {
        failures.push(DeadmanCertificationFailure::JournalTruncatedTail);
    }
    if !unmapped_nonterminal_order_ids.is_empty() {
        failures.push(DeadmanCertificationFailure::UnmappedNonterminalOrders {
            count: unmapped_nonterminal_order_ids.len(),
        });
    }

    let recovered_pending_new_orders = recovered_orders
        .iter()
        .filter(|order| order.status == OrderStatus::PendingNew)
        .count();
    if recovered_pending_new_orders > 0 {
        failures.push(DeadmanCertificationFailure::PendingNewOrdersRecovered {
            count: recovered_pending_new_orders,
        });
    }
    let live_orders = recovered_orders
        .iter()
        .filter(|order| {
            matches!(
                order.status,
                OrderStatus::Live | OrderStatus::PartiallyFilled
            )
        })
        .collect::<Vec<_>>();
    if live_orders.is_empty() {
        failures.push(DeadmanCertificationFailure::NoLiveOrdersRecovered);
    }

    let details_by_client = order_details
        .iter()
        .map(|(client_order_id, details)| (client_order_id.as_str(), details))
        .collect::<BTreeMap<_, _>>();
    let mut deadman_cancelled_orders = 0;
    let mut detail_evidence_complete = order_details.len() == live_orders.len();
    for recovered in &live_orders {
        let Some(exchange_order_id) = recovered.exchange_order_id.as_deref() else {
            failures.push(DeadmanCertificationFailure::MissingExchangeBinding {
                client_order_id: recovered.client_order_id.clone(),
            });
            detail_evidence_complete = false;
            continue;
        };
        let Some(details) = details_by_client.get(recovered.client_order_id.as_str()) else {
            failures.push(DeadmanCertificationFailure::MissingOrderDetail {
                client_order_id: recovered.client_order_id.clone(),
            });
            detail_evidence_complete = false;
            continue;
        };
        let remote = &details.order;
        if remote.client_order_id != recovered.client_order_id
            || remote.exchange_order_id != exchange_order_id
            || remote.symbol != recovered.symbol
            || remote.side != recovered.side
            || remote.price != recovered.price
            || remote.qty != recovered.qty
        {
            failures.push(DeadmanCertificationFailure::OrderIdentityMismatch {
                client_order_id: recovered.client_order_id.clone(),
            });
            continue;
        }
        if remote.update_time_ms < recovered.update_time_ms
            || remote.cumulative_filled_qty < recovered.filled_qty
        {
            failures.push(DeadmanCertificationFailure::OrderStateRegression {
                client_order_id: recovered.client_order_id.clone(),
            });
            continue;
        }
        if remote.state != PrivateOrderState::Cancelled {
            failures.push(DeadmanCertificationFailure::OrderNotCancelled {
                client_order_id: recovered.client_order_id.clone(),
                state: remote.state,
            });
            continue;
        }
        if details.cancel_source != OKX_DEADMAN_CANCEL_SOURCE {
            failures.push(DeadmanCertificationFailure::CancelSourceMismatch {
                client_order_id: recovered.client_order_id.clone(),
                actual: details.cancel_source.clone(),
            });
            continue;
        }
        deadman_cancelled_orders += 1;
    }
    let recovered_live_orders = live_orders.len();
    let orders_all_deadman_cancelled = recovered_live_orders > 0
        && deadman_cancelled_orders == recovered_live_orders
        && detail_evidence_complete;

    let regular_open_orders = open_orders.len();
    let regular_open_orders_zero = regular_open_orders == 0;
    if !regular_open_orders_zero {
        failures.push(DeadmanCertificationFailure::RegularOpenOrdersRemain {
            count: regular_open_orders,
        });
    }

    let account_identity_stable = !config_before.user_id.trim().is_empty()
        && !config_before.main_user_id.trim().is_empty()
        && config_before.user_id == config_after.user_id
        && config_before.main_user_id == config_after.main_user_id;
    if !account_identity_stable {
        failures.push(DeadmanCertificationFailure::AccountIdentityUnstable);
    }
    let account_settings_stable = config_before == config_after;
    if !account_settings_stable {
        failures.push(DeadmanCertificationFailure::AccountSettingsChanged);
    }
    let expected = config.account(&options.account_id);
    let account_settings_match = expected.is_some_and(|expected| {
        config_before.account_level == expected.expected_account_level
            && config_before.position_mode == expected.expected_position_mode
    });
    if !account_settings_match {
        failures.push(DeadmanCertificationFailure::AccountSettingsMismatch);
    }
    let clock_evidence_valid = clock_is_valid(
        start_clock,
        finish_clock,
        config.runtime.max_exchange_clock_skew_ms,
    );
    if !clock_evidence_valid {
        failures.push(DeadmanCertificationFailure::ClockEvidenceInvalid);
    }

    let evidence_complete = detail_evidence_complete
        && unmapped_nonterminal_order_ids.is_empty()
        && journal_tail_complete;
    let passed = failures.is_empty()
        && evidence_complete
        && orders_all_deadman_cancelled
        && regular_open_orders_zero;
    DeadmanExpiryCertificationSummary {
        coverage: DeadmanExpiryCertificationCoverage::RecoveredRegularOrdersAndAccountWidePendingOrders,
        environment: config.venue.environment,
        account_id: options.account_id.clone(),
        account_identity_sha256: okx_account_identity_sha256(
            config.venue.environment,
            &options.account_id,
            &config_before.user_id,
            &config_before.main_user_id,
        ),
        order_producers_stopped_attested: options.order_producers_stopped_attested,
        journal_identity_matches,
        journal_tail_complete,
        account_identity_stable,
        account_settings_stable,
        account_settings_match,
        clock_evidence_valid,
        recovered_live_orders,
        recovered_pending_new_orders,
        deadman_cancelled_orders,
        orders_all_deadman_cancelled,
        regular_open_orders,
        regular_open_orders_zero,
        evidence_complete,
        passed,
        failures,
        limitations: vec![
            "the artifact does not itself prove SIGKILL or abnormal process exit; retain supervisor and injector evidence".to_string(),
            "the all-order-producers-stopped assertion is an operator attestation, not machine-verifiable evidence".to_string(),
            "the exclusive lease excludes cooperating Reap processes using this journal, not unrelated exchange clients".to_string(),
            "OKX orders-pending covers regular orders; algo and spread orders require separate controls".to_string(),
            "authenticated GETs are sequential rather than one atomic exchange snapshot".to_string(),
            "offline verification requires the exact journal bytes fingerprinted by the collector".to_string(),
        ],
    }
}

fn select_recovered_orders(
    config: &LiveConfig,
    account_id: &str,
    recovered: &RecoveredStorage,
) -> Result<(Vec<DeadmanRecoveredOrderEvidence>, Vec<String>), DeadmanExpiryCertificationError> {
    let bindings = recovered.order_bindings.get(account_id);
    let mut selected = Vec::new();
    let mut unmapped = Vec::new();
    for update in recovered.latest_orders.values().filter(|update| {
        matches!(
            update.status,
            OrderStatus::PendingNew | OrderStatus::Live | OrderStatus::PartiallyFilled
        )
    }) {
        let Some(owner) = config.account_for_symbol(&update.symbol) else {
            unmapped.push(update.order_id.clone());
            continue;
        };
        if owner.id != account_id {
            continue;
        }
        let exchange_order_id = bindings.and_then(|bindings| {
            bindings
                .iter()
                .find_map(|(exchange_order_id, client_order_id)| {
                    (client_order_id == &update.order_id).then(|| exchange_order_id.clone())
                })
        });
        selected.push(DeadmanRecoveredOrderEvidence {
            client_order_id: update.order_id.clone(),
            exchange_order_id,
            symbol: update.symbol.clone(),
            side: update.side,
            status: update.status,
            update_time_ms: update.ts_ms,
            price: update.price,
            qty: update.qty,
            filled_qty: update.filled_qty,
        });
    }
    selected.sort_by(|left, right| left.client_order_id.cmp(&right.client_order_id));
    unmapped.sort();
    if selected.len() > MAX_DEADMAN_CERTIFICATION_ORDERS {
        return Err(DeadmanExpiryCertificationError::TooManyOrders {
            actual: selected.len(),
            limit: MAX_DEADMAN_CERTIFICATION_ORDERS,
        });
    }
    Ok((selected, unmapped))
}

fn journal_identity_matches(
    config: &LiveConfig,
    journal: &DeadmanJournalEvidence,
    account_id: &str,
) -> bool {
    let Ok(config_fingerprint) = config.fingerprint() else {
        return false;
    };
    let selected_exists = journal.bootstraps.iter().any(|bootstrap| {
        bootstrap.account_id == account_id
            && bootstrap.strategy_name == config.strategy.strategy_name
            && bootstrap.config_fingerprint == config_fingerprint
    });
    selected_exists
        && journal.bootstraps.iter().all(|bootstrap| {
            config.account(&bootstrap.account_id).is_some()
                && bootstrap.strategy_name == config.strategy.strategy_name
                && bootstrap.config_fingerprint == config_fingerprint
        })
}

fn journal_evidence(
    path: &Path,
    bytes: &[u8],
    recovered: &RecoveredStorage,
) -> Result<DeadmanJournalEvidence, DeadmanExpiryCertificationError> {
    let collector_path = path
        .to_str()
        .ok_or_else(|| {
            DeadmanExpiryCertificationError::InvalidEvidence(
                "canonical journal path is not valid UTF-8".to_string(),
            )
        })?
        .to_string();
    let mut bootstraps = recovered
        .bootstrap_identities
        .iter()
        .map(
            |(account_id, (strategy_name, config_fingerprint))| DeadmanBootstrapEvidence {
                account_id: account_id.clone(),
                strategy_name: strategy_name.clone(),
                config_fingerprint: config_fingerprint.clone(),
            },
        )
        .collect::<Vec<_>>();
    bootstraps.sort_by(|left, right| left.account_id.cmp(&right.account_id));
    Ok(DeadmanJournalEvidence {
        collector_path,
        bytes: bytes.len() as u64,
        sha256: sha256_bytes(bytes),
        recovered_records: recovered.records,
        recovered_last_ts_ms: recovered.last_ts_ms,
        ignored_truncated_tail: recovered.ignored_truncated_tail,
        exclusive_lease_held_while_collecting: true,
        bootstraps,
    })
}

fn journal_evidence_matches(
    collected: &DeadmanJournalEvidence,
    derived: &DeadmanJournalEvidence,
) -> bool {
    collected.bytes == derived.bytes
        && collected.sha256 == derived.sha256
        && collected.recovered_records == derived.recovered_records
        && collected.recovered_last_ts_ms == derived.recovered_last_ts_ms
        && collected.ignored_truncated_tail == derived.ignored_truncated_tail
        && collected.exclusive_lease_held_while_collecting
        && collected.bootstraps == derived.bootstraps
}

async fn sample_clock<C>(
    client: &C,
    maximum_skew_ms: u64,
) -> Result<AccountCertificationClockEvidence, DeadmanExpiryCertificationError>
where
    C: EvidenceReadOnly<Error = EvidenceReadError> + ?Sized,
{
    let before = unix_time_ms()?;
    let server_ms = client.server_time_ms().await?;
    let after = unix_time_ms()?;
    let local_midpoint_ms = before.saturating_add(after.saturating_sub(before) / 2);
    let absolute_skew_ms = local_midpoint_ms.abs_diff(server_ms);
    if absolute_skew_ms > maximum_skew_ms {
        return Err(DeadmanExpiryCertificationError::Clock(format!(
            "absolute local/exchange skew {absolute_skew_ms} ms exceeds configured limit {maximum_skew_ms} ms"
        )));
    }
    Ok(AccountCertificationClockEvidence {
        local_midpoint_ms,
        server_ms,
        absolute_skew_ms,
    })
}

fn clock_is_valid(
    start: &AccountCertificationClockEvidence,
    finish: &AccountCertificationClockEvidence,
    maximum_skew_ms: u64,
) -> bool {
    start.absolute_skew_ms == start.local_midpoint_ms.abs_diff(start.server_ms)
        && finish.absolute_skew_ms == finish.local_midpoint_ms.abs_diff(finish.server_ms)
        && start.absolute_skew_ms <= maximum_skew_ms
        && finish.absolute_skew_ms <= maximum_skew_ms
        && finish.local_midpoint_ms >= start.local_midpoint_ms
        && finish.server_ms >= start.server_ms
        && finish.server_ms.saturating_sub(start.server_ms).max(
            finish
                .local_midpoint_ms
                .saturating_sub(start.local_midpoint_ms),
        ) <= MAX_DEADMAN_CERTIFICATION_SPAN_MS
}

fn order_details_path(symbol: &str, exchange_order_id: &str, client_order_id: &str) -> String {
    let mut query = form_urlencoded::Serializer::new(String::new());
    query.append_pair("instId", symbol);
    query.append_pair("ordId", exchange_order_id);
    query.append_pair("clOrdId", client_order_id);
    format!("{ORDER_DETAILS_ENDPOINT}?{}", query.finish())
}

fn response_evidence(
    expected_path: &str,
    request_path: &str,
    body: String,
) -> Result<DeadmanCertificationResponseEvidence, DeadmanExpiryCertificationError> {
    if request_path != expected_path {
        return invalid_evidence(format!(
            "collector requested {request_path:?}; expected {expected_path:?}"
        ));
    }
    let bytes = body.len() as u64;
    if bytes > MAX_DEADMAN_CERTIFICATION_RESPONSE_BYTES {
        return Err(DeadmanExpiryCertificationError::ResponseTooLarge {
            request_path: request_path.to_string(),
            actual: bytes,
            limit: MAX_DEADMAN_CERTIFICATION_RESPONSE_BYTES,
        });
    }
    Ok(DeadmanCertificationResponseEvidence {
        request_path: request_path.to_string(),
        bytes,
        sha256: sha256_bytes(body.as_bytes()),
        body,
    })
}

fn validate_response_evidence(
    response: &DeadmanCertificationResponseEvidence,
    expected_path: &str,
) -> Result<(), DeadmanExpiryCertificationError> {
    if response.request_path != expected_path {
        return invalid_evidence(format!(
            "response path {:?} does not match {expected_path:?}",
            response.request_path
        ));
    }
    if response.bytes > MAX_DEADMAN_CERTIFICATION_RESPONSE_BYTES {
        return Err(DeadmanExpiryCertificationError::ResponseTooLarge {
            request_path: expected_path.to_string(),
            actual: response.bytes,
            limit: MAX_DEADMAN_CERTIFICATION_RESPONSE_BYTES,
        });
    }
    if response.bytes != response.body.len() as u64
        || response.sha256 != sha256_bytes(response.body.as_bytes())
    {
        return invalid_evidence(format!(
            "response {expected_path} byte count or SHA-256 does not match"
        ));
    }
    Ok(())
}

fn validate_artifact_header(
    artifact: &DeadmanExpiryCertificationArtifact,
) -> Result<(), DeadmanExpiryCertificationError> {
    if artifact.schema_version != DEADMAN_EXPIRY_CERTIFICATION_SCHEMA_VERSION {
        return invalid_evidence(format!(
            "schema version {} is unsupported; expected {DEADMAN_EXPIRY_CERTIFICATION_SCHEMA_VERSION}",
            artifact.schema_version
        ));
    }
    if artifact.java_reference_revision != PINNED_JAVA_REVISION {
        return invalid_evidence("pinned Java revision does not match this verifier");
    }
    if artifact.reap_version.trim().is_empty() {
        return invalid_evidence("collector Reap version is empty");
    }
    for (label, value) in [
        ("collector executable", artifact.executable_sha256.as_str()),
        (
            "collector host identity",
            artifact.host_identity_sha256.as_str(),
        ),
        ("config", artifact.config.sha256.as_str()),
        ("config fingerprint", artifact.config_fingerprint.as_str()),
        ("journal", artifact.journal.sha256.as_str()),
        (
            "account identity",
            artifact.summary.account_identity_sha256.as_str(),
        ),
    ] {
        if !is_lower_sha256(value) {
            return invalid_evidence(format!("{label} SHA-256 is not lowercase hexadecimal"));
        }
    }
    validate_account_id(&artifact.summary.account_id)?;
    if artifact.config.source_path.trim().is_empty()
        || artifact.journal.collector_path.trim().is_empty()
    {
        return invalid_evidence("embedded config or journal source path is empty");
    }
    Ok(())
}

fn validate_options(
    options: &DeadmanExpiryCertificationOptions,
) -> Result<(), DeadmanExpiryCertificationError> {
    validate_account_id(&options.account_id)?;
    if !options.order_producers_stopped_attested {
        return Err(DeadmanExpiryCertificationError::InvalidOptions(
            "--confirm-order-producers-stopped is required".to_string(),
        ));
    }
    Ok(())
}

fn validate_account_id(account_id: &str) -> Result<(), DeadmanExpiryCertificationError> {
    if account_id.is_empty() || account_id.trim() != account_id {
        return Err(DeadmanExpiryCertificationError::InvalidOptions(
            "account id must be non-empty and contain no surrounding whitespace".to_string(),
        ));
    }
    if account_id.len() > 128 {
        return Err(DeadmanExpiryCertificationError::InvalidOptions(
            "account id exceeds 128 bytes".to_string(),
        ));
    }
    Ok(())
}

fn read_config(
    path: &Path,
) -> Result<AccountCertificationConfigEvidence, DeadmanExpiryCertificationError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        DeadmanExpiryCertificationError::InvalidConfigPath {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(DeadmanExpiryCertificationError::InvalidConfigPath {
            path: path.to_path_buf(),
            message: "must be a regular file and not a symbolic link".to_string(),
        });
    }
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        DeadmanExpiryCertificationError::InvalidConfigPath {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    let metadata = std::fs::metadata(&canonical).map_err(|source| {
        DeadmanExpiryCertificationError::ReadConfig {
            path: canonical.clone(),
            source,
        }
    })?;
    if metadata.len() > MAX_DEADMAN_CERTIFICATION_CONFIG_BYTES {
        return Err(DeadmanExpiryCertificationError::ConfigTooLarge {
            path: canonical,
            actual: metadata.len(),
            limit: MAX_DEADMAN_CERTIFICATION_CONFIG_BYTES,
        });
    }
    let bytes = std::fs::read(&canonical).map_err(|source| {
        DeadmanExpiryCertificationError::ReadConfig {
            path: canonical.clone(),
            source,
        }
    })?;
    let toml = String::from_utf8(bytes).map_err(|error| {
        DeadmanExpiryCertificationError::InvalidConfigPath {
            path: canonical.clone(),
            message: format!("config is not valid UTF-8: {error}"),
        }
    })?;
    if toml.len() as u64 > MAX_DEADMAN_CERTIFICATION_CONFIG_BYTES {
        return Err(DeadmanExpiryCertificationError::ConfigTooLarge {
            path: canonical,
            actual: toml.len() as u64,
            limit: MAX_DEADMAN_CERTIFICATION_CONFIG_BYTES,
        });
    }
    let source_path = canonical
        .to_str()
        .ok_or_else(|| DeadmanExpiryCertificationError::InvalidConfigPath {
            path: canonical.clone(),
            message: "canonical path is not valid UTF-8".to_string(),
        })?
        .to_string();
    Ok(AccountCertificationConfigEvidence {
        source_path,
        bytes: toml.len() as u64,
        sha256: sha256_bytes(toml.as_bytes()),
        toml,
    })
}

fn read_journal(path: &Path) -> Result<Vec<u8>, DeadmanExpiryCertificationError> {
    let metadata =
        std::fs::metadata(path).map_err(|source| DeadmanExpiryCertificationError::ReadJournal {
            path: path.to_path_buf(),
            source,
        })?;
    if metadata.len() > MAX_DEADMAN_CERTIFICATION_JOURNAL_BYTES {
        return Err(DeadmanExpiryCertificationError::JournalTooLarge {
            path: path.to_path_buf(),
            actual: metadata.len(),
            limit: MAX_DEADMAN_CERTIFICATION_JOURNAL_BYTES,
        });
    }
    let bytes =
        std::fs::read(path).map_err(|source| DeadmanExpiryCertificationError::ReadJournal {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() as u64 > MAX_DEADMAN_CERTIFICATION_JOURNAL_BYTES {
        return Err(DeadmanExpiryCertificationError::JournalTooLarge {
            path: path.to_path_buf(),
            actual: bytes.len() as u64,
            limit: MAX_DEADMAN_CERTIFICATION_JOURNAL_BYTES,
        });
    }
    Ok(bytes)
}

fn reserve_output(path: &Path) -> Result<File, DeadmanExpiryCertificationError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|source| DeadmanExpiryCertificationError::ReserveOutput {
            path: path.to_path_buf(),
            source,
        })
}

fn read_artifact(path: &Path) -> Result<Vec<u8>, DeadmanExpiryCertificationError> {
    let canonical = canonical_regular_file(path, "artifact")?;
    let metadata = std::fs::metadata(&canonical).map_err(|source| {
        DeadmanExpiryCertificationError::ReadArtifact {
            path: canonical.clone(),
            source,
        }
    })?;
    if metadata.len() > MAX_DEADMAN_CERTIFICATION_ARTIFACT_BYTES {
        return Err(DeadmanExpiryCertificationError::ArtifactTooLarge {
            actual: metadata.len(),
            limit: MAX_DEADMAN_CERTIFICATION_ARTIFACT_BYTES,
        });
    }
    let bytes = std::fs::read(&canonical).map_err(|source| {
        DeadmanExpiryCertificationError::ReadArtifact {
            path: canonical,
            source,
        }
    })?;
    if bytes.len() as u64 > MAX_DEADMAN_CERTIFICATION_ARTIFACT_BYTES {
        return Err(DeadmanExpiryCertificationError::ArtifactTooLarge {
            actual: bytes.len() as u64,
            limit: MAX_DEADMAN_CERTIFICATION_ARTIFACT_BYTES,
        });
    }
    Ok(bytes)
}

fn canonical_regular_file(
    path: &Path,
    label: &'static str,
) -> Result<PathBuf, DeadmanExpiryCertificationError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        DeadmanExpiryCertificationError::InvalidArtifactPath {
            path: path.to_path_buf(),
            message: format!("{label}: {error}"),
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(DeadmanExpiryCertificationError::InvalidArtifactPath {
            path: path.to_path_buf(),
            message: format!("{label} must be a regular file and not a symbolic link"),
        });
    }
    std::fs::canonicalize(path).map_err(|error| {
        DeadmanExpiryCertificationError::InvalidArtifactPath {
            path: path.to_path_buf(),
            message: format!("{label}: {error}"),
        }
    })
}

fn sync_parent(path: &Path) -> Result<(), DeadmanExpiryCertificationError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let directory =
        File::open(parent).map_err(|source| DeadmanExpiryCertificationError::WriteOutput {
            path: parent.to_path_buf(),
            source,
        })?;
    directory
        .sync_all()
        .map_err(|source| DeadmanExpiryCertificationError::WriteOutput {
            path: parent.to_path_buf(),
            source,
        })
}

fn unix_time_ms() -> Result<u64, DeadmanExpiryCertificationError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| DeadmanExpiryCertificationError::Clock(error.to_string()))?
        .as_millis();
    u64::try_from(millis).map_err(|error| DeadmanExpiryCertificationError::Clock(error.to_string()))
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn invalid_evidence<T>(message: impl Into<String>) -> Result<T, DeadmanExpiryCertificationError> {
    Err(DeadmanExpiryCertificationError::InvalidEvidence(
        message.into(),
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::{Arc, Mutex};

    use reap_core::{OrderEvent, OrderUpdate, TimeInForce};
    use reap_risk::RiskLimits;
    use reap_storage::{
        BootstrapRecord, OrderAckRecord, OrderAckStatus, OrderOperation, StorageConfig,
        StorageRecord, start_jsonl_storage,
    };
    use reap_strategy::ChaosConfig;
    use reap_venue::okx::{OkxAccountLevel, OkxApiKeyPermission, OkxPositionMode};

    use crate::account_certification::NarrowEvidenceFake;
    use crate::{
        AlertConfig, HostGuardConfig, LiveAccountConfig, LiveStorageConfig, OkxTradeModeConfig,
        OkxVenueConfig, OperatorConfig, RuntimeConfig,
    };

    use super::*;

    fn live_config(journal_path: PathBuf) -> LiveConfig {
        let mut strategy: ChaosConfig =
            toml::from_str(include_str!("../../../examples/iarb2-basic.toml")).unwrap();
        strategy.reference_data_stale_threshold_ms = Some(120_000);
        strategy.risk_groups[0].account_id = Some("main".to_string());
        let config = LiveConfig {
            strategy,
            risk: RiskLimits::default(),
            venue: OkxVenueConfig::default(),
            runtime: RuntimeConfig::default(),
            storage: LiveStorageConfig {
                path: journal_path,
                ..LiveStorageConfig::default()
            },
            operator: OperatorConfig::default(),
            alerts: AlertConfig::default(),
            host_guard: HostGuardConfig::default(),
            accounts: vec![LiveAccountConfig {
                id: "main".to_string(),
                api_key_env: "KEY".to_string(),
                secret_key_env: "SECRET".to_string(),
                passphrase_env: "PASS".to_string(),
                expected_account_level: OkxAccountLevel::Simple,
                expected_position_mode: OkxPositionMode::NetMode,
                api_key_policy: crate::OkxApiKeyPolicyConfig::default(),
                id_prefix: "reap".to_string(),
                node_id: 1,
                trade_modes: HashMap::from([
                    ("BTC-USDT".to_string(), OkxTradeModeConfig::Cash),
                    ("BTC-PERP".to_string(), OkxTradeModeConfig::Cross),
                ]),
            }],
        };
        assert!(config.validate().valid, "{:?}", config.validate().errors);
        config
    }

    async fn write_live_order_journal(config: &LiveConfig) {
        let mut storage = start_jsonl_storage(StorageConfig {
            path: config.storage.path.clone(),
            channel_capacity: 16,
            flush_every_records: 1,
        })
        .await
        .unwrap();
        let sink = storage.sink();
        sink.record_durable(StorageRecord::Bootstrap(BootstrapRecord {
            ts_ms: 900,
            account_id: "main".to_string(),
            strategy_name: config.strategy.strategy_name.clone(),
            config_fingerprint: config.fingerprint().unwrap(),
            baseline_fill_ids: Vec::new(),
        }))
        .await
        .unwrap();
        sink.record_durable(StorageRecord::OrderAck(OrderAckRecord {
            ts_ms: 950,
            account_id: "main".to_string(),
            operation: OrderOperation::Submit,
            client_order_id: "reap-order-1".to_string(),
            exchange_order_id: Some("123".to_string()),
            status: OrderAckStatus::Accepted,
            message: String::new(),
        }))
        .await
        .unwrap();
        sink.record_durable(StorageRecord::Order {
            account_id: Some("main".to_string()),
            update: OrderUpdate {
                ts_ms: 1_000,
                order_id: "reap-order-1".to_string(),
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                event: OrderEvent::New,
                status: OrderStatus::Live,
                price: 100.0,
                time_in_force: Some(TimeInForce::PostOnly),
                qty: 1.0,
                open_qty: 1.0,
                filled_qty: 0.0,
                avg_fill_price: 0.0,
                last_fill_qty: 0.0,
                last_fill_price: 0.0,
                last_fill_liquidity: None,
                last_fill_fee: None,
                reason: String::new(),
            },
        })
        .await
        .unwrap();
        storage.stop_writer().await.unwrap();
    }

    fn account_config_json() -> String {
        r#"{"code":"0","msg":"","data":[{"acctLv":"1","posMode":"net_mode","acctStpMode":"cancel_maker","uid":"7","mainUid":"6","label":"reap-demo","perm":"read_only,trade","ip":"203.0.113.5","enableSpotBorrow":false,"autoLoan":false,"spotBorrowAutoRepay":false}]}"#.to_string()
    }

    fn order_detail_json(cancel_source: &str) -> String {
        format!(
            r#"{{"code":"0","msg":"","data":[{{"ordId":"123","clOrdId":"reap-order-1","instId":"BTC-USDT","side":"buy","state":"canceled","px":"100","sz":"1","accFillSz":"0","avgPx":"","uTime":"2000","cancelSource":"{cancel_source}","cancelSourceReason":"Cancel all after triggered"}}]}}"#
        )
    }

    #[tokio::test]
    async fn read_only_artifact_round_trips_and_detects_tampering_and_live_ownership() {
        let now = unix_time_ms().unwrap();
        let root = std::env::temp_dir().join(format!(
            "reap-deadman-certification-{}-{now}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let journal_path = root.join("journal.jsonl");
        let config = live_config(journal_path.clone());
        write_live_order_journal(&config).await;

        let canonical_journal = std::fs::canonicalize(&journal_path).unwrap();
        let journal_bytes = read_journal(&canonical_journal).unwrap();
        let recovered = recover_jsonl_bytes(&journal_bytes).unwrap();
        let journal = journal_evidence(&canonical_journal, &journal_bytes, &recovered).unwrap();
        let (recovered_orders, unmapped) =
            select_recovered_orders(&config, "main", &recovered).unwrap();
        let config_toml = toml::to_string(&config).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let client = NarrowEvidenceFake::new(
            VecDeque::from([
                Ok(format!(
                    r#"{{"code":"0","msg":"","data":[{{"ts":"{now}"}}]}}"#
                )),
                Ok(account_config_json()),
                Ok(order_detail_json(OKX_DEADMAN_CANCEL_SOURCE)),
                Ok(r#"{"code":"0","msg":"","data":[]}"#.to_string()),
                Ok(account_config_json()),
                Ok(format!(
                    r#"{{"code":"0","msg":"","data":[{{"ts":"{}"}}]}}"#,
                    now + 1
                )),
            ]),
            Arc::clone(&requests),
        );
        let artifact = collect_with_client(
            &client,
            &config,
            AccountCertificationConfigEvidence {
                source_path: "/tmp/live.toml".to_string(),
                bytes: config_toml.len() as u64,
                sha256: sha256_bytes(config_toml.as_bytes()),
                toml: config_toml,
            },
            config.fingerprint().unwrap(),
            journal,
            recovered_orders,
            unmapped,
            &DeadmanExpiryCertificationOptions {
                account_id: "main".to_string(),
                order_producers_stopped_attested: true,
            },
            "a".repeat(64),
            "b".repeat(64),
        )
        .await
        .unwrap();
        assert!(artifact.summary.passed, "{:?}", artifact.summary.failures);
        assert_eq!(artifact.summary.deadman_cancelled_orders, 1);
        let requests = requests.lock().unwrap();
        assert_eq!(
            requests.iter().map(String::as_str).collect::<Vec<_>>(),
            vec![
                "/api/v5/public/time",
                ACCOUNT_CONFIG_ENDPOINT,
                "/api/v5/trade/order?instId=BTC-USDT&ordId=123&clOrdId=reap-order-1",
                OPEN_ORDERS_ENDPOINT,
                ACCOUNT_CONFIG_ENDPOINT,
                "/api/v5/public/time",
            ]
        );
        drop(requests);

        let mut stale_detail = parse_okx_order_details_response_json(
            artifact.order_details[0].response.body.as_bytes(),
        )
        .unwrap();
        stale_detail.order.update_time_ms = 999;
        let parsed_account =
            parse_okx_account_config_response_json(artifact.account_config_before.body.as_bytes())
                .unwrap();
        let stale_summary = derive_summary(
            &config,
            &DeadmanExpiryCertificationOptions {
                account_id: "main".to_string(),
                order_producers_stopped_attested: true,
            },
            &artifact.journal,
            &artifact.recovered_orders,
            &artifact.unmapped_nonterminal_order_ids,
            &parsed_account,
            &[("reap-order-1".to_string(), stale_detail)],
            &[],
            &parsed_account,
            &artifact.start_clock,
            &artifact.finish_clock,
        );
        assert!(stale_summary.failures.contains(
            &DeadmanCertificationFailure::OrderStateRegression {
                client_order_id: "reap-order-1".to_string(),
            }
        ));

        let artifact_path = root.join("artifact.json");
        std::fs::write(&artifact_path, serde_json::to_vec(&artifact).unwrap()).unwrap();
        assert_eq!(
            verify_deadman_expiry_certification_path(&artifact_path, &journal_path).unwrap(),
            artifact.summary
        );
        assert_eq!(
            verify_deadman_expiry_certification_artifact_path(&artifact_path, &journal_path)
                .unwrap(),
            artifact
        );

        let lease = acquire_storage_lease(&journal_path).unwrap();
        assert!(matches!(
            verify_deadman_expiry_certification_path(&artifact_path, &journal_path),
            Err(DeadmanExpiryCertificationError::Storage(
                StorageError::AlreadyLocked { .. }
            ))
        ));
        drop(lease);

        let mut tampered = artifact;
        let detail = &mut tampered.order_details[0].response;
        detail.body = detail
            .body
            .replace(r#""cancelSource":"20""#, r#""cancelSource":"1""#);
        detail.bytes = detail.body.len() as u64;
        detail.sha256 = sha256_bytes(detail.body.as_bytes());
        std::fs::write(&artifact_path, serde_json::to_vec(&tampered).unwrap()).unwrap();
        let error =
            verify_deadman_expiry_certification_path(&artifact_path, &journal_path).unwrap_err();
        assert!(error.to_string().contains("summary does not match"));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pending_new_and_missing_live_order_fail_closed() {
        let config = live_config(PathBuf::from("journal.jsonl"));
        let config_fingerprint = config.fingerprint().unwrap();
        let summary = derive_summary(
            &config,
            &DeadmanExpiryCertificationOptions {
                account_id: "main".to_string(),
                order_producers_stopped_attested: true,
            },
            &DeadmanJournalEvidence {
                collector_path: "/tmp/journal.jsonl".to_string(),
                bytes: 1,
                sha256: "a".repeat(64),
                recovered_records: 1,
                recovered_last_ts_ms: 1,
                ignored_truncated_tail: false,
                exclusive_lease_held_while_collecting: true,
                bootstraps: vec![DeadmanBootstrapEvidence {
                    account_id: "main".to_string(),
                    strategy_name: config.strategy.strategy_name.clone(),
                    config_fingerprint,
                }],
            },
            &[DeadmanRecoveredOrderEvidence {
                client_order_id: "pending".to_string(),
                exchange_order_id: None,
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                status: OrderStatus::PendingNew,
                update_time_ms: 1,
                price: 1.0,
                qty: 1.0,
                filled_qty: 0.0,
            }],
            &[],
            &OkxAccountConfig {
                account_level: OkxAccountLevel::Simple,
                position_mode: OkxPositionMode::NetMode,
                account_stp_mode: "cancel_maker".to_string(),
                user_id: "7".to_string(),
                main_user_id: "6".to_string(),
                api_key_label: "reap-demo".to_string(),
                api_key_permissions: BTreeSet::from([
                    OkxApiKeyPermission::ReadOnly,
                    OkxApiKeyPermission::Trade,
                ]),
                api_key_ip_bindings: BTreeSet::from(["203.0.113.5".to_string()]),
                enable_spot_borrow: Some(false),
                auto_loan: Some(false),
                spot_borrow_auto_repay: Some(false),
            },
            &[],
            &[],
            &OkxAccountConfig {
                account_level: OkxAccountLevel::Simple,
                position_mode: OkxPositionMode::NetMode,
                account_stp_mode: "cancel_maker".to_string(),
                user_id: "7".to_string(),
                main_user_id: "6".to_string(),
                api_key_label: "reap-demo".to_string(),
                api_key_permissions: BTreeSet::from([
                    OkxApiKeyPermission::ReadOnly,
                    OkxApiKeyPermission::Trade,
                ]),
                api_key_ip_bindings: BTreeSet::from(["203.0.113.5".to_string()]),
                enable_spot_borrow: Some(false),
                auto_loan: Some(false),
                spot_borrow_auto_repay: Some(false),
            },
            &AccountCertificationClockEvidence {
                local_midpoint_ms: 1,
                server_ms: 1,
                absolute_skew_ms: 0,
            },
            &AccountCertificationClockEvidence {
                local_midpoint_ms: 2,
                server_ms: 2,
                absolute_skew_ms: 0,
            },
        );
        assert!(!summary.passed);
        assert!(
            summary
                .failures
                .contains(&DeadmanCertificationFailure::PendingNewOrdersRecovered { count: 1 })
        );
        assert!(
            summary
                .failures
                .contains(&DeadmanCertificationFailure::NoLiveOrdersRecovered)
        );
    }
}
