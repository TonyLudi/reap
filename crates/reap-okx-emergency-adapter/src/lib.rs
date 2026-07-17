//! Account-wide OKX cancellation authority for the separate emergency plane.
//!
//! The role intentionally contains only the emergency HTTP contract: sample
//! time, bind account identity, enumerate regular/algo/spread pending orders,
//! arm regular/spread Cancel All After, and cancel those three domains. It has
//! no submit, amend, transfer, withdrawal, or arbitrary-path operation.
//! Workflow ordering, retry, pacing, independent deadlines, reports, and final
//! zero proof remain outside this adapter.

use std::fmt;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reap_emergency_core as core;
use reap_okx_wire::{
    Client, Credentials, Error as WireError, ReqwestTransport, Response, Transport,
};
use reap_venue::okx::{
    OKX_ALGO_CANCEL_BATCH_LIMIT, OkxAlgoCancelResult, OkxAlgoOrderPage, OkxAlgoOrderQuery,
    OkxCancelAlgoOrder, OkxCancelOrder, OkxCancelOrderResult, OkxRegularOrderPage,
    OkxSpreadOrderPage, RestError,
};
use reap_venue::okx::{
    parse_okx_account_config_response_json, parse_okx_algo_order_page_response_json,
    parse_okx_cancel_all_after_response_json, parse_okx_cancel_order_results_response_json,
    parse_okx_regular_order_page_response_json, parse_okx_server_time_response_json,
    parse_okx_spread_order_page_response_json,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

const PUBLIC_TIME_PATH: &str = "/api/v5/public/time";
const ACCOUNT_CONFIG_PATH: &str = "/api/v5/account/config";
const REGULAR_PENDING_PATH: &str = "/api/v5/trade/orders-pending";
const ALGO_PENDING_PATH: &str = "/api/v5/trade/orders-algo-pending";
const SPREAD_PENDING_PATH: &str = "/api/v5/sprd/orders-pending";
const REGULAR_CANCEL_ALL_AFTER_PATH: &str = "/api/v5/trade/cancel-all-after";
const REGULAR_CANCEL_BATCH_PATH: &str = "/api/v5/trade/cancel-batch-orders";
const ALGO_CANCEL_PATH: &str = "/api/v5/trade/cancel-algos";
const SPREAD_MASS_CANCEL_PATH: &str = "/api/v5/sprd/mass-cancel";
const SPREAD_CANCEL_ALL_AFTER_PATH: &str = "/api/v5/sprd/cancel-all-after";
const REGULAR_CANCEL_BATCH_LIMIT: usize = 20;

/// The complete HTTP authority held by the emergency account-stop role.
pub const EMERGENCY_HTTP_ALLOWLIST: &[(&str, &str)] = &[
    ("GET", PUBLIC_TIME_PATH),
    ("GET", ACCOUNT_CONFIG_PATH),
    ("GET", REGULAR_PENDING_PATH),
    ("GET", ALGO_PENDING_PATH),
    ("GET", SPREAD_PENDING_PATH),
    ("POST", REGULAR_CANCEL_ALL_AFTER_PATH),
    ("POST", REGULAR_CANCEL_BATCH_PATH),
    ("POST", ALGO_CANCEL_PATH),
    ("POST", SPREAD_MASS_CANCEL_PATH),
    ("POST", SPREAD_CANCEL_ALL_AFTER_PATH),
];

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("missing or empty OKX credential environment variable {0}")]
    MissingCredential(String),
    #[error("invalid OKX emergency adapter configuration: {0}")]
    InvalidConfiguration(String),
    #[error("invalid OKX emergency credential material: {0}")]
    InvalidCredential(String),
    #[error("{0}")]
    Wire(String),
}

/// Environment-variable names from which the isolated emergency executable
/// obtains one account's credentials.
#[derive(Clone)]
pub struct CredentialEnvNames {
    api_key: String,
    secret_key: String,
    passphrase: String,
}

impl CredentialEnvNames {
    pub fn new(
        api_key: impl Into<String>,
        secret_key: impl Into<String>,
        passphrase: impl Into<String>,
    ) -> Result<Self, AdapterError> {
        let names = Self {
            api_key: api_key.into(),
            secret_key: secret_key.into(),
            passphrase: passphrase.into(),
        };
        for name in [&names.api_key, &names.secret_key, &names.passphrase] {
            if name.trim().is_empty() || name.trim() != name {
                return Err(AdapterError::InvalidConfiguration(
                    "credential environment names must be non-empty and trimmed".to_string(),
                ));
            }
        }
        Ok(names)
    }

    fn read(&self) -> Result<Credentials, AdapterError> {
        let read = |name: &str| {
            std::env::var(name)
                .ok()
                .ok_or_else(|| AdapterError::MissingCredential(name.to_string()))
        };
        Credentials::new(
            read(&self.api_key)?,
            read(&self.secret_key)?,
            read(&self.passphrase)?,
        )
        .map_err(|error| AdapterError::InvalidCredential(error.to_string()))
    }
}

impl fmt::Debug for CredentialEnvNames {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialEnvNames")
            .field("api_key", &self.api_key)
            .field("secret_key", &self.secret_key)
            .field("passphrase", &self.passphrase)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct ConnectionSettings {
    rest_url: String,
    demo_trading: bool,
    connect_timeout: Duration,
    request_timeout: Duration,
}

impl ConnectionSettings {
    pub fn new(
        rest_url: impl Into<String>,
        demo_trading: bool,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, AdapterError> {
        validate_official_origin(&rest_url.into(), demo_trading).and_then(|rest_url| {
            Self::from_validated_origin(rest_url, demo_trading, connect_timeout, request_timeout)
        })
    }

    /// Constructs settings for a deliberately local fault-injection server.
    /// This is the only public constructor that accepts a loopback origin.
    pub fn for_loopback_fault_test(
        rest_url: impl Into<String>,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, AdapterError> {
        validate_loopback_fault_test_origin(&rest_url.into()).and_then(|rest_url| {
            Self::from_validated_origin(rest_url, true, connect_timeout, request_timeout)
        })
    }

    fn from_validated_origin(
        rest_url: String,
        demo_trading: bool,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, AdapterError> {
        if connect_timeout.is_zero() || request_timeout.is_zero() {
            return Err(AdapterError::InvalidConfiguration(
                "REST timeouts must be positive".to_string(),
            ));
        }
        if request_timeout < connect_timeout {
            return Err(AdapterError::InvalidConfiguration(
                "REST request timeout must not be shorter than connect timeout".to_string(),
            ));
        }
        Ok(Self {
            rest_url,
            demo_trading,
            connect_timeout,
            request_timeout,
        })
    }
}

fn validate_official_origin(rest_url: &str, demo_trading: bool) -> Result<String, AdapterError> {
    const DEMO_HOSTS: &[&str] = &[
        "openapi.okx.com",
        "www.okx.com",
        "us.okx.com",
        "eea.okx.com",
    ];
    const PRODUCTION_HOSTS: &[&str] = &[
        "openapi.okx.com",
        "www.okx.com",
        "us.okx.com",
        "eea.okx.com",
        "tr.okx.com",
    ];
    let url = parse_exact_origin(rest_url)?;
    let host = url.host_str().expect("parse_exact_origin requires a host");
    let allowed = if demo_trading {
        DEMO_HOSTS
    } else {
        PRODUCTION_HOSTS
    };
    if url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || is_loopback_host(host)
        || !allowed.contains(&host.to_ascii_lowercase().as_str())
    {
        return Err(AdapterError::InvalidConfiguration(
            "REST URL must be an official HTTPS OKX origin on port 443".to_string(),
        ));
    }
    Ok(rest_url.to_string())
}

fn validate_loopback_fault_test_origin(rest_url: &str) -> Result<String, AdapterError> {
    let url = parse_exact_origin(rest_url)?;
    let host = url.host_str().expect("parse_exact_origin requires a host");
    if !matches!(url.scheme(), "http" | "https") || !is_loopback_host(host) {
        return Err(AdapterError::InvalidConfiguration(
            "fault-test REST URL must be an explicit loopback HTTP(S) origin".to_string(),
        ));
    }
    Ok(rest_url.to_string())
}

fn parse_exact_origin(rest_url: &str) -> Result<url::Url, AdapterError> {
    let url = url::Url::parse(rest_url).map_err(|error| {
        AdapterError::InvalidConfiguration(format!("REST URL is invalid: {error}"))
    })?;
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AdapterError::InvalidConfiguration(
            "REST URL must be an exact origin without user information, path, query, or fragment"
                .to_string(),
        ));
    }
    Ok(url)
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

#[async_trait]
trait RoleWire: Send + Sync {
    async fn public_get(&self, path: &str) -> Result<Response, reap_okx_wire::Error>;
    async fn get_at(&self, timestamp: &str, path: &str) -> Result<Response, reap_okx_wire::Error>;
    async fn post_at(
        &self,
        timestamp: &str,
        path: &str,
        body: &str,
    ) -> Result<Response, reap_okx_wire::Error>;
}

#[async_trait]
impl<T> RoleWire for Client<T>
where
    T: Transport + Send + Sync,
{
    async fn public_get(&self, path: &str) -> Result<Response, reap_okx_wire::Error> {
        self.public_get(path).await
    }

    async fn get_at(&self, timestamp: &str, path: &str) -> Result<Response, reap_okx_wire::Error> {
        Client::get_at(self, timestamp, path).await
    }

    async fn post_at(
        &self,
        timestamp: &str,
        path: &str,
        body: &str,
    ) -> Result<Response, reap_okx_wire::Error> {
        Client::post_at(self, timestamp, path, body).await
    }
}

/// Narrow account-wide cancellation role for one OKX account.
///
/// Construction reads credentials exactly once. The role exposes neither its
/// credentials nor its signer, wire client, transport, or arbitrary paths.
pub struct EmergencyAccountStop {
    wire: Arc<dyn RoleWire>,
}

/// Production factory used only by the separate emergency composition root.
#[derive(Debug, Clone, Copy, Default)]
pub struct OkxEmergencyAccountStopFactory;

impl EmergencyAccountStop {
    pub fn from_env(
        settings: ConnectionSettings,
        credentials: CredentialEnvNames,
    ) -> Result<Self, AdapterError> {
        // Preserve current setup precedence: credential failures are reported
        // before REST transport construction is attempted.
        let credentials = credentials.read()?;
        Self::from_credentials(settings, credentials)
    }

    fn from_credentials(
        settings: ConnectionSettings,
        credentials: Credentials,
    ) -> Result<Self, AdapterError> {
        let transport = ReqwestTransport::with_timeouts(
            settings.rest_url,
            settings.connect_timeout,
            settings.request_timeout,
        )
        .map_err(|error| match error {
            WireError::Transport(message) => {
                AdapterError::Wire(format!("HTTP transport failed: {message}"))
            }
            error => AdapterError::Wire(error.to_string()),
        })?;
        let wire = Client::new(transport, credentials, settings.demo_trading);
        Ok(Self {
            wire: Arc::new(wire),
        })
    }

    async fn server_time_ms(&self) -> Result<u64, RestError> {
        let body = public_get_body(&self.wire, PUBLIC_TIME_PATH).await?;
        parse_okx_server_time_response_json(&body)
    }

    async fn account_identity_at(
        &self,
        timestamp: &str,
    ) -> Result<core::EmergencyAccountIdentity, RestError> {
        let body = get_body_at(&self.wire, timestamp, ACCOUNT_CONFIG_PATH).await?;
        let config = parse_okx_account_config_response_json(&body)?;
        Ok(core::EmergencyAccountIdentity {
            user_id: config.user_id,
            main_user_id: config.main_user_id,
        })
    }

    async fn regular_pending_orders_page_at(
        &self,
        timestamp: &str,
        after: Option<&str>,
    ) -> Result<OkxRegularOrderPage, RestError> {
        let path = query_path(
            REGULAR_PENDING_PATH,
            [("after", after), ("limit", Some("100"))],
        );
        let body = get_body_at(&self.wire, timestamp, &path).await?;
        parse_okx_regular_order_page_response_json(&body)
    }

    async fn algo_pending_orders_page_at(
        &self,
        timestamp: &str,
        query: OkxAlgoOrderQuery,
        after: Option<&str>,
    ) -> Result<OkxAlgoOrderPage, RestError> {
        let path = query_path(
            ALGO_PENDING_PATH,
            [
                ("ordType", Some(query.as_str())),
                ("after", after),
                ("limit", Some("100")),
            ],
        );
        let body = get_body_at(&self.wire, timestamp, &path).await?;
        parse_okx_algo_order_page_response_json(&body)
    }

    async fn spread_pending_orders_page_at(
        &self,
        timestamp: &str,
        end_id: Option<&str>,
    ) -> Result<OkxSpreadOrderPage, RestError> {
        let path = query_path(
            SPREAD_PENDING_PATH,
            [("endId", end_id), ("limit", Some("100"))],
        );
        let body = get_body_at(&self.wire, timestamp, &path).await?;
        parse_okx_spread_order_page_response_json(&body)
    }

    async fn cancel_all_after_at(
        &self,
        timestamp: &str,
        timeout_secs: u64,
    ) -> Result<(), RestError> {
        let body = serialize_deadman_timeout(timeout_secs)?;
        let response =
            post_body_at(&self.wire, timestamp, REGULAR_CANCEL_ALL_AFTER_PATH, &body).await?;
        parse_okx_cancel_all_after_response_json(&response, timeout_secs)
    }

    async fn spread_cancel_all_after_at(
        &self,
        timestamp: &str,
        timeout_secs: u64,
    ) -> Result<(), RestError> {
        let body = serialize_deadman_timeout(timeout_secs)?;
        let response =
            post_body_at(&self.wire, timestamp, SPREAD_CANCEL_ALL_AFTER_PATH, &body).await?;
        parse_spread_cancel_all_after_response(&response, timeout_secs)
    }

    async fn cancel_batch_orders_at(
        &self,
        timestamp: &str,
        orders: &[OkxCancelOrder],
    ) -> Result<Vec<OkxCancelOrderResult>, RestError> {
        let body = serialize_regular_cancel_batch(orders)?;
        let response =
            post_body_at(&self.wire, timestamp, REGULAR_CANCEL_BATCH_PATH, &body).await?;
        parse_okx_cancel_order_results_response_json(&response)
    }

    async fn cancel_algo_orders_at(
        &self,
        timestamp: &str,
        orders: &[OkxCancelAlgoOrder],
    ) -> Result<Vec<OkxAlgoCancelResult>, RestError> {
        let body = serialize_algo_cancel_batch(orders)?;
        let response = post_body_at(&self.wire, timestamp, ALGO_CANCEL_PATH, &body).await?;
        parse_algo_cancel_response(&response)
    }

    async fn spread_mass_cancel_at(&self, timestamp: &str) -> Result<(), RestError> {
        let response = post_body_at(&self.wire, timestamp, SPREAD_MASS_CANCEL_PATH, "{}").await?;
        parse_spread_mass_cancel_response(&response)
    }

    #[cfg(test)]
    fn from_wire(wire: Arc<dyn RoleWire>) -> Self {
        Self { wire }
    }
}

impl fmt::Debug for EmergencyAccountStop {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmergencyAccountStop")
            .field("wire", &"[REDACTED]")
            .finish()
    }
}

fn role_error(error: RestError) -> core::EmergencyRoleError {
    core::EmergencyRoleError(error.to_string())
}

fn core_query(query: core::AlgoOrderQuery) -> OkxAlgoOrderQuery {
    match query {
        core::AlgoOrderQuery::ConditionalAndOco => OkxAlgoOrderQuery::ConditionalAndOco,
        core::AlgoOrderQuery::Chase => OkxAlgoOrderQuery::Chase,
        core::AlgoOrderQuery::Trigger => OkxAlgoOrderQuery::Trigger,
        core::AlgoOrderQuery::MoveOrderStop => OkxAlgoOrderQuery::MoveOrderStop,
        core::AlgoOrderQuery::Iceberg => OkxAlgoOrderQuery::Iceberg,
        core::AlgoOrderQuery::Twap => OkxAlgoOrderQuery::Twap,
        core::AlgoOrderQuery::SmartIceberg => OkxAlgoOrderQuery::SmartIceberg,
    }
}

#[async_trait]
impl core::EmergencyAccountStopRole for EmergencyAccountStop {
    async fn server_time_ms(&self) -> Result<u64, core::EmergencyRoleError> {
        EmergencyAccountStop::server_time_ms(self)
            .await
            .map_err(role_error)
    }

    async fn account_identity_at(
        &self,
        timestamp: &str,
    ) -> Result<core::EmergencyAccountIdentity, core::EmergencyRoleError> {
        EmergencyAccountStop::account_identity_at(self, timestamp)
            .await
            .map(|identity| core::EmergencyAccountIdentity {
                user_id: identity.user_id,
                main_user_id: identity.main_user_id,
            })
            .map_err(role_error)
    }

    async fn regular_pending_orders_page_at(
        &self,
        timestamp: &str,
        after: Option<&str>,
    ) -> Result<core::RegularOrderPage, core::EmergencyRoleError> {
        EmergencyAccountStop::regular_pending_orders_page_at(self, timestamp, after)
            .await
            .map(|page| core::RegularOrderPage {
                orders: page
                    .orders
                    .into_iter()
                    .map(|order| core::RegularOrder {
                        symbol: order.symbol,
                        exchange_order_id: order.exchange_order_id,
                        client_order_id: order.client_order_id,
                    })
                    .collect(),
                next_after: page.next_after,
            })
            .map_err(role_error)
    }

    async fn algo_pending_orders_page_at(
        &self,
        timestamp: &str,
        query: core::AlgoOrderQuery,
        after: Option<&str>,
    ) -> Result<core::AlgoOrderPage, core::EmergencyRoleError> {
        EmergencyAccountStop::algo_pending_orders_page_at(self, timestamp, core_query(query), after)
            .await
            .map(|page| core::AlgoOrderPage {
                orders: page
                    .orders
                    .into_iter()
                    .map(|order| core::AlgoOrder {
                        algo_id: order.algo_id,
                        client_order_id: order.client_order_id,
                        symbol: order.symbol,
                    })
                    .collect(),
                next_after: page.next_after,
            })
            .map_err(role_error)
    }

    async fn spread_pending_orders_page_at(
        &self,
        timestamp: &str,
        end_id: Option<&str>,
    ) -> Result<core::SpreadOrderPage, core::EmergencyRoleError> {
        EmergencyAccountStop::spread_pending_orders_page_at(self, timestamp, end_id)
            .await
            .map(|page| core::SpreadOrderPage {
                orders: page
                    .orders
                    .into_iter()
                    .map(|order| core::SpreadOrder {
                        spread_id: order.spread_id,
                        exchange_order_id: order.exchange_order_id,
                        client_order_id: order.client_order_id,
                    })
                    .collect(),
                next_end_id: page.next_end_id,
            })
            .map_err(role_error)
    }

    async fn cancel_all_after_at(
        &self,
        timestamp: &str,
        timeout_secs: u64,
    ) -> Result<(), core::EmergencyRoleError> {
        EmergencyAccountStop::cancel_all_after_at(self, timestamp, timeout_secs)
            .await
            .map_err(role_error)
    }

    async fn spread_cancel_all_after_at(
        &self,
        timestamp: &str,
        timeout_secs: u64,
    ) -> Result<(), core::EmergencyRoleError> {
        EmergencyAccountStop::spread_cancel_all_after_at(self, timestamp, timeout_secs)
            .await
            .map_err(role_error)
    }

    async fn cancel_batch_orders_at(
        &self,
        timestamp: &str,
        orders: &[core::CancelOrder],
    ) -> Result<Vec<core::CancelOrderResult>, core::EmergencyRoleError> {
        let orders = orders
            .iter()
            .map(|order| OkxCancelOrder {
                symbol: order.symbol.clone(),
                exchange_order_id: order.exchange_order_id.clone(),
                client_order_id: order.client_order_id.clone(),
            })
            .collect::<Vec<_>>();
        EmergencyAccountStop::cancel_batch_orders_at(self, timestamp, &orders)
            .await
            .map(|results| {
                results
                    .into_iter()
                    .map(|result| core::CancelOrderResult {
                        exchange_order_id: result.exchange_order_id,
                        client_order_id: result.client_order_id,
                        code: result.code,
                        message: result.message,
                    })
                    .collect()
            })
            .map_err(role_error)
    }

    async fn cancel_algo_orders_at(
        &self,
        timestamp: &str,
        orders: &[core::CancelAlgoOrder],
    ) -> Result<Vec<core::AlgoCancelResult>, core::EmergencyRoleError> {
        let orders = orders
            .iter()
            .map(|order| OkxCancelAlgoOrder {
                symbol: order.symbol.clone(),
                algo_id: order.algo_id.clone(),
            })
            .collect::<Vec<_>>();
        EmergencyAccountStop::cancel_algo_orders_at(self, timestamp, &orders)
            .await
            .map(|results| {
                results
                    .into_iter()
                    .map(|result| core::AlgoCancelResult {
                        algo_id: result.algo_id,
                        client_order_id: result.client_order_id,
                        code: result.code,
                        message: result.message,
                    })
                    .collect()
            })
            .map_err(role_error)
    }

    async fn spread_mass_cancel_at(&self, timestamp: &str) -> Result<(), core::EmergencyRoleError> {
        EmergencyAccountStop::spread_mass_cancel_at(self, timestamp)
            .await
            .map_err(role_error)
    }
}

impl core::EmergencyAccountStopFactory for OkxEmergencyAccountStopFactory {
    fn create(
        &self,
        venue: &core::EmergencyVenueConfig,
        runtime: &core::EmergencyRuntimeConfig,
        account: &core::EmergencyAccountConfig,
    ) -> Result<Box<dyn core::EmergencyAccountStopRole>, core::EmergencyRoleSetupError> {
        let read = |name: &str| {
            let value = std::env::var(name).map_err(|_| {
                core::EmergencyRoleSetupError::Credential(format!(
                    "account {} is missing credential env {name}",
                    account.id
                ))
            })?;
            if value.is_empty() {
                Err(core::EmergencyRoleSetupError::Credential(format!(
                    "account {} has an empty credential env {name}",
                    account.id
                )))
            } else {
                Ok(value)
            }
        };
        let credentials = Credentials::new(
            read(&account.api_key_env)?,
            read(&account.secret_key_env)?,
            read(&account.passphrase_env)?,
        )
        .map_err(|error| core::EmergencyRoleSetupError::Credential(error.to_string()))?;
        let connect_timeout = Duration::from_millis(runtime.rest_connect_timeout_ms);
        let request_timeout = Duration::from_millis(runtime.rest_request_timeout_ms);
        let settings = if venue.environment.is_demo()
            && url::Url::parse(&venue.rest_url)
                .ok()
                .and_then(|url| url.host_str().map(is_loopback_host))
                .unwrap_or(false)
        {
            ConnectionSettings::for_loopback_fault_test(
                venue.rest_url.clone(),
                connect_timeout,
                request_timeout,
            )
        } else {
            ConnectionSettings::new(
                venue.rest_url.clone(),
                venue.environment.is_demo(),
                connect_timeout,
                request_timeout,
            )
        }
        .map_err(|error| core::EmergencyRoleSetupError::Transport(error.to_string()))?;
        EmergencyAccountStop::from_credentials(settings, credentials)
            .map(|role| Box::new(role) as Box<dyn core::EmergencyAccountStopRole>)
            .map_err(|error| match error {
                AdapterError::MissingCredential(_) | AdapterError::InvalidCredential(_) => {
                    core::EmergencyRoleSetupError::Credential(error.to_string())
                }
                AdapterError::InvalidConfiguration(_) | AdapterError::Wire(_) => {
                    core::EmergencyRoleSetupError::Transport(error.to_string())
                }
            })
    }
}

fn serialize_deadman_timeout(timeout_secs: u64) -> Result<String, RestError> {
    if !(10..=120).contains(&timeout_secs) {
        return Err(RestError::InvalidField {
            field: "timeOut",
            value: timeout_secs.to_string(),
            message: "must be between 10 and 120 seconds for emergency arming".to_string(),
        });
    }
    #[derive(Serialize)]
    struct Body {
        #[serde(rename = "timeOut")]
        timeout_secs: String,
    }
    Ok(serde_json::to_string(&Body {
        timeout_secs: timeout_secs.to_string(),
    })?)
}

fn serialize_regular_cancel_batch(orders: &[OkxCancelOrder]) -> Result<String, RestError> {
    if orders.is_empty() || orders.len() > REGULAR_CANCEL_BATCH_LIMIT {
        return Err(RestError::InvalidField {
            field: "orders",
            value: orders.len().to_string(),
            message: "cancel batch must contain 1-20 orders".to_string(),
        });
    }

    #[derive(Serialize)]
    struct Body<'a> {
        #[serde(rename = "instId")]
        symbol: &'a str,
        #[serde(rename = "ordId", skip_serializing_if = "Option::is_none")]
        exchange_order_id: Option<&'a str>,
        #[serde(rename = "clOrdId", skip_serializing_if = "Option::is_none")]
        client_order_id: Option<&'a str>,
    }

    let mut body = Vec::with_capacity(orders.len());
    for order in orders {
        if order.exchange_order_id.is_none() && order.client_order_id.is_none() {
            return Err(RestError::InvalidField {
                field: "ordId/clOrdId",
                value: String::new(),
                message: "one identifier is required for every cancel".to_string(),
            });
        }
        body.push(Body {
            symbol: &order.symbol,
            exchange_order_id: order.exchange_order_id.as_deref(),
            client_order_id: order.client_order_id.as_deref(),
        });
    }
    Ok(serde_json::to_string(&body)?)
}

fn serialize_algo_cancel_batch(orders: &[OkxCancelAlgoOrder]) -> Result<String, RestError> {
    if orders.is_empty() || orders.len() > OKX_ALGO_CANCEL_BATCH_LIMIT {
        return Err(RestError::InvalidField {
            field: "orders",
            value: orders.len().to_string(),
            message: format!(
                "algo cancel batch must contain 1-{OKX_ALGO_CANCEL_BATCH_LIMIT} orders"
            ),
        });
    }

    #[derive(Serialize)]
    struct Body<'a> {
        #[serde(rename = "instId")]
        symbol: &'a str,
        #[serde(rename = "algoId")]
        algo_id: &'a str,
    }

    let mut body = Vec::with_capacity(orders.len());
    for order in orders {
        validate_required_text("instId", &order.symbol)?;
        validate_required_text("algoId", &order.algo_id)?;
        body.push(Body {
            symbol: &order.symbol,
            algo_id: &order.algo_id,
        });
    }
    Ok(serde_json::to_string(&body)?)
}

#[derive(Debug, Deserialize)]
struct OkxResponse<T> {
    code: String,
    #[serde(rename = "msg")]
    message: String,
    data: Vec<T>,
}

fn decode_okx_response<T: DeserializeOwned>(body: &[u8]) -> Result<OkxResponse<T>, RestError> {
    let response: OkxResponse<T> = serde_json::from_slice(body)?;
    if response.code != "0" {
        return Err(RestError::Api {
            code: response.code,
            message: response.message,
        });
    }
    Ok(response)
}

#[derive(Debug, Deserialize)]
struct AlgoCancelWire {
    #[serde(rename = "algoId")]
    algo_id: String,
    #[serde(default, rename = "algoClOrdId")]
    client_order_id: String,
    #[serde(default, rename = "sCode")]
    code: String,
    #[serde(default, rename = "sMsg")]
    message: String,
}

fn parse_algo_cancel_response(body: &[u8]) -> Result<Vec<OkxAlgoCancelResult>, RestError> {
    let response: OkxResponse<AlgoCancelWire> = decode_okx_response(body)?;
    if response.data.is_empty() {
        return Err(RestError::EmptyData {
            operation: "cancel algo orders",
        });
    }
    response
        .data
        .into_iter()
        .map(|wire| {
            validate_required_text("algoId", &wire.algo_id)?;
            validate_optional_text("algoClOrdId", &wire.client_order_id)?;
            Ok(OkxAlgoCancelResult {
                algo_id: wire.algo_id,
                client_order_id: wire.client_order_id,
                code: wire.code,
                message: wire.message,
            })
        })
        .collect()
}

#[derive(Debug, Deserialize)]
struct SpreadMassCancelWire {
    result: bool,
}

fn parse_spread_mass_cancel_response(body: &[u8]) -> Result<(), RestError> {
    let mut response: OkxResponse<SpreadMassCancelWire> = decode_okx_response(body)?;
    if response.data.len() != 1 {
        return Err(RestError::InvalidField {
            field: "data",
            value: response.data.len().to_string(),
            message: "spread mass cancel must return exactly one result".to_string(),
        });
    }
    if !response.data.remove(0).result {
        return Err(RestError::InvalidField {
            field: "result",
            value: "false".to_string(),
            message: "spread mass cancel was not accepted".to_string(),
        });
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct SpreadCancelAllAfterWire {
    #[serde(rename = "triggerTime")]
    trigger_time: String,
}

fn parse_spread_cancel_all_after_response(body: &[u8], timeout_secs: u64) -> Result<(), RestError> {
    let mut response: OkxResponse<SpreadCancelAllAfterWire> = decode_okx_response(body)?;
    if response.data.len() != 1 {
        return Err(RestError::InvalidField {
            field: "data",
            value: response.data.len().to_string(),
            message: "spread Cancel All After must return exactly one acknowledgement".to_string(),
        });
    }
    let acknowledgement = response.data.remove(0);
    if timeout_secs != 0 && parse_u64("triggerTime", &acknowledgement.trigger_time)? == 0 {
        return Err(RestError::InvalidField {
            field: "triggerTime",
            value: acknowledgement.trigger_time,
            message: "must be nonzero when spread Cancel All After is armed".to_string(),
        });
    }
    Ok(())
}

fn parse_u64(field: &'static str, value: &str) -> Result<u64, RestError> {
    value
        .parse()
        .map_err(|error: std::num::ParseIntError| RestError::InvalidField {
            field,
            value: value.to_string(),
            message: error.to_string(),
        })
}

fn validate_required_text(field: &'static str, value: &str) -> Result<(), RestError> {
    if value.is_empty() || value.trim() != value {
        return Err(RestError::InvalidField {
            field,
            value: value.to_string(),
            message: "must be non-empty and contain no surrounding whitespace".to_string(),
        });
    }
    Ok(())
}

fn validate_optional_text(field: &'static str, value: &str) -> Result<(), RestError> {
    if value.trim() != value {
        return Err(RestError::InvalidField {
            field,
            value: value.to_string(),
            message: "must contain no surrounding whitespace".to_string(),
        });
    }
    Ok(())
}

async fn public_get_body(wire: &Arc<dyn RoleWire>, path: &str) -> Result<Vec<u8>, RestError> {
    response_body(wire.public_get(path).await)
}

async fn get_body_at(
    wire: &Arc<dyn RoleWire>,
    timestamp: &str,
    path: &str,
) -> Result<Vec<u8>, RestError> {
    response_body(wire.get_at(timestamp, path).await)
}

async fn post_body_at(
    wire: &Arc<dyn RoleWire>,
    timestamp: &str,
    path: &str,
    body: &str,
) -> Result<Vec<u8>, RestError> {
    response_body(wire.post_at(timestamp, path, body).await)
}

fn response_body(response: Result<Response, reap_okx_wire::Error>) -> Result<Vec<u8>, RestError> {
    let response = response.map_err(|error| match error {
        WireError::Transport(message) => RestError::Transport(message),
        error => RestError::Transport(error.to_string()),
    })?;
    if !response.is_success() {
        return Err(RestError::Transport(format!(
            "HTTP status {}: {}",
            response.status(),
            String::from_utf8_lossy(response.body())
        )));
    }
    Ok(response.into_body())
}

fn query_path<'a, I>(base: &str, fields: I) -> String
where
    I: IntoIterator<Item = (&'a str, Option<&'a str>)>,
{
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (name, value) in fields {
        if let Some(value) = value {
            serializer.append_pair(name, value);
        }
    }
    let query = serializer.finish();
    if query.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{query}")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use reap_okx_wire::Method;

    use super::*;

    const TIMESTAMP: &str = "2020-12-08T09:08:57.715Z";

    #[test]
    fn public_connection_settings_restrict_remote_origins_and_name_loopback_tests() {
        let connect = Duration::from_millis(10);
        let request = Duration::from_millis(20);

        assert!(
            ConnectionSettings::new("https://openapi.okx.com", false, connect, request).is_ok()
        );
        assert!(
            ConnectionSettings::new("https://attacker.example", false, connect, request).is_err()
        );
        assert!(ConnectionSettings::new("http://127.0.0.1:8123", true, connect, request).is_err());
        assert!(
            ConnectionSettings::for_loopback_fault_test("http://127.0.0.1:8123", connect, request,)
                .is_ok()
        );
        assert!(
            ConnectionSettings::for_loopback_fault_test(
                "https://attacker.example",
                connect,
                request,
            )
            .is_err()
        );
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RecordedRequest {
        method: Method,
        timestamp: Option<String>,
        path: String,
        body: String,
    }

    #[derive(Default)]
    struct FakeWire {
        requests: Mutex<Vec<RecordedRequest>>,
        responses: Mutex<VecDeque<Response>>,
    }

    #[async_trait]
    impl RoleWire for FakeWire {
        async fn public_get(&self, path: &str) -> Result<Response, reap_okx_wire::Error> {
            self.requests.lock().unwrap().push(RecordedRequest {
                method: Method::Get,
                timestamp: None,
                path: path.to_string(),
                body: String::new(),
            });
            Ok(self.responses.lock().unwrap().pop_front().unwrap())
        }

        async fn get_at(
            &self,
            timestamp: &str,
            path: &str,
        ) -> Result<Response, reap_okx_wire::Error> {
            self.requests.lock().unwrap().push(RecordedRequest {
                method: Method::Get,
                timestamp: Some(timestamp.to_string()),
                path: path.to_string(),
                body: String::new(),
            });
            Ok(self.responses.lock().unwrap().pop_front().unwrap())
        }

        async fn post_at(
            &self,
            timestamp: &str,
            path: &str,
            body: &str,
        ) -> Result<Response, reap_okx_wire::Error> {
            self.requests.lock().unwrap().push(RecordedRequest {
                method: Method::Post,
                timestamp: Some(timestamp.to_string()),
                path: path.to_string(),
                body: body.to_string(),
            });
            Ok(self.responses.lock().unwrap().pop_front().unwrap())
        }
    }

    fn response(body: &str) -> Response {
        Response::new(200, body.as_bytes().to_vec())
    }

    fn empty_response() -> Response {
        response(r#"{"code":"0","msg":"","data":[]}"#)
    }

    #[tokio::test]
    async fn emergency_role_emits_only_exact_allowlisted_requests() {
        let wire = Arc::new(FakeWire::default());
        wire.responses.lock().unwrap().extend([
            response(r#"{"code":"0","msg":"","data":[{"ts":"1607418537715"}]}"#),
            response(
                r#"{"code":"0","msg":"","data":[{"triggerTime":"1607418547715"}]}"#,
            ),
            response(
                r#"{"code":"0","msg":"","data":[{"triggerTime":"1607418547715"}]}"#,
            ),
            empty_response(),
            empty_response(),
            empty_response(),
            empty_response(),
            empty_response(),
            empty_response(),
            empty_response(),
            empty_response(),
            empty_response(),
            response(
                r#"{"code":"0","msg":"","data":[{"ordId":"regular-1","clOrdId":"","sCode":"0","sMsg":""},{"ordId":"","clOrdId":"client-2","sCode":"0","sMsg":""}]}"#,
            ),
            response(
                r#"{"code":"0","msg":"","data":[{"algoId":"algo-1","algoClOrdId":"","sCode":"0","sMsg":""}]}"#,
            ),
            response(r#"{"code":"0","msg":"","data":[{"result":true}]}"#),
            response(
                r#"{"code":"0","msg":"","data":[{"acctLv":"2","posMode":"net_mode","acctStpMode":"cancel_maker","uid":"7","mainUid":"6"}]}"#,
            ),
        ]);
        let role_wire: Arc<dyn RoleWire> = wire.clone();
        let role = EmergencyAccountStop::from_wire(role_wire);

        assert_eq!(role.server_time_ms().await.unwrap(), 1_607_418_537_715);
        role.cancel_all_after_at(TIMESTAMP, 10).await.unwrap();
        role.spread_cancel_all_after_at(TIMESTAMP, 10)
            .await
            .unwrap();
        role.regular_pending_orders_page_at(TIMESTAMP, None)
            .await
            .unwrap();
        for query in OkxAlgoOrderQuery::ALL {
            role.algo_pending_orders_page_at(TIMESTAMP, query, None)
                .await
                .unwrap();
        }
        role.spread_pending_orders_page_at(TIMESTAMP, None)
            .await
            .unwrap();
        role.cancel_batch_orders_at(
            TIMESTAMP,
            &[
                OkxCancelOrder {
                    symbol: "BTC-USDT".to_string(),
                    exchange_order_id: Some("regular-1".to_string()),
                    client_order_id: None,
                },
                OkxCancelOrder {
                    symbol: "ETH-USDT".to_string(),
                    exchange_order_id: None,
                    client_order_id: Some("client-2".to_string()),
                },
            ],
        )
        .await
        .unwrap();
        role.cancel_algo_orders_at(
            TIMESTAMP,
            &[OkxCancelAlgoOrder {
                symbol: "BTC-USDT".to_string(),
                algo_id: "algo-1".to_string(),
            }],
        )
        .await
        .unwrap();
        role.spread_mass_cancel_at(TIMESTAMP).await.unwrap();
        assert_eq!(
            role.account_identity_at(TIMESTAMP).await.unwrap(),
            core::EmergencyAccountIdentity {
                user_id: "7".to_string(),
                main_user_id: "6".to_string(),
            }
        );

        let algo_paths = [
            "conditional%2Coco",
            "chase",
            "trigger",
            "move_order_stop",
            "iceberg",
            "twap",
            "smart_iceberg",
        ]
        .into_iter()
        .map(|order_type| RecordedRequest {
            method: Method::Get,
            timestamp: Some(TIMESTAMP.to_string()),
            path: format!("/api/v5/trade/orders-algo-pending?ordType={order_type}&limit=100"),
            body: String::new(),
        });
        let expected = [
            RecordedRequest {
                method: Method::Get,
                timestamp: None,
                path: PUBLIC_TIME_PATH.to_string(),
                body: String::new(),
            },
            RecordedRequest {
                method: Method::Post,
                timestamp: Some(TIMESTAMP.to_string()),
                path: REGULAR_CANCEL_ALL_AFTER_PATH.to_string(),
                body: r#"{"timeOut":"10"}"#.to_string(),
            },
            RecordedRequest {
                method: Method::Post,
                timestamp: Some(TIMESTAMP.to_string()),
                path: SPREAD_CANCEL_ALL_AFTER_PATH.to_string(),
                body: r#"{"timeOut":"10"}"#.to_string(),
            },
            RecordedRequest {
                method: Method::Get,
                timestamp: Some(TIMESTAMP.to_string()),
                path: "/api/v5/trade/orders-pending?limit=100".to_string(),
                body: String::new(),
            },
        ]
        .into_iter()
        .chain(algo_paths)
        .chain([
            RecordedRequest {
                method: Method::Get,
                timestamp: Some(TIMESTAMP.to_string()),
                path: "/api/v5/sprd/orders-pending?limit=100".to_string(),
                body: String::new(),
            },
            RecordedRequest {
                method: Method::Post,
                timestamp: Some(TIMESTAMP.to_string()),
                path: REGULAR_CANCEL_BATCH_PATH.to_string(),
                body: concat!(
                    r#"[{"instId":"BTC-USDT","ordId":"regular-1"},"#,
                    r#"{"instId":"ETH-USDT","clOrdId":"client-2"}]"#
                )
                .to_string(),
            },
            RecordedRequest {
                method: Method::Post,
                timestamp: Some(TIMESTAMP.to_string()),
                path: ALGO_CANCEL_PATH.to_string(),
                body: r#"[{"instId":"BTC-USDT","algoId":"algo-1"}]"#.to_string(),
            },
            RecordedRequest {
                method: Method::Post,
                timestamp: Some(TIMESTAMP.to_string()),
                path: SPREAD_MASS_CANCEL_PATH.to_string(),
                body: "{}".to_string(),
            },
            RecordedRequest {
                method: Method::Get,
                timestamp: Some(TIMESTAMP.to_string()),
                path: ACCOUNT_CONFIG_PATH.to_string(),
                body: String::new(),
            },
        ])
        .collect::<Vec<_>>();

        assert_eq!(*wire.requests.lock().unwrap(), expected);
        assert!(wire.responses.lock().unwrap().is_empty());
    }

    #[test]
    fn allowlist_is_exact_and_contains_no_submit_or_account_administration() {
        assert_eq!(
            EMERGENCY_HTTP_ALLOWLIST,
            &[
                ("GET", "/api/v5/public/time"),
                ("GET", "/api/v5/account/config"),
                ("GET", "/api/v5/trade/orders-pending"),
                ("GET", "/api/v5/trade/orders-algo-pending"),
                ("GET", "/api/v5/sprd/orders-pending"),
                ("POST", "/api/v5/trade/cancel-all-after"),
                ("POST", "/api/v5/trade/cancel-batch-orders"),
                ("POST", "/api/v5/trade/cancel-algos"),
                ("POST", "/api/v5/sprd/mass-cancel"),
                ("POST", "/api/v5/sprd/cancel-all-after"),
            ]
        );
        let paths = EMERGENCY_HTTP_ALLOWLIST
            .iter()
            .map(|(_, path)| *path)
            .collect::<Vec<_>>();
        for forbidden in [
            "/api/v5/trade/order",
            "/api/v5/trade/order-algo",
            "/api/v5/sprd/order",
            "/api/v5/trade/amend-order",
            "/api/v5/account/balance",
            "/api/v5/asset/withdrawal",
            "/api/v5/asset/transfer",
        ] {
            assert!(!paths.contains(&forbidden));
        }
    }

    #[tokio::test]
    async fn pagination_cursors_preserve_exact_query_order_and_encoding() {
        let wire = Arc::new(FakeWire::default());
        wire.responses.lock().unwrap().extend([
            empty_response(),
            empty_response(),
            empty_response(),
        ]);
        let role_wire: Arc<dyn RoleWire> = wire.clone();
        let role = EmergencyAccountStop::from_wire(role_wire);

        role.regular_pending_orders_page_at(TIMESTAMP, Some("regular cursor"))
            .await
            .unwrap();
        role.algo_pending_orders_page_at(
            TIMESTAMP,
            OkxAlgoOrderQuery::ConditionalAndOco,
            Some("algo/9"),
        )
        .await
        .unwrap();
        role.spread_pending_orders_page_at(TIMESTAMP, Some("spread:9"))
            .await
            .unwrap();

        let paths = wire
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request.path.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            [
                "/api/v5/trade/orders-pending?after=regular+cursor&limit=100",
                concat!(
                    "/api/v5/trade/orders-algo-pending?",
                    "ordType=conditional%2Coco&after=algo%2F9&limit=100"
                ),
                "/api/v5/sprd/orders-pending?endId=spread%3A9&limit=100",
            ]
        );
    }

    #[tokio::test]
    async fn invalid_cancel_shapes_fail_before_the_wire() {
        let wire = Arc::new(FakeWire::default());
        let role_wire: Arc<dyn RoleWire> = wire.clone();
        let role = EmergencyAccountStop::from_wire(role_wire);

        for timeout_secs in [0, 9, 121] {
            assert!(matches!(
                role.cancel_all_after_at(TIMESTAMP, timeout_secs).await,
                Err(RestError::InvalidField {
                    field: "timeOut",
                    ..
                })
            ));
            assert!(matches!(
                role.spread_cancel_all_after_at(TIMESTAMP, timeout_secs)
                    .await,
                Err(RestError::InvalidField {
                    field: "timeOut",
                    ..
                })
            ));
        }
        assert!(matches!(
            role.cancel_batch_orders_at(TIMESTAMP, &[]).await,
            Err(RestError::InvalidField {
                field: "orders",
                ..
            })
        ));
        assert!(matches!(
            role.cancel_algo_orders_at(TIMESTAMP, &[]).await,
            Err(RestError::InvalidField {
                field: "orders",
                ..
            })
        ));
        assert!(wire.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn authority_debug_does_not_expose_wire_or_credential_values() {
        let wire: Arc<dyn RoleWire> = Arc::new(FakeWire::default());
        let role = EmergencyAccountStop::from_wire(wire);
        let debug = format!("{role:?}");
        assert!(!debug.contains("api-secret"));
        assert!(debug.contains("[REDACTED]"));
    }
}
