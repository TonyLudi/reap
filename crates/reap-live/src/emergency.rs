use std::collections::{BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reap_core::PINNED_JAVA_REVISION;
use reap_order::{PacingPolicy, RequestKind, RequestPacer};
use reap_venue::RemoteOrder;
use reap_venue::okx::{
    HttpTransport, OkxCancelOrder, OkxCancelOrderResult, OkxCredentials, OkxRestClient, OkxSigner,
    ReqwestTransport, RestError, format_okx_timestamp_ms,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::task::JoinSet;
use url::Url;

use crate::provenance::{
    current_executable_sha256, host_identity_sha256, okx_account_identity_sha256, sha256_bytes,
};
use crate::{OkxVenueConfig, RuntimeConfig, TradingEnvironment};

pub(crate) const MAX_INCIDENTS: usize = 64;
pub(crate) const MAX_INCIDENT_MESSAGE_BYTES: usize = 4_096;
pub(crate) const MAX_REMAINING_ORDER_DETAILS: usize = 100;
pub(crate) const REGULAR_ORDER_SCOPE: &str = "okx_regular_orders";
pub(crate) const EXCLUDED_ORDER_CLASSES: [&str; 2] = ["algo_orders", "spread_orders"];
pub const EMERGENCY_CANCEL_REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct EmergencyCancelOptions {
    pub account_ids: Vec<String>,
    pub all_configured_accounts: bool,
    pub confirm_account_wide_cancel: bool,
    pub confirm_order_producers_stopped: bool,
    pub confirm_production: bool,
    pub account_timeout: Duration,
    pub poll_interval: Duration,
    pub deadman_timeout_secs: u64,
}

impl Default for EmergencyCancelOptions {
    fn default() -> Self {
        Self {
            account_ids: Vec::new(),
            all_configured_accounts: false,
            confirm_account_wide_cancel: false,
            confirm_order_producers_stopped: false,
            confirm_production: false,
            account_timeout: Duration::from_secs(40),
            poll_interval: Duration::from_millis(250),
            deadman_timeout_secs: 10,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmergencyOrderRef {
    pub symbol: String,
    pub exchange_order_id: String,
    pub client_order_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmergencyAccountReport {
    pub account_id: String,
    pub account_identity_sha256: Option<String>,
    pub exchange_clock_sampled: bool,
    pub exchange_clock_skew_ms: Option<u64>,
    pub deadman_armed: bool,
    pub enumeration_attempts: u64,
    pub enumeration_failures: u64,
    pub initial_open_orders: Option<usize>,
    pub unique_orders_seen: usize,
    pub cancel_batches: u64,
    pub cancel_batch_failures: u64,
    pub accepted_cancel_requests: u64,
    pub rejected_cancel_requests: u64,
    pub unacknowledged_cancel_requests: u64,
    pub verified_zero_after_deadman: bool,
    pub final_open_orders: Option<usize>,
    pub unmanaged_symbols: Vec<String>,
    pub remaining_orders: Vec<EmergencyOrderRef>,
    pub incident_count: u64,
    pub incidents: Vec<String>,
    pub elapsed_ms: u64,
    pub all_clear: bool,
}

impl EmergencyAccountReport {
    fn new(account_id: String) -> Self {
        Self {
            account_id,
            account_identity_sha256: None,
            exchange_clock_sampled: false,
            exchange_clock_skew_ms: None,
            deadman_armed: false,
            enumeration_attempts: 0,
            enumeration_failures: 0,
            initial_open_orders: None,
            unique_orders_seen: 0,
            cancel_batches: 0,
            cancel_batch_failures: 0,
            accepted_cancel_requests: 0,
            rejected_cancel_requests: 0,
            unacknowledged_cancel_requests: 0,
            verified_zero_after_deadman: false,
            final_open_orders: None,
            unmanaged_symbols: Vec::new(),
            remaining_orders: Vec::new(),
            incident_count: 0,
            incidents: Vec::new(),
            elapsed_ms: 0,
            all_clear: false,
        }
    }

    fn setup_failure(account_id: String, message: String) -> Self {
        let mut report = Self::new(account_id);
        report.push_incident(message);
        report
    }

    fn push_incident(&mut self, message: impl Into<String>) {
        self.incident_count = self.incident_count.saturating_add(1);
        if self.incidents.len() < MAX_INCIDENTS {
            self.incidents
                .push(truncate_utf8(message.into(), MAX_INCIDENT_MESSAGE_BYTES));
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmergencyCancelReport {
    pub schema_version: u32,
    pub report_id: String,
    /// SHA-256 of the exact emergency input file, without invoking the live parser.
    pub config_file_sha256: String,
    pub java_reference_revision: String,
    pub reap_version: String,
    pub executable_sha256: Option<String>,
    pub host_identity_sha256: Option<String>,
    pub provenance_incident_count: u64,
    pub provenance_incidents: Vec<String>,
    pub environment: TradingEnvironment,
    pub scope: String,
    pub excluded_order_classes: Vec<String>,
    pub started_at_ms: u64,
    pub elapsed_ms: u64,
    pub account_timeout_ms: u64,
    pub poll_interval_ms: u64,
    pub deadman_timeout_secs: u64,
    pub selected_accounts: Vec<String>,
    pub accounts: Vec<EmergencyAccountReport>,
    pub execution_incident_count: u64,
    pub execution_incidents: Vec<String>,
    pub regular_orders_all_clear: bool,
    pub evidence_complete: bool,
    pub all_clear: bool,
}

#[derive(Debug, Error)]
pub enum EmergencyCancelError {
    #[error("failed to read emergency config {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse emergency config: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("emergency cancel configuration is invalid: {0}")]
    Invalid(String),
}

#[derive(Debug)]
struct EmergencyProvenance {
    config_file_sha256: String,
    executable_sha256: Option<String>,
    host_identity_sha256: Option<String>,
    incidents: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EmergencyCompletion {
    regular_orders_all_clear: bool,
    evidence_complete: bool,
    all_clear: bool,
}

#[derive(Debug, Deserialize)]
struct EmergencyFileConfig {
    #[serde(default)]
    venue: EmergencyVenueConfig,
    #[serde(default)]
    runtime: EmergencyRuntimeConfig,
    accounts: Vec<EmergencyAccountConfig>,
}

pub(crate) struct EmergencyConfigReview {
    pub environment: TradingEnvironment,
    pub account_ids: Vec<String>,
    pub validation_error: Option<String>,
}

pub(crate) fn review_emergency_config(
    text: &str,
    selected_accounts: &[String],
    account_timeout_ms: u64,
    poll_interval_ms: u64,
    deadman_timeout_secs: u64,
) -> Result<EmergencyConfigReview, toml::de::Error> {
    let config: EmergencyFileConfig = toml::from_str(text)?;
    let validation_error = validate_and_select_accounts(
        &config,
        &EmergencyCancelOptions {
            account_ids: selected_accounts.to_vec(),
            all_configured_accounts: false,
            confirm_account_wide_cancel: true,
            confirm_order_producers_stopped: true,
            confirm_production: true,
            account_timeout: Duration::from_millis(account_timeout_ms),
            poll_interval: Duration::from_millis(poll_interval_ms),
            deadman_timeout_secs,
        },
    )
    .err()
    .map(|error| error.to_string());
    Ok(EmergencyConfigReview {
        environment: config.venue.environment,
        account_ids: config
            .accounts
            .into_iter()
            .map(|account| account.id)
            .collect(),
        validation_error,
    })
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct EmergencyVenueConfig {
    environment: TradingEnvironment,
    rest_url: String,
}

impl Default for EmergencyVenueConfig {
    fn default() -> Self {
        let venue = OkxVenueConfig::default();
        Self {
            environment: venue.environment,
            rest_url: venue.rest_url,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct EmergencyRuntimeConfig {
    rest_connect_timeout_ms: u64,
    rest_request_timeout_ms: u64,
    max_exchange_clock_skew_ms: u64,
    cancel_requests_per_window: usize,
    reconcile_requests_per_window: usize,
    request_window_ms: u64,
}

impl Default for EmergencyRuntimeConfig {
    fn default() -> Self {
        let runtime = RuntimeConfig::default();
        Self {
            rest_connect_timeout_ms: runtime.rest_connect_timeout_ms,
            rest_request_timeout_ms: runtime.rest_request_timeout_ms,
            max_exchange_clock_skew_ms: runtime.max_exchange_clock_skew_ms,
            cancel_requests_per_window: runtime.cancel_requests_per_window,
            reconcile_requests_per_window: runtime.reconcile_requests_per_window,
            request_window_ms: runtime.request_window_ms,
        }
    }
}

impl EmergencyRuntimeConfig {
    fn pacing_policy(&self) -> PacingPolicy {
        PacingPolicy {
            submit_requests: 1,
            cancel_requests: self.cancel_requests_per_window,
            reconcile_requests: self.reconcile_requests_per_window,
            window: Duration::from_millis(self.request_window_ms),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct EmergencyAccountConfig {
    id: String,
    api_key_env: String,
    secret_key_env: String,
    passphrase_env: String,
    #[serde(default)]
    trade_modes: HashMap<String, toml::Value>,
}

impl EmergencyAccountConfig {
    fn credentials_from_env(&self) -> Result<OkxCredentials, String> {
        let read = |name: &str| {
            let value = std::env::var(name)
                .map_err(|_| format!("account {} is missing credential env {name}", self.id))?;
            if value.is_empty() {
                Err(format!(
                    "account {} has an empty credential env {name}",
                    self.id
                ))
            } else {
                Ok(value)
            }
        };
        Ok(OkxCredentials::new(
            read(&self.api_key_env)?,
            read(&self.secret_key_env)?,
            read(&self.passphrase_env)?,
        ))
    }
}

#[derive(Debug, Clone)]
struct AccountCancelSettings {
    environment: TradingEnvironment,
    account_timeout: Duration,
    poll_interval: Duration,
    verification_delay: Duration,
    pacing_policy: PacingPolicy,
    max_exchange_clock_skew_ms: u64,
    deadman_timeout_secs: u64,
}

#[derive(Debug, Clone)]
struct ExchangeClock {
    exchange_ms: u64,
    sampled_at: Instant,
}

impl ExchangeClock {
    fn local() -> Self {
        Self {
            exchange_ms: unix_time_ms(),
            sampled_at: Instant::now(),
        }
    }

    fn timestamp(&self) -> Result<String, RestError> {
        let elapsed_ms = self.sampled_at.elapsed().as_millis().min(u64::MAX as u128) as u64;
        format_okx_timestamp_ms(self.exchange_ms.saturating_add(elapsed_ms))
    }
}

pub async fn run_emergency_cancel_path(
    path: impl AsRef<Path>,
    options: EmergencyCancelOptions,
) -> Result<EmergencyCancelReport, EmergencyCancelError> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).map_err(|source| EmergencyCancelError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let provenance = collect_emergency_provenance(sha256_bytes(text.as_bytes()));
    let config: EmergencyFileConfig = toml::from_str(&text)?;
    run_emergency_cancel(config, options, provenance).await
}

async fn run_emergency_cancel(
    config: EmergencyFileConfig,
    options: EmergencyCancelOptions,
    provenance: EmergencyProvenance,
) -> Result<EmergencyCancelReport, EmergencyCancelError> {
    let selected = validate_and_select_accounts(&config, &options)?;
    let started_at_ms = unix_time_ms();
    let report_id = format!("{:x}", unix_time_ns());
    let started = Instant::now();
    let verification_delay =
        Duration::from_secs(options.deadman_timeout_secs).saturating_add(Duration::from_secs(2));
    let settings = AccountCancelSettings {
        environment: config.venue.environment,
        account_timeout: options.account_timeout,
        poll_interval: options.poll_interval,
        verification_delay,
        pacing_policy: config.runtime.pacing_policy(),
        max_exchange_clock_skew_ms: config.runtime.max_exchange_clock_skew_ms,
        deadman_timeout_secs: options.deadman_timeout_secs,
    };
    let mut selected_accounts = selected
        .iter()
        .map(|account| account.id.clone())
        .collect::<Vec<_>>();
    selected_accounts.sort();
    let mut reports = Vec::new();
    let mut tasks = JoinSet::new();

    for account in selected {
        let account_id = account.id.clone();
        let credentials = match account.credentials_from_env() {
            Ok(credentials) => credentials,
            Err(error) => {
                reports.push(EmergencyAccountReport::setup_failure(
                    account_id,
                    format!("credential setup failed: {error}"),
                ));
                continue;
            }
        };
        let transport = match ReqwestTransport::with_timeouts(
            &config.venue.rest_url,
            Duration::from_millis(config.runtime.rest_connect_timeout_ms),
            Duration::from_millis(config.runtime.rest_request_timeout_ms),
        ) {
            Ok(transport) => transport,
            Err(error) => {
                reports.push(EmergencyAccountReport::setup_failure(
                    account_id,
                    format!("REST transport setup failed: {error}"),
                ));
                continue;
            }
        };
        let signer = OkxSigner::new(credentials, config.venue.environment.is_demo());
        let client = OkxRestClient::new(transport, signer);
        let managed_symbols = account.trade_modes.keys().cloned().collect::<HashSet<_>>();
        let account_settings = settings.clone();
        tasks.spawn(async move {
            run_account_cancel(client, account_id, managed_symbols, account_settings).await
        });
    }
    let mut execution_incident_count = 0;
    let mut execution_incidents = Vec::new();
    collect_account_reports(
        &mut tasks,
        &mut reports,
        &mut execution_incident_count,
        &mut execution_incidents,
    )
    .await;
    reports.sort_by(|left, right| left.account_id.cmp(&right.account_id));
    let provenance_incident_count = provenance.incidents.len() as u64;
    let completion = emergency_completion(
        &selected_accounts,
        &reports,
        &provenance,
        execution_incident_count,
    );
    Ok(EmergencyCancelReport {
        schema_version: EMERGENCY_CANCEL_REPORT_SCHEMA_VERSION,
        report_id,
        config_file_sha256: provenance.config_file_sha256,
        java_reference_revision: PINNED_JAVA_REVISION.to_string(),
        reap_version: env!("CARGO_PKG_VERSION").to_string(),
        executable_sha256: provenance.executable_sha256,
        host_identity_sha256: provenance.host_identity_sha256,
        provenance_incident_count,
        provenance_incidents: provenance.incidents,
        environment: config.venue.environment,
        scope: REGULAR_ORDER_SCOPE.to_string(),
        excluded_order_classes: EXCLUDED_ORDER_CLASSES
            .into_iter()
            .map(str::to_string)
            .collect(),
        started_at_ms,
        elapsed_ms: elapsed_ms(&started),
        account_timeout_ms: duration_ms(options.account_timeout),
        poll_interval_ms: duration_ms(options.poll_interval),
        deadman_timeout_secs: options.deadman_timeout_secs,
        selected_accounts,
        accounts: reports,
        execution_incident_count,
        execution_incidents,
        regular_orders_all_clear: completion.regular_orders_all_clear,
        evidence_complete: completion.evidence_complete,
        all_clear: completion.all_clear,
    })
}

fn emergency_completion(
    selected_accounts: &[String],
    reports: &[EmergencyAccountReport],
    provenance: &EmergencyProvenance,
    execution_incident_count: u64,
) -> EmergencyCompletion {
    let reported_accounts = reports
        .iter()
        .map(|report| report.account_id.as_str())
        .collect::<Vec<_>>();
    let selected_accounts = selected_accounts
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let account_coverage_complete = reported_accounts == selected_accounts;
    let regular_orders_all_clear = account_coverage_complete
        && !reports.is_empty()
        && reports.iter().all(|report| report.all_clear);
    let evidence_complete = provenance.incidents.is_empty()
        && is_lower_sha256(&provenance.config_file_sha256)
        && provenance
            .executable_sha256
            .as_deref()
            .is_some_and(is_lower_sha256)
        && provenance
            .host_identity_sha256
            .as_deref()
            .is_some_and(is_lower_sha256)
        && execution_incident_count == 0
        && account_coverage_complete
        && reports.iter().all(|report| {
            report
                .account_identity_sha256
                .as_deref()
                .is_some_and(is_lower_sha256)
        });
    EmergencyCompletion {
        regular_orders_all_clear,
        evidence_complete,
        all_clear: regular_orders_all_clear && evidence_complete,
    }
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn collect_emergency_provenance(config_file_sha256: String) -> EmergencyProvenance {
    let mut incidents = Vec::new();
    let executable_sha256 = current_executable_sha256()
        .map_err(|error| incidents.push(format!("executable provenance failed: {error}")))
        .ok();
    let host_identity_sha256 = host_identity_sha256()
        .map_err(|error| incidents.push(format!("host provenance failed: {error}")))
        .ok();
    EmergencyProvenance {
        config_file_sha256,
        executable_sha256,
        host_identity_sha256,
        incidents,
    }
}

async fn collect_account_reports(
    tasks: &mut JoinSet<EmergencyAccountReport>,
    reports: &mut Vec<EmergencyAccountReport>,
    incident_count: &mut u64,
    incidents: &mut Vec<String>,
) {
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(report) => reports.push(report),
            Err(error) => push_incident(
                incident_count,
                incidents,
                format!("emergency account task failed: {error}"),
            ),
        }
    }
}

fn push_incident(count: &mut u64, incidents: &mut Vec<String>, message: String) {
    *count = count.saturating_add(1);
    if incidents.len() < MAX_INCIDENTS {
        incidents.push(truncate_utf8(message, MAX_INCIDENT_MESSAGE_BYTES));
    }
}

fn truncate_utf8(mut value: String, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value;
    }
    let mut boundary = maximum_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    value
}

fn validate_and_select_accounts(
    config: &EmergencyFileConfig,
    options: &EmergencyCancelOptions,
) -> Result<Vec<EmergencyAccountConfig>, EmergencyCancelError> {
    let mut errors = Vec::new();
    if !options.confirm_account_wide_cancel {
        errors.push("--confirm-account-wide-cancel is required".to_string());
    }
    if !options.confirm_order_producers_stopped {
        errors.push("--confirm-order-producers-stopped is required".to_string());
    }
    if config.venue.environment == TradingEnvironment::Production && !options.confirm_production {
        errors.push("--confirm-production is required for production credentials".to_string());
    }
    if options.all_configured_accounts != options.account_ids.is_empty() {
        errors.push(
            "select explicit --account values or --all-configured-accounts, but not both"
                .to_string(),
        );
    }
    if options.account_timeout.is_zero() || options.account_timeout > Duration::from_secs(300) {
        errors.push("account timeout must be between 1ms and 300s".to_string());
    }
    if options.poll_interval < Duration::from_millis(50)
        || options.poll_interval > Duration::from_secs(5)
    {
        errors.push("poll interval must be between 50ms and 5s".to_string());
    }
    if !(10..=120).contains(&options.deadman_timeout_secs) {
        errors.push("deadman timeout must be between 10 and 120 seconds".to_string());
    }
    if config.runtime.rest_connect_timeout_ms == 0
        || config.runtime.rest_request_timeout_ms < config.runtime.rest_connect_timeout_ms
    {
        errors.push("REST connect/request timeout configuration is invalid".to_string());
    }
    if config.runtime.cancel_requests_per_window == 0
        || config.runtime.reconcile_requests_per_window == 0
        || config.runtime.request_window_ms == 0
    {
        errors.push("emergency request pacing configuration is invalid".to_string());
    }
    let evidence_budget = Duration::from_secs(options.deadman_timeout_secs)
        .saturating_add(Duration::from_secs(2))
        .saturating_add(
            Duration::from_millis(config.runtime.rest_request_timeout_ms).saturating_mul(4),
        )
        .saturating_add(Duration::from_millis(config.runtime.request_window_ms))
        .saturating_add(options.poll_interval);
    if options.account_timeout < evidence_budget {
        errors.push(format!(
            "account timeout must be at least {}ms to reserve clock, deadman, pacing, and final-zero evidence budgets",
            duration_ms(evidence_budget)
        ));
    }
    validate_rest_url(&config.venue, &mut errors);

    let mut accounts_by_id = HashMap::new();
    for account in &config.accounts {
        if account.id.trim().is_empty() {
            errors.push("configured account id must not be empty".to_string());
        } else if accounts_by_id.insert(account.id.clone(), account).is_some() {
            errors.push(format!("duplicate configured account id {}", account.id));
        }
        for (field, value) in [
            ("api_key_env", &account.api_key_env),
            ("secret_key_env", &account.secret_key_env),
            ("passphrase_env", &account.passphrase_env),
        ] {
            if value.trim().is_empty() {
                errors.push(format!(
                    "configured account {} has empty {field}",
                    account.id
                ));
            }
        }
    }
    let requested_ids = if options.all_configured_accounts {
        accounts_by_id.keys().cloned().collect::<Vec<_>>()
    } else {
        options.account_ids.clone()
    };
    let mut seen = HashSet::new();
    let mut selected = Vec::new();
    for account_id in requested_ids {
        if account_id.trim().is_empty() {
            errors.push("requested account id must not be empty".to_string());
            continue;
        }
        if !seen.insert(account_id.clone()) {
            errors.push(format!("duplicate requested account id {account_id}"));
            continue;
        }
        match accounts_by_id.get(&account_id) {
            Some(account) => selected.push((*account).clone()),
            None => errors.push(format!("unknown configured account {account_id}")),
        }
    }
    if selected.is_empty() {
        errors.push("at least one configured account must be selected".to_string());
    }
    if errors.is_empty() {
        selected.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(selected)
    } else {
        errors.sort();
        errors.dedup();
        Err(EmergencyCancelError::Invalid(errors.join("; ")))
    }
}

fn validate_rest_url(venue: &EmergencyVenueConfig, errors: &mut Vec<String>) {
    match Url::parse(&venue.rest_url) {
        Ok(url) => {
            let loopback_http = url.scheme() == "http"
                && venue.environment == TradingEnvironment::Demo
                && url.host_str().is_some_and(|host| {
                    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
                });
            if url.scheme() != "https" && !loopback_http {
                errors.push(
                    "emergency REST URL must use HTTPS (loopback HTTP is demo-test only)"
                        .to_string(),
                );
            }
            if !url.username().is_empty() || url.password().is_some() {
                errors.push("emergency REST URL must not contain user information".to_string());
            }
            if url.host_str().is_none() {
                errors.push("emergency REST URL must contain a host".to_string());
            }
            if !matches!(url.path(), "" | "/") || url.query().is_some() || url.fragment().is_some()
            {
                errors.push(
                    "emergency REST URL must be an origin without path/query/fragment".to_string(),
                );
            }
        }
        Err(error) => errors.push(format!("emergency REST URL is invalid: {error}")),
    }
}

async fn run_account_cancel<T>(
    client: OkxRestClient<T>,
    account_id: String,
    managed_symbols: HashSet<String>,
    settings: AccountCancelSettings,
) -> EmergencyAccountReport
where
    T: HttpTransport + 'static,
{
    let started = Instant::now();
    let mut report = EmergencyAccountReport::new(account_id.clone());
    let (clock, sampled, skew_ms) = match run_bounded(
        started,
        settings.account_timeout,
        sample_exchange_clock(&client),
    )
    .await
    {
        Some(Ok((clock, skew_ms))) => (clock, true, Some(skew_ms)),
        Some(Err(error)) => {
            report.push_incident(format!(
                "exchange clock sampling failed; using local UTC for cancellation: {error}"
            ));
            (ExchangeClock::local(), false, None)
        }
        None => {
            report.push_incident(
                "account timeout expired while sampling exchange clock; cancellation could not start",
            );
            (ExchangeClock::local(), false, None)
        }
    };
    report.exchange_clock_sampled = sampled;
    report.exchange_clock_skew_ms = skew_ms;
    if skew_ms.is_some_and(|skew| skew > settings.max_exchange_clock_skew_ms) {
        report.push_incident(format!(
            "local/exchange clock skew {}ms exceeds configured maximum {}ms; exchange-adjusted timestamps are in use",
            skew_ms.unwrap_or_default(),
            settings.max_exchange_clock_skew_ms
        ));
    }

    match clock.timestamp() {
        Ok(timestamp) => match run_bounded(
            started,
            settings.account_timeout,
            client.cancel_all_after_at(&timestamp, settings.deadman_timeout_secs),
        )
        .await
        {
            Some(Ok(())) => report.deadman_armed = true,
            Some(Err(error)) => {
                report.push_incident(format!("failed to arm Cancel All After: {error}"));
            }
            None => report.push_incident("account timeout expired while arming Cancel All After"),
        },
        Err(error) => report.push_incident(format!("failed to format deadman timestamp: {error}")),
    }
    let verify_after = Instant::now() + settings.verification_delay;
    let mut pacer = RequestPacer::new(settings.pacing_policy.clone());
    let mut seen_orders = BTreeSet::new();
    let mut unmanaged_symbols = BTreeSet::new();
    let mut last_orders: Option<Vec<RemoteOrder>> = None;

    while started.elapsed() < settings.account_timeout {
        report.enumeration_attempts = report.enumeration_attempts.saturating_add(1);
        if run_bounded(
            started,
            settings.account_timeout,
            pacer.pace(RequestKind::Reconcile, &account_id),
        )
        .await
        .is_none()
        {
            report.push_incident("account timeout expired while pacing pending-order enumeration");
            break;
        }
        let timestamp = match clock.timestamp() {
            Ok(timestamp) => timestamp,
            Err(error) => {
                report.push_incident(format!("failed to format enumeration timestamp: {error}"));
                break;
            }
        };
        let orders = match run_bounded(
            started,
            settings.account_timeout,
            client.open_orders_at(&timestamp, None, None),
        )
        .await
        {
            Some(Ok(orders)) => orders,
            Some(Err(error)) => {
                report.enumeration_failures = report.enumeration_failures.saturating_add(1);
                report.push_incident(format!("pending-order enumeration failed: {error}"));
                sleep_bounded(settings.poll_interval, started, settings.account_timeout).await;
                continue;
            }
            None => {
                report.enumeration_failures = report.enumeration_failures.saturating_add(1);
                report.push_incident(
                    "account timeout expired during pending-order enumeration; zero was not proven",
                );
                break;
            }
        };
        if report.initial_open_orders.is_none() {
            report.initial_open_orders = Some(orders.len());
        }
        observe_orders(
            &orders,
            &managed_symbols,
            &mut seen_orders,
            &mut unmanaged_symbols,
            &mut report,
        );
        last_orders = Some(orders.clone());
        if orders.is_empty() {
            if Instant::now() >= verify_after {
                report.verified_zero_after_deadman = true;
                break;
            }
        } else {
            if !cancel_pending_orders(
                &client,
                &clock,
                &orders,
                &mut pacer,
                &mut report,
                started,
                &settings,
            )
            .await
            {
                break;
            }
        }
        sleep_bounded(settings.poll_interval, started, settings.account_timeout).await;
    }

    report.unique_orders_seen = seen_orders.len();
    report.unmanaged_symbols = unmanaged_symbols.into_iter().collect();
    report.final_open_orders = last_orders.as_ref().map(Vec::len);
    report.remaining_orders = last_orders
        .unwrap_or_default()
        .into_iter()
        .take(MAX_REMAINING_ORDER_DETAILS)
        .map(order_ref)
        .collect();
    if !report.verified_zero_after_deadman && started.elapsed() >= settings.account_timeout {
        report.push_incident("account timeout expired before regular orders were proven zero");
    }
    report.all_clear = report.deadman_armed && report.verified_zero_after_deadman;
    if report.all_clear {
        let account_identity_sha256 = sample_account_identity(
            &client,
            &clock,
            &account_id,
            settings.environment,
            started,
            settings.account_timeout,
            &mut report,
        )
        .await;
        report.account_identity_sha256 = account_identity_sha256;
    }
    report.elapsed_ms = elapsed_ms(&started);
    report
}

async fn sample_account_identity<T>(
    client: &OkxRestClient<T>,
    clock: &ExchangeClock,
    account_id: &str,
    environment: TradingEnvironment,
    started: Instant,
    account_timeout: Duration,
    report: &mut EmergencyAccountReport,
) -> Option<String>
where
    T: HttpTransport,
{
    let timestamp = match clock.timestamp() {
        Ok(timestamp) => timestamp,
        Err(error) => {
            report.push_incident(format!(
                "failed to format account-identity timestamp: {error}"
            ));
            return None;
        }
    };
    match run_bounded(
        started,
        account_timeout,
        client.account_config_at(&timestamp),
    )
    .await
    {
        Some(Ok(config))
            if !config.user_id.trim().is_empty() && !config.main_user_id.trim().is_empty() =>
        {
            Some(okx_account_identity_sha256(
                environment,
                account_id,
                &config.user_id,
                &config.main_user_id,
            ))
        }
        Some(Ok(_)) => {
            report.push_incident("exchange account identity response was empty");
            None
        }
        Some(Err(error)) => {
            report.push_incident(format!("exchange account identity query failed: {error}"));
            None
        }
        None => {
            report
                .push_incident("account timeout expired while querying exchange account identity");
            None
        }
    }
}

fn observe_orders(
    orders: &[RemoteOrder],
    managed_symbols: &HashSet<String>,
    seen_orders: &mut BTreeSet<(String, String)>,
    unmanaged_symbols: &mut BTreeSet<String>,
    report: &mut EmergencyAccountReport,
) {
    for (index, order) in orders.iter().enumerate() {
        if !managed_symbols.contains(&order.symbol) {
            unmanaged_symbols.insert(order.symbol.clone());
        }
        let identity = if !order.exchange_order_id.is_empty() {
            order.exchange_order_id.clone()
        } else if !order.client_order_id.is_empty() {
            order.client_order_id.clone()
        } else {
            report.push_incident(format!(
                "pending order {} at response index {index} has no exchange or client id",
                order.symbol
            ));
            format!("missing-id-{index}")
        };
        seen_orders.insert((order.symbol.clone(), identity));
    }
}

#[allow(clippy::too_many_arguments)]
async fn cancel_pending_orders<T>(
    client: &OkxRestClient<T>,
    clock: &ExchangeClock,
    orders: &[RemoteOrder],
    pacer: &mut RequestPacer,
    report: &mut EmergencyAccountReport,
    started: Instant,
    settings: &AccountCancelSettings,
) -> bool
where
    T: HttpTransport,
{
    let cancels = orders
        .iter()
        .filter_map(|order| {
            let exchange_order_id =
                (!order.exchange_order_id.is_empty()).then(|| order.exchange_order_id.clone());
            let client_order_id =
                (!order.client_order_id.is_empty()).then(|| order.client_order_id.clone());
            (exchange_order_id.is_some() || client_order_id.is_some()).then(|| OkxCancelOrder {
                symbol: order.symbol.clone(),
                exchange_order_id,
                client_order_id,
            })
        })
        .collect::<Vec<_>>();
    for batch in cancels.chunks(20) {
        if started.elapsed() >= settings.account_timeout {
            report.push_incident("account timeout expired before all cancel batches were sent");
            return false;
        }
        for cancel in batch {
            if run_bounded(
                started,
                settings.account_timeout,
                pacer.pace(RequestKind::Cancel, &cancel.symbol),
            )
            .await
            .is_none()
            {
                report.push_incident("account timeout expired while pacing cancel requests");
                return false;
            }
        }
        let timestamp = match clock.timestamp() {
            Ok(timestamp) => timestamp,
            Err(error) => {
                report.push_incident(format!("failed to format cancel timestamp: {error}"));
                return false;
            }
        };
        report.cancel_batches = report.cancel_batches.saturating_add(1);
        match run_bounded(
            started,
            settings.account_timeout,
            client.cancel_batch_orders_at(&timestamp, batch),
        )
        .await
        {
            Some(Ok(results)) => {
                if results.len() != batch.len() {
                    report.push_incident(format!(
                        "batch cancel returned {} results for {} orders",
                        results.len(),
                        batch.len()
                    ));
                }
                let mut matched = HashSet::new();
                for result in results {
                    match matching_cancel_index(batch, &result) {
                        Some(index) if matched.insert(index) => {}
                        Some(_) => report.push_incident(format!(
                            "batch cancel returned a duplicate result for order {}/{}",
                            result.exchange_order_id, result.client_order_id
                        )),
                        None => report.push_incident(format!(
                            "batch cancel returned an unknown result for order {}/{}",
                            result.exchange_order_id, result.client_order_id
                        )),
                    }
                    if result.accepted() {
                        report.accepted_cancel_requests =
                            report.accepted_cancel_requests.saturating_add(1);
                    } else {
                        report.rejected_cancel_requests =
                            report.rejected_cancel_requests.saturating_add(1);
                        report.push_incident(format!(
                            "cancel rejected for order {}/{}: {} {}",
                            result.exchange_order_id,
                            result.client_order_id,
                            result.code,
                            result.message
                        ));
                    }
                }
                let unacknowledged = batch.len().saturating_sub(matched.len()) as u64;
                report.unacknowledged_cancel_requests = report
                    .unacknowledged_cancel_requests
                    .saturating_add(unacknowledged);
                if unacknowledged > 0 {
                    report.push_incident(format!(
                        "batch cancel left {unacknowledged} request(s) without a matching acknowledgement"
                    ));
                }
            }
            Some(Err(error)) => {
                report.cancel_batch_failures = report.cancel_batch_failures.saturating_add(1);
                report.unacknowledged_cancel_requests = report
                    .unacknowledged_cancel_requests
                    .saturating_add(batch.len() as u64);
                report.push_incident(format!("batch cancel request failed: {error}"));
            }
            None => {
                report.cancel_batch_failures = report.cancel_batch_failures.saturating_add(1);
                report.unacknowledged_cancel_requests = report
                    .unacknowledged_cancel_requests
                    .saturating_add(batch.len() as u64);
                report.push_incident("account timeout expired during a batch cancel request");
                return false;
            }
        }
    }
    true
}

fn matching_cancel_index(batch: &[OkxCancelOrder], result: &OkxCancelOrderResult) -> Option<usize> {
    batch.iter().position(|cancel| {
        (!result.exchange_order_id.is_empty()
            && cancel.exchange_order_id.as_deref() == Some(result.exchange_order_id.as_str()))
            || (!result.client_order_id.is_empty()
                && cancel.client_order_id.as_deref() == Some(result.client_order_id.as_str()))
    })
}

async fn sample_exchange_clock<T>(
    client: &OkxRestClient<T>,
) -> Result<(ExchangeClock, u64), RestError>
where
    T: HttpTransport,
{
    let before_ms = unix_time_ms();
    let before = Instant::now();
    let exchange_ms = client.server_time_ms().await?;
    let after_ms = unix_time_ms();
    let round_trip = before.elapsed();
    let midpoint_ms = before_ms.saturating_add(after_ms.saturating_sub(before_ms) / 2);
    Ok((
        ExchangeClock {
            exchange_ms,
            sampled_at: before + round_trip / 2,
        },
        midpoint_ms.abs_diff(exchange_ms),
    ))
}

async fn sleep_bounded(interval: Duration, started: Instant, timeout: Duration) {
    let remaining = timeout.saturating_sub(started.elapsed());
    if !remaining.is_zero() {
        tokio::time::sleep(interval.min(remaining)).await;
    }
}

async fn run_bounded<F, T>(started: Instant, timeout: Duration, future: F) -> Option<T>
where
    F: Future<Output = T>,
{
    let remaining = timeout.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        return None;
    }
    tokio::time::timeout(remaining, future).await.ok()
}

fn order_ref(order: RemoteOrder) -> EmergencyOrderRef {
    EmergencyOrderRef {
        symbol: order.symbol,
        exchange_order_id: order.exchange_order_id,
        client_order_id: order.client_order_id,
    }
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn unix_time_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn elapsed_ms(started: &Instant) -> u64 {
    duration_ms(started.elapsed())
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
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
                .expect("mock response")
        }
    }

    #[derive(Clone)]
    struct HangingTransport;

    #[async_trait]
    impl HttpTransport for HangingTransport {
        async fn execute(&self, _request: SignedRequest) -> Result<HttpResponse, RestError> {
            std::future::pending().await
        }
    }

    fn response(body: &str) -> Result<HttpResponse, RestError> {
        Ok(HttpResponse {
            status: 200,
            body: body.to_string(),
        })
    }

    fn client(
        responses: Vec<Result<HttpResponse, RestError>>,
    ) -> (OkxRestClient<MockTransport>, Arc<Mutex<Vec<SignedRequest>>>) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let transport = MockTransport {
            responses: Arc::new(Mutex::new(responses.into())),
            requests: Arc::clone(&requests),
        };
        let signer = OkxSigner::new(OkxCredentials::new("key", "secret", "pass"), true);
        (OkxRestClient::new(transport, signer), requests)
    }

    fn settings() -> AccountCancelSettings {
        AccountCancelSettings {
            environment: TradingEnvironment::Demo,
            account_timeout: Duration::from_secs(1),
            poll_interval: Duration::from_millis(1),
            verification_delay: Duration::ZERO,
            pacing_policy: PacingPolicy {
                submit_requests: 100,
                cancel_requests: 100,
                reconcile_requests: 100,
                window: Duration::from_millis(1),
            },
            max_exchange_clock_skew_ms: 250,
            deadman_timeout_secs: 10,
        }
    }

    fn complete_provenance() -> EmergencyProvenance {
        EmergencyProvenance {
            config_file_sha256: "1".repeat(64),
            executable_sha256: Some("2".repeat(64)),
            host_identity_sha256: Some("3".repeat(64)),
            incidents: Vec::new(),
        }
    }

    #[test]
    fn emergency_completion_requires_account_coverage_and_provenance() {
        let selected = vec!["main".to_string()];
        let mut account = EmergencyAccountReport::new("main".to_string());
        account.all_clear = true;
        account.account_identity_sha256 = Some("4".repeat(64));
        let complete = emergency_completion(
            &selected,
            std::slice::from_ref(&account),
            &complete_provenance(),
            0,
        );
        assert_eq!(
            complete,
            EmergencyCompletion {
                regular_orders_all_clear: true,
                evidence_complete: true,
                all_clear: true,
            }
        );

        let mut missing_host = complete_provenance();
        missing_host.host_identity_sha256 = None;
        let incomplete =
            emergency_completion(&selected, std::slice::from_ref(&account), &missing_host, 0);
        assert!(incomplete.regular_orders_all_clear);
        assert!(!incomplete.evidence_complete);
        assert!(!incomplete.all_clear);

        let mut missing_identity = account.clone();
        missing_identity.account_identity_sha256 = None;
        let incomplete =
            emergency_completion(&selected, &[missing_identity], &complete_provenance(), 0);
        assert!(incomplete.regular_orders_all_clear);
        assert!(!incomplete.evidence_complete);
        assert!(!incomplete.all_clear);

        let missing_account = emergency_completion(&selected, &[], &complete_provenance(), 1);
        assert!(!missing_account.regular_orders_all_clear);
        assert!(!missing_account.evidence_complete);
        assert!(!missing_account.all_clear);
    }

    #[test]
    fn emergency_incident_messages_are_utf8_safe_and_bounded() {
        let message = "\u{20ac}".repeat(2_000);
        let mut account = EmergencyAccountReport::new("main".to_string());
        account.push_incident(message.clone());
        assert!(account.incidents[0].len() <= MAX_INCIDENT_MESSAGE_BYTES);
        assert!(account.incidents[0].ends_with('\u{20ac}'));

        let mut count = 0;
        let mut incidents = Vec::new();
        push_incident(&mut count, &mut incidents, message);
        assert_eq!(count, 1);
        assert!(incidents[0].len() <= MAX_INCIDENT_MESSAGE_BYTES);
        assert!(incidents[0].ends_with('\u{20ac}'));
    }

    #[tokio::test]
    async fn account_task_join_failure_becomes_bounded_evidence() {
        let mut tasks = JoinSet::new();
        let task = tasks.spawn(std::future::pending::<EmergencyAccountReport>());
        task.abort();
        let mut reports = Vec::new();
        let mut incident_count = 0;
        let mut incidents = Vec::new();

        collect_account_reports(
            &mut tasks,
            &mut reports,
            &mut incident_count,
            &mut incidents,
        )
        .await;

        assert!(reports.is_empty());
        assert_eq!(incident_count, 1);
        assert_eq!(incidents.len(), 1);
        assert!(incidents[0].contains("emergency account task failed"));
    }

    #[tokio::test]
    async fn credential_setup_failure_still_produces_schema_bound_evidence() {
        const MISSING_ENV: &str = "REAP_EMERGENCY_TEST_MISSING_CREDENTIAL_4F67A6E9";
        assert!(std::env::var_os(MISSING_ENV).is_none());
        let config = EmergencyFileConfig {
            venue: EmergencyVenueConfig::default(),
            runtime: EmergencyRuntimeConfig::default(),
            accounts: vec![EmergencyAccountConfig {
                id: "main".to_string(),
                api_key_env: MISSING_ENV.to_string(),
                secret_key_env: MISSING_ENV.to_string(),
                passphrase_env: MISSING_ENV.to_string(),
                trade_modes: HashMap::new(),
            }],
        };
        let options = EmergencyCancelOptions {
            account_ids: vec!["main".to_string()],
            confirm_account_wide_cancel: true,
            confirm_order_producers_stopped: true,
            ..EmergencyCancelOptions::default()
        };

        let report = run_emergency_cancel(config, options, complete_provenance())
            .await
            .unwrap();

        assert_eq!(
            report.schema_version,
            EMERGENCY_CANCEL_REPORT_SCHEMA_VERSION
        );
        assert_eq!(report.java_reference_revision, PINNED_JAVA_REVISION);
        assert_eq!(report.selected_accounts, ["main"]);
        assert_eq!(report.accounts.len(), 1);
        assert!(report.accounts[0].incidents[0].contains("credential setup failed"));
        assert!(!report.regular_orders_all_clear);
        assert!(!report.evidence_complete);
        assert!(!report.all_clear);
        let encoded = serde_json::to_vec(&report).unwrap();
        let decoded: EmergencyCancelReport = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.config_file_sha256, "1".repeat(64));
    }

    #[tokio::test]
    async fn emergency_cancel_arms_deadman_cancels_every_symbol_and_requeries_zero() {
        let (client, requests) = client(vec![
            response(r#"{"code":"0","msg":"","data":[{"ts":"1607418537715"}]}"#),
            response(
                r#"{"code":"0","msg":"","data":[{"triggerTime":"1607418547715","tag":"","ts":"1607418537715"}]}"#,
            ),
            response(
                r#"{"code":"0","msg":"","data":[{"ordId":"123","clOrdId":"reap1","instId":"BTC-USDT","side":"buy","state":"live","px":"100","sz":"1","accFillSz":"0","avgPx":"","uTime":"1000"},{"ordId":"456","clOrdId":"manual1","instId":"ETH-USDT","side":"sell","state":"partially_filled","px":"200","sz":"2","accFillSz":"1","avgPx":"201","uTime":"1001"}]}"#,
            ),
            response(
                r#"{"code":"0","msg":"","data":[{"ordId":"123","clOrdId":"reap1","sCode":"0","sMsg":""},{"ordId":"456","clOrdId":"manual1","sCode":"0","sMsg":""}]}"#,
            ),
            response(r#"{"code":"0","msg":"","data":[]}"#),
            response(
                r#"{"code":"0","msg":"","data":[{"acctLv":"2","posMode":"net_mode","acctStpMode":"cancel_maker","uid":"7","mainUid":"6"}]}"#,
            ),
        ]);

        let report = run_account_cancel(
            client,
            "main".to_string(),
            HashSet::from(["BTC-USDT".to_string()]),
            settings(),
        )
        .await;

        assert!(report.all_clear, "{:?}", report.incidents);
        assert!(report.deadman_armed);
        assert!(report.verified_zero_after_deadman);
        assert_eq!(report.initial_open_orders, Some(2));
        assert_eq!(report.unique_orders_seen, 2);
        assert_eq!(report.accepted_cancel_requests, 2);
        assert_eq!(report.final_open_orders, Some(0));
        assert_eq!(report.unmanaged_symbols, vec!["ETH-USDT"]);
        assert!(
            report
                .account_identity_sha256
                .as_deref()
                .is_some_and(is_lower_sha256)
        );
        let requests = requests.lock().unwrap();
        assert_eq!(requests[0].path, "/api/v5/public/time");
        assert_eq!(requests[1].path, "/api/v5/trade/cancel-all-after");
        assert_eq!(requests[2].path, "/api/v5/trade/orders-pending");
        assert_eq!(requests[3].path, "/api/v5/trade/cancel-batch-orders");
        assert_eq!(requests[4].path, "/api/v5/trade/orders-pending");
        assert_eq!(requests[5].path, "/api/v5/account/config");
    }

    #[tokio::test]
    async fn emergency_cancel_reports_deadman_failure_even_when_account_is_zero() {
        let (client, _) = client(vec![
            response(r#"{"code":"0","msg":"","data":[{"ts":"1607418537715"}]}"#),
            response(r#"{"code":"50000","msg":"deadman unavailable","data":[]}"#),
            response(r#"{"code":"0","msg":"","data":[]}"#),
        ]);

        let report =
            run_account_cancel(client, "main".to_string(), HashSet::new(), settings()).await;

        assert!(!report.all_clear);
        assert!(!report.deadman_armed);
        assert!(report.verified_zero_after_deadman);
        assert!(
            report
                .incidents
                .iter()
                .any(|incident| incident.contains("failed to arm Cancel All After"))
        );
    }

    #[tokio::test]
    async fn account_identity_failure_preserves_zero_proof_but_not_identity_evidence() {
        let (client, _) = client(vec![
            response(r#"{"code":"0","msg":"","data":[{"ts":"1607418537715"}]}"#),
            response(
                r#"{"code":"0","msg":"","data":[{"triggerTime":"1607418547715","tag":"","ts":"1607418537715"}]}"#,
            ),
            response(r#"{"code":"0","msg":"","data":[]}"#),
            response(r#"{"code":"50000","msg":"identity unavailable","data":[]}"#),
        ]);

        let report =
            run_account_cancel(client, "main".to_string(), HashSet::new(), settings()).await;

        assert!(report.all_clear);
        assert!(report.account_identity_sha256.is_none());
        assert!(
            report
                .incidents
                .iter()
                .any(|incident| incident.contains("account identity query failed"))
        );
    }

    #[tokio::test]
    async fn partial_batch_ack_is_reported_but_final_zero_is_authoritative() {
        let (client, _) = client(vec![
            response(r#"{"code":"0","msg":"","data":[{"ts":"1607418537715"}]}"#),
            response(
                r#"{"code":"0","msg":"","data":[{"triggerTime":"1607418547715","tag":"","ts":"1607418537715"}]}"#,
            ),
            response(
                r#"{"code":"0","msg":"","data":[{"ordId":"123","clOrdId":"reap1","instId":"BTC-USDT","side":"buy","state":"live","px":"100","sz":"1","accFillSz":"0","avgPx":"","uTime":"1000"},{"ordId":"456","clOrdId":"reap2","instId":"BTC-USDT","side":"sell","state":"live","px":"101","sz":"1","accFillSz":"0","avgPx":"","uTime":"1001"}]}"#,
            ),
            response(
                r#"{"code":"0","msg":"","data":[{"ordId":"123","clOrdId":"reap1","sCode":"51000","sMsg":"rejected"}]}"#,
            ),
            response(r#"{"code":"0","msg":"","data":[]}"#),
            response(
                r#"{"code":"0","msg":"","data":[{"acctLv":"2","posMode":"net_mode","acctStpMode":"cancel_maker","uid":"7","mainUid":"6"}]}"#,
            ),
        ]);

        let report =
            run_account_cancel(client, "main".to_string(), HashSet::new(), settings()).await;

        assert!(report.all_clear);
        assert!(report.account_identity_sha256.is_some());
        assert_eq!(report.rejected_cancel_requests, 1);
        assert_eq!(report.unacknowledged_cancel_requests, 1);
        assert_eq!(report.cancel_batch_failures, 0);
        assert!(
            report
                .incidents
                .iter()
                .any(|incident| incident.contains("without a matching acknowledgement"))
        );
    }

    #[tokio::test]
    async fn account_deadline_bounds_a_hung_transport() {
        let signer = OkxSigner::new(OkxCredentials::new("key", "secret", "pass"), true);
        let client = OkxRestClient::new(HangingTransport, signer);
        let mut bounded_settings = settings();
        bounded_settings.account_timeout = Duration::from_millis(20);
        let started = Instant::now();

        let report =
            run_account_cancel(client, "main".to_string(), HashSet::new(), bounded_settings).await;

        assert!(!report.all_clear);
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(
            report
                .incidents
                .iter()
                .any(|incident| incident.contains("account timeout"))
        );
    }

    #[test]
    fn emergency_parser_ignores_strategy_and_live_only_account_fields() {
        let config: EmergencyFileConfig = toml::from_str(
            r#"
strategy = "deliberately invalid for the live parser"

[venue]
environment = "demo"
rest_url = "https://www.okx.com"

[runtime]
event_channel_capacity = "deliberately invalid for the live parser"

[[accounts]]
id = "main"
api_key_env = "OKX_API_KEY"
secret_key_env = "OKX_SECRET_KEY"
passphrase_env = "OKX_PASSPHRASE"
expected_account_level = ["deliberately", "invalid"]
node_id = "deliberately invalid"

[accounts.trade_modes]
BTC-USDT = { deliberately = "invalid for the live parser" }
"#,
        )
        .unwrap();
        let options = EmergencyCancelOptions {
            account_ids: vec!["main".to_string()],
            confirm_account_wide_cancel: true,
            confirm_order_producers_stopped: true,
            ..EmergencyCancelOptions::default()
        };

        let selected = validate_and_select_accounts(&config, &options).unwrap();

        assert_eq!(selected.len(), 1);
        assert!(selected[0].trade_modes.contains_key("BTC-USDT"));
    }

    #[test]
    fn production_and_account_wide_confirmations_are_mandatory() {
        let config = EmergencyFileConfig {
            venue: EmergencyVenueConfig {
                environment: TradingEnvironment::Production,
                ..EmergencyVenueConfig::default()
            },
            runtime: EmergencyRuntimeConfig::default(),
            accounts: Vec::new(),
        };
        let error = validate_and_select_accounts(&config, &EmergencyCancelOptions::default())
            .unwrap_err()
            .to_string();

        assert!(error.contains("--confirm-account-wide-cancel"));
        assert!(error.contains("--confirm-order-producers-stopped"));
        assert!(error.contains("--confirm-production"));
        assert!(error.contains("at least one configured account"));
    }
}
