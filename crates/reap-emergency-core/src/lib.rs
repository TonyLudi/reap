//! Credential-free contracts and verification for account-wide emergency stop.
//!
//! This crate deliberately contains no OKX adapter, HTTP client, signer,
//! credential reader, or production transport. The separate emergency runner
//! supplies a narrow role factory and coordinates independently progressing
//! regular, algo, and spread mitigation workflows.

use std::collections::{BTreeSet, HashMap};
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

mod verification;
pub use verification::*;

pub const MAX_INCIDENTS: usize = 64;
pub const MAX_INCIDENT_MESSAGE_BYTES: usize = 4_096;
pub const MAX_REMAINING_ORDER_DETAILS: usize = 100;
pub const ACCOUNT_WIDE_ORDER_SCOPE: &str = "okx_regular_algo_spread_orders";
pub const EXCLUDED_ORDER_CLASSES: [&str; 0] = [];
pub const EMERGENCY_CANCEL_REPORT_SCHEMA_VERSION: u32 = 2;
pub const ALGO_CANCEL_BATCH_LIMIT: usize = 10;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradingEnvironment {
    #[default]
    Demo,
    Production,
}

impl TradingEnvironment {
    pub const fn is_demo(self) -> bool {
        matches!(self, Self::Demo)
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmergencyAlgoOrderRef {
    pub symbol: String,
    pub algo_id: String,
    pub client_order_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmergencySpreadOrderRef {
    pub spread_id: String,
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
    pub spread_deadman_armed: bool,
    pub enumeration_attempts: u64,
    pub enumeration_failures: u64,
    pub initial_open_orders: Option<usize>,
    pub initial_algo_orders: Option<usize>,
    pub initial_spread_orders: Option<usize>,
    pub unique_orders_seen: usize,
    pub unique_algo_orders_seen: usize,
    pub unique_spread_orders_seen: usize,
    pub cancel_batches: u64,
    pub cancel_batch_failures: u64,
    pub accepted_cancel_requests: u64,
    pub rejected_cancel_requests: u64,
    pub unacknowledged_cancel_requests: u64,
    pub algo_cancel_batches: u64,
    pub algo_cancel_batch_failures: u64,
    pub accepted_algo_cancel_requests: u64,
    pub rejected_algo_cancel_requests: u64,
    pub unacknowledged_algo_cancel_requests: u64,
    pub spread_mass_cancel_attempts: u64,
    pub spread_mass_cancel_failures: u64,
    pub verified_zero_after_deadman: bool,
    pub verified_algo_zero_after_deadman: bool,
    pub verified_spread_zero_after_deadman: bool,
    pub final_open_orders: Option<usize>,
    pub final_algo_orders: Option<usize>,
    pub final_spread_orders: Option<usize>,
    pub unmanaged_symbols: Vec<String>,
    pub remaining_orders: Vec<EmergencyOrderRef>,
    pub remaining_algo_orders: Vec<EmergencyAlgoOrderRef>,
    pub remaining_spread_orders: Vec<EmergencySpreadOrderRef>,
    pub incident_count: u64,
    pub incidents: Vec<String>,
    pub elapsed_ms: u64,
    pub all_clear: bool,
}

impl EmergencyAccountReport {
    pub fn new(account_id: String) -> Self {
        Self {
            account_id,
            account_identity_sha256: None,
            exchange_clock_sampled: false,
            exchange_clock_skew_ms: None,
            deadman_armed: false,
            spread_deadman_armed: false,
            enumeration_attempts: 0,
            enumeration_failures: 0,
            initial_open_orders: None,
            initial_algo_orders: None,
            initial_spread_orders: None,
            unique_orders_seen: 0,
            unique_algo_orders_seen: 0,
            unique_spread_orders_seen: 0,
            cancel_batches: 0,
            cancel_batch_failures: 0,
            accepted_cancel_requests: 0,
            rejected_cancel_requests: 0,
            unacknowledged_cancel_requests: 0,
            algo_cancel_batches: 0,
            algo_cancel_batch_failures: 0,
            accepted_algo_cancel_requests: 0,
            rejected_algo_cancel_requests: 0,
            unacknowledged_algo_cancel_requests: 0,
            spread_mass_cancel_attempts: 0,
            spread_mass_cancel_failures: 0,
            verified_zero_after_deadman: false,
            verified_algo_zero_after_deadman: false,
            verified_spread_zero_after_deadman: false,
            final_open_orders: None,
            final_algo_orders: None,
            final_spread_orders: None,
            unmanaged_symbols: Vec::new(),
            remaining_orders: Vec::new(),
            remaining_algo_orders: Vec::new(),
            remaining_spread_orders: Vec::new(),
            incident_count: 0,
            incidents: Vec::new(),
            elapsed_ms: 0,
            all_clear: false,
        }
    }

    pub fn setup_failure(account_id: String, message: String) -> Self {
        let mut report = Self::new(account_id);
        report.push_incident(message);
        report
    }

    pub fn push_incident(&mut self, message: impl Into<String>) {
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
    /// SHA-256 of the exact emergency input file, without invoking a live parser.
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
    pub algo_orders_all_clear: bool,
    pub spread_orders_all_clear: bool,
    pub account_wide_orders_all_clear: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmergencyCompletion {
    pub regular_orders_all_clear: bool,
    pub algo_orders_all_clear: bool,
    pub spread_orders_all_clear: bool,
    pub account_wide_orders_all_clear: bool,
    pub evidence_complete: bool,
    pub all_clear: bool,
}

#[derive(Debug, Deserialize)]
pub struct EmergencyFileConfig {
    #[serde(default)]
    pub venue: EmergencyVenueConfig,
    #[serde(default)]
    pub runtime: EmergencyRuntimeConfig,
    pub accounts: Vec<EmergencyAccountConfig>,
}

#[derive(Debug, Clone)]
pub struct EmergencyConfigReview {
    pub environment: TradingEnvironment,
    pub account_ids: Vec<String>,
    pub validation_error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct EmergencyVenueConfig {
    pub environment: TradingEnvironment,
    pub rest_url: String,
}

impl Default for EmergencyVenueConfig {
    fn default() -> Self {
        Self {
            environment: TradingEnvironment::Demo,
            rest_url: "https://openapi.okx.com".to_string(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct EmergencyRuntimeConfig {
    pub rest_connect_timeout_ms: u64,
    pub rest_request_timeout_ms: u64,
    pub max_exchange_clock_skew_ms: u64,
    pub max_order_reconciliation_pages: usize,
    pub cancel_requests_per_window: usize,
    pub reconcile_requests_per_window: usize,
    pub request_window_ms: u64,
}

impl Default for EmergencyRuntimeConfig {
    fn default() -> Self {
        Self {
            rest_connect_timeout_ms: 2_000,
            rest_request_timeout_ms: 5_000,
            max_exchange_clock_skew_ms: 250,
            max_order_reconciliation_pages: 64,
            cancel_requests_per_window: 50,
            reconcile_requests_per_window: 20,
            request_window_ms: 2_000,
        }
    }
}

impl EmergencyRuntimeConfig {
    pub fn pacing_policy(&self) -> EmergencyPacingPolicy {
        EmergencyPacingPolicy {
            submit_requests: 1,
            cancel_requests: self.cancel_requests_per_window.min(20),
            reconcile_requests: self.reconcile_requests_per_window.min(10),
            window: Duration::from_millis(self.request_window_ms.max(2_000)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmergencyPacingPolicy {
    pub submit_requests: usize,
    pub cancel_requests: usize,
    pub reconcile_requests: usize,
    pub window: Duration,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EmergencyAccountConfig {
    pub id: String,
    pub api_key_env: String,
    pub secret_key_env: String,
    pub passphrase_env: String,
    #[serde(default)]
    pub trade_modes: HashMap<String, toml::Value>,
}

pub fn parse_emergency_config(text: &str) -> Result<EmergencyFileConfig, toml::de::Error> {
    toml::from_str(text)
}

pub fn review_emergency_config(
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

pub fn validate_and_select_accounts(
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
    if config.runtime.max_order_reconciliation_pages == 0
        || config.runtime.max_order_reconciliation_pages > 1_000
    {
        errors.push("emergency pending-order page bound is invalid".to_string());
    }
    let evidence_budget = Duration::from_secs(options.deadman_timeout_secs)
        .saturating_add(Duration::from_secs(2))
        .saturating_add(
            Duration::from_millis(config.runtime.rest_request_timeout_ms).saturating_mul(4),
        )
        .saturating_add(config.runtime.pacing_policy().window)
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
    let mut seen = std::collections::HashSet::new();
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
    let name = "emergency REST URL";
    let initial_errors = errors.len();
    let url = match Url::parse(&venue.rest_url) {
        Ok(url) => url,
        Err(error) => {
            errors.push(format!("{name} is invalid: {error}"));
            return;
        }
    };
    let Some(host) = url.host_str() else {
        errors.push(format!("{name} must contain a host"));
        return;
    };
    let loopback = is_loopback_host(host);
    let demo_loopback = venue.environment == TradingEnvironment::Demo && loopback;
    if url.scheme() != "https" && !(demo_loopback && url.scheme() == "http") {
        errors.push(format!(
            "{name} must use https (loopback http is demo-test only)"
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        errors.push(format!("{name} must not contain user information"));
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        errors.push(format!(
            "{name} must use exact path / without query or fragment"
        ));
    }
    if !demo_loopback && url.port_or_known_default() != Some(443) {
        errors.push(format!("{name} must use port 443"));
    }
    if errors.len() != initial_errors {
        return;
    }
    if loopback {
        if venue.environment == TradingEnvironment::Production {
            errors.push(format!("{name} loopback origin is demo-test only"));
        }
        return;
    }
    let allowed = match venue.environment {
        TradingEnvironment::Demo => [
            "openapi.okx.com",
            "www.okx.com",
            "us.okx.com",
            "eea.okx.com",
        ]
        .as_slice(),
        TradingEnvironment::Production => [
            "openapi.okx.com",
            "www.okx.com",
            "us.okx.com",
            "eea.okx.com",
            "tr.okx.com",
        ]
        .as_slice(),
    };
    if !allowed.contains(&host.to_ascii_lowercase().as_str()) {
        errors.push(format!(
            "{name} host is not a documented OKX REST origin for {:?}",
            venue.environment
        ));
    }
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegularOrder {
    pub symbol: String,
    pub exchange_order_id: String,
    pub client_order_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlgoOrder {
    pub algo_id: String,
    pub client_order_id: String,
    pub symbol: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpreadOrder {
    pub spread_id: String,
    pub exchange_order_id: String,
    pub client_order_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegularOrderPage {
    pub orders: Vec<RegularOrder>,
    pub next_after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlgoOrderPage {
    pub orders: Vec<AlgoOrder>,
    pub next_after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpreadOrderPage {
    pub orders: Vec<SpreadOrder>,
    pub next_end_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlgoOrderQuery {
    ConditionalAndOco,
    Chase,
    Trigger,
    MoveOrderStop,
    Iceberg,
    Twap,
    SmartIceberg,
}

impl AlgoOrderQuery {
    pub const ALL: [Self; 7] = [
        Self::ConditionalAndOco,
        Self::Chase,
        Self::Trigger,
        Self::MoveOrderStop,
        Self::Iceberg,
        Self::Twap,
        Self::SmartIceberg,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelOrder {
    pub symbol: String,
    pub exchange_order_id: Option<String>,
    pub client_order_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelOrderResult {
    pub exchange_order_id: String,
    pub client_order_id: String,
    pub code: String,
    pub message: String,
}

impl CancelOrderResult {
    pub fn accepted(&self) -> bool {
        self.code.is_empty() || self.code == "0"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelAlgoOrder {
    pub symbol: String,
    pub algo_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlgoCancelResult {
    pub algo_id: String,
    pub client_order_id: String,
    pub code: String,
    pub message: String,
}

impl AlgoCancelResult {
    pub fn accepted(&self) -> bool {
        self.code.is_empty() || self.code == "0"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmergencyAccountIdentity {
    pub user_id: String,
    pub main_user_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct EmergencyRoleError(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EmergencyRoleSetupError {
    #[error("{0}")]
    Credential(String),
    #[error("{0}")]
    Transport(String),
}

#[async_trait]
pub trait EmergencyAccountStopRole: Send + Sync {
    async fn server_time_ms(&self) -> Result<u64, EmergencyRoleError>;
    async fn account_identity_at(
        &self,
        timestamp: &str,
    ) -> Result<EmergencyAccountIdentity, EmergencyRoleError>;
    async fn regular_pending_orders_page_at(
        &self,
        timestamp: &str,
        after: Option<&str>,
    ) -> Result<RegularOrderPage, EmergencyRoleError>;
    async fn algo_pending_orders_page_at(
        &self,
        timestamp: &str,
        query: AlgoOrderQuery,
        after: Option<&str>,
    ) -> Result<AlgoOrderPage, EmergencyRoleError>;
    async fn spread_pending_orders_page_at(
        &self,
        timestamp: &str,
        end_id: Option<&str>,
    ) -> Result<SpreadOrderPage, EmergencyRoleError>;
    async fn cancel_all_after_at(
        &self,
        timestamp: &str,
        timeout_secs: u64,
    ) -> Result<(), EmergencyRoleError>;
    async fn spread_cancel_all_after_at(
        &self,
        timestamp: &str,
        timeout_secs: u64,
    ) -> Result<(), EmergencyRoleError>;
    async fn cancel_batch_orders_at(
        &self,
        timestamp: &str,
        orders: &[CancelOrder],
    ) -> Result<Vec<CancelOrderResult>, EmergencyRoleError>;
    async fn cancel_algo_orders_at(
        &self,
        timestamp: &str,
        orders: &[CancelAlgoOrder],
    ) -> Result<Vec<AlgoCancelResult>, EmergencyRoleError>;
    async fn spread_mass_cancel_at(&self, timestamp: &str) -> Result<(), EmergencyRoleError>;
}

pub trait EmergencyAccountStopFactory: Send + Sync {
    fn create(
        &self,
        venue: &EmergencyVenueConfig,
        runtime: &EmergencyRuntimeConfig,
        account: &EmergencyAccountConfig,
    ) -> Result<Box<dyn EmergencyAccountStopRole>, EmergencyRoleSetupError>;
}

#[derive(Debug)]
struct PendingOrderPagination<T> {
    domain: &'static str,
    max_pages: usize,
    pages: usize,
    cursor: Option<String>,
    orders: Vec<T>,
    seen_cursors: BTreeSet<String>,
    seen_order_ids: BTreeSet<String>,
}

impl<T> PendingOrderPagination<T> {
    fn new(domain: &'static str, max_pages: usize) -> Result<Self, EmergencyRoleError> {
        if max_pages == 0 {
            return Err(EmergencyRoleError(
                "invalid OKX response field max_pending_order_pages=\"0\": must be positive"
                    .to_string(),
            ));
        }
        Ok(Self {
            domain,
            max_pages,
            pages: 0,
            cursor: None,
            orders: Vec::new(),
            seen_cursors: BTreeSet::new(),
            seen_order_ids: BTreeSet::new(),
        })
    }

    fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    fn accept(
        &mut self,
        orders: Vec<T>,
        next_cursor: Option<String>,
        identity: impl Fn(&T) -> &str,
    ) -> Result<bool, EmergencyRoleError> {
        self.pages = self.pages.saturating_add(1);
        for order in &orders {
            let order_id = identity(order);
            if order_id.is_empty() || order_id.trim() != order_id {
                return Err(EmergencyRoleError(format!(
                    "invalid OKX response field pendingOrderId={order_id:?}: must be non-empty and contain no surrounding whitespace"
                )));
            }
            if !self.seen_order_ids.insert(order_id.to_string()) {
                return Err(EmergencyRoleError(format!(
                    "OKX {} pending-order pagination repeated order {order_id}",
                    self.domain
                )));
            }
        }
        self.orders.extend(orders);
        let Some(next_cursor) = next_cursor else {
            self.cursor = None;
            return Ok(true);
        };
        if !self.seen_cursors.insert(next_cursor.clone()) {
            return Err(EmergencyRoleError(format!(
                "OKX {} pending-order pagination repeated cursor {next_cursor}",
                self.domain
            )));
        }
        if self.pages >= self.max_pages {
            return Err(EmergencyRoleError(format!(
                "OKX {} pending-order pagination reached the configured limit after {} pages and {} records; next cursor={next_cursor}",
                self.domain,
                self.pages,
                self.orders.len()
            )));
        }
        self.cursor = Some(next_cursor);
        Ok(false)
    }
}

#[derive(Debug)]
pub struct RegularOrderPagination(PendingOrderPagination<RegularOrder>);

impl RegularOrderPagination {
    pub fn new(max_pages: usize) -> Result<Self, EmergencyRoleError> {
        PendingOrderPagination::new("regular", max_pages).map(Self)
    }
    pub fn after(&self) -> Option<&str> {
        self.0.cursor()
    }
    pub fn accept(&mut self, page: RegularOrderPage) -> Result<bool, EmergencyRoleError> {
        self.0.accept(page.orders, page.next_after, |order| {
            &order.exchange_order_id
        })
    }
    pub fn into_orders(self) -> Vec<RegularOrder> {
        self.0.orders
    }
}

#[derive(Debug)]
pub struct AlgoOrderPagination(PendingOrderPagination<AlgoOrder>);

impl AlgoOrderPagination {
    pub fn new(max_pages: usize) -> Result<Self, EmergencyRoleError> {
        PendingOrderPagination::new("algo", max_pages).map(Self)
    }
    pub fn after(&self) -> Option<&str> {
        self.0.cursor()
    }
    pub fn accept(&mut self, page: AlgoOrderPage) -> Result<bool, EmergencyRoleError> {
        self.0
            .accept(page.orders, page.next_after, |order| &order.algo_id)
    }
    pub fn into_orders(self) -> Vec<AlgoOrder> {
        self.0.orders
    }
}

#[derive(Debug)]
pub struct SpreadOrderPagination(PendingOrderPagination<SpreadOrder>);

impl SpreadOrderPagination {
    pub fn new(max_pages: usize) -> Result<Self, EmergencyRoleError> {
        PendingOrderPagination::new("spread", max_pages).map(Self)
    }
    pub fn end_id(&self) -> Option<&str> {
        self.0.cursor()
    }
    pub fn accept(&mut self, page: SpreadOrderPage) -> Result<bool, EmergencyRoleError> {
        self.0.accept(page.orders, page.next_end_id, |order| {
            &order.exchange_order_id
        })
    }
    pub fn into_orders(self) -> Vec<SpreadOrder> {
        self.0.orders
    }
}

pub fn emergency_completion(
    selected_accounts: &[String],
    reports: &[EmergencyAccountReport],
    config_file_sha256: &str,
    executable_sha256: Option<&str>,
    host_identity_sha256: Option<&str>,
    provenance_incidents_empty: bool,
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
        && reports
            .iter()
            .all(|report| report.deadman_armed && report.verified_zero_after_deadman);
    let algo_orders_all_clear = account_coverage_complete
        && !reports.is_empty()
        && reports
            .iter()
            .all(|report| report.verified_algo_zero_after_deadman);
    let spread_orders_all_clear = account_coverage_complete
        && !reports.is_empty()
        && reports
            .iter()
            .all(|report| report.spread_deadman_armed && report.verified_spread_zero_after_deadman);
    let account_wide_orders_all_clear = regular_orders_all_clear
        && algo_orders_all_clear
        && spread_orders_all_clear
        && reports.iter().all(|report| report.all_clear);
    let evidence_complete = provenance_incidents_empty
        && is_lower_sha256(config_file_sha256)
        && executable_sha256.is_some_and(is_lower_sha256)
        && host_identity_sha256.is_some_and(is_lower_sha256)
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
        algo_orders_all_clear,
        spread_orders_all_clear,
        account_wide_orders_all_clear,
        evidence_complete,
        all_clear: account_wide_orders_all_clear && evidence_complete,
    }
}

pub fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn truncate_utf8(mut value: String, maximum_bytes: usize) -> String {
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

pub fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emergency_pacing_preserves_okx_two_second_limits() {
        let runtime = EmergencyRuntimeConfig {
            request_window_ms: 1,
            cancel_requests_per_window: 50,
            reconcile_requests_per_window: 50,
            ..EmergencyRuntimeConfig::default()
        };
        let policy = runtime.pacing_policy();
        assert_eq!(policy.window, Duration::from_secs(2));
        assert_eq!(policy.cancel_requests, 20);
        assert_eq!(policy.reconcile_requests, 10);
    }

    #[test]
    fn incident_messages_are_utf8_safe_and_bounded() {
        let message = "\u{20ac}".repeat(2_000);
        let mut account = EmergencyAccountReport::new("main".to_string());
        account.push_incident(message);
        assert!(account.incidents[0].len() <= MAX_INCIDENT_MESSAGE_BYTES);
        assert!(account.incidents[0].ends_with('\u{20ac}'));
    }

    #[test]
    fn emergency_completion_requires_account_coverage_and_provenance() {
        let selected = vec!["main".to_string()];
        let mut account = EmergencyAccountReport::new("main".to_string());
        account.deadman_armed = true;
        account.spread_deadman_armed = true;
        account.verified_zero_after_deadman = true;
        account.verified_algo_zero_after_deadman = true;
        account.verified_spread_zero_after_deadman = true;
        account.all_clear = true;
        account.account_identity_sha256 = Some("4".repeat(64));

        let complete = emergency_completion(
            &selected,
            std::slice::from_ref(&account),
            &"1".repeat(64),
            Some(&"2".repeat(64)),
            Some(&"3".repeat(64)),
            true,
            0,
        );
        assert_eq!(
            complete,
            EmergencyCompletion {
                regular_orders_all_clear: true,
                algo_orders_all_clear: true,
                spread_orders_all_clear: true,
                account_wide_orders_all_clear: true,
                evidence_complete: true,
                all_clear: true,
            }
        );

        let mut missing_identity = account.clone();
        missing_identity.account_identity_sha256 = None;
        let incomplete = emergency_completion(
            &selected,
            &[missing_identity],
            &"1".repeat(64),
            Some(&"2".repeat(64)),
            Some(&"3".repeat(64)),
            true,
            0,
        );
        assert!(incomplete.regular_orders_all_clear);
        assert!(!incomplete.evidence_complete);
        assert!(!incomplete.all_clear);

        let missing_account = emergency_completion(
            &selected,
            &[],
            &"1".repeat(64),
            Some(&"2".repeat(64)),
            Some(&"3".repeat(64)),
            true,
            1,
        );
        assert!(!missing_account.regular_orders_all_clear);
        assert!(!missing_account.evidence_complete);
        assert!(!missing_account.all_clear);
    }

    #[test]
    fn emergency_completion_preserves_mixed_domain_results() {
        let selected = vec!["main".to_string()];

        for domain_results in [
            [false, true, true],
            [true, false, true],
            [true, true, false],
        ] {
            let mut account = EmergencyAccountReport::new("main".to_string());
            account.deadman_armed = true;
            account.spread_deadman_armed = true;
            account.verified_zero_after_deadman = domain_results[0];
            account.verified_algo_zero_after_deadman = domain_results[1];
            account.verified_spread_zero_after_deadman = domain_results[2];

            let completion = emergency_completion(
                &selected,
                &[account],
                &"1".repeat(64),
                Some(&"2".repeat(64)),
                Some(&"3".repeat(64)),
                true,
                0,
            );

            assert_eq!(
                [
                    completion.regular_orders_all_clear,
                    completion.algo_orders_all_clear,
                    completion.spread_orders_all_clear,
                ],
                domain_results
            );
            assert!(!completion.account_wide_orders_all_clear);
            assert!(!completion.evidence_complete);
            assert!(!completion.all_clear);
        }
    }

    #[test]
    fn emergency_parser_ignores_strategy_and_live_only_account_fields() {
        let config = parse_emergency_config(
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
    fn emergency_rest_validation_rejects_arbitrary_remote_origins() {
        let validate = |environment: TradingEnvironment, rest_url: &str| {
            let mut errors = Vec::new();
            validate_rest_url(
                &EmergencyVenueConfig {
                    environment,
                    rest_url: rest_url.to_string(),
                },
                &mut errors,
            );
            errors
        };

        assert!(validate(TradingEnvironment::Demo, "https://www.okx.com").is_empty());
        assert!(validate(TradingEnvironment::Demo, "http://127.0.0.1:18080").is_empty());
        assert!(
            validate(TradingEnvironment::Demo, "https://credentials.example")
                .iter()
                .any(|error| error.contains("documented OKX"))
        );
        assert!(
            validate(TradingEnvironment::Production, "http://127.0.0.1:18080")
                .iter()
                .any(|error| error.contains("must use https"))
        );
        assert!(
            validate(TradingEnvironment::Production, "https://127.0.0.1")
                .iter()
                .any(|error| error.contains("loopback origin is demo-test only"))
        );
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
