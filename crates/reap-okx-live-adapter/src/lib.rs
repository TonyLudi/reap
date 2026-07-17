//! Role-specific OKX authority for the Chaos live runtime.
//!
//! The exported handles are deliberately non-interchangeable. None exposes
//! credentials, signatures, a transport, an arbitrary path, or conversion to
//! the lower-level wire client.

use std::fmt;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reap_core::{AccountUpdate, Channel, SelfTradePrevention, Side, TimeInForce, Venue};
use reap_feed::{BootstrapFactory, ConnectionError};
use reap_okx_wire::{Client, Credentials, ReqwestTransport, Response, Transport};
use reap_order::{
    RegularExecution as RegularExecutionPort, RegularReconciliation as RegularReconciliationPort,
};
use reap_venue::okx::{
    OkxAccountBalanceSnapshot, OkxAccountConfig, OkxAccountPositionsSnapshot, OkxAlgoOrderPage,
    OkxAlgoOrderQuery, OkxCancelOrder, OkxFillPage, OkxInstrument, OkxInstrumentType, OkxOrderAck,
    OkxPlaceOrder, OkxRegularOrderPage, OkxSpreadOrderPage, OkxSystemStatus, OkxTradeFeeRate,
    OkxWsOrderProtocolError, RestError, parse_okx_account_balance_response_json,
    parse_okx_account_config_response_json, parse_okx_account_instruments_response_json,
    parse_okx_account_positions_response_json, parse_okx_algo_order_page_response_json,
    parse_okx_cancel_all_after_response_json, parse_okx_fill_page_response_json,
    parse_okx_order_ack_response_json, parse_okx_order_details_response_json,
    parse_okx_regular_order_page_response_json, parse_okx_server_time_response_json,
    parse_okx_spread_order_page_response_json, parse_okx_system_status_response_json,
    parse_okx_trade_fee_response_json,
};
use serde::Serialize;
use thiserror::Error;

const PUBLIC_TIME_PATH: &str = "/api/v5/public/time";
const SYSTEM_STATUS_PATH: &str = "/api/v5/system/status";
const REGULAR_PENDING_PATH: &str = "/api/v5/trade/orders-pending";
const FILLS_PATH: &str = "/api/v5/trade/fills";
const ORDER_DETAILS_PATH: &str = "/api/v5/trade/order";
const CANCEL_ORDER_PATH: &str = "/api/v5/trade/cancel-order";
const CANCEL_ALL_AFTER_PATH: &str = "/api/v5/trade/cancel-all-after";
const ACCOUNT_INSTRUMENTS_PATH: &str = "/api/v5/account/instruments";
const ACCOUNT_TRADE_FEE_PATH: &str = "/api/v5/account/trade-fee";
const ACCOUNT_CONFIG_PATH: &str = "/api/v5/account/config";
const ACCOUNT_BALANCE_PATH: &str = "/api/v5/account/balance";
const ACCOUNT_POSITIONS_PATH: &str = "/api/v5/account/positions";
const ALGO_PENDING_PATH: &str = "/api/v5/trade/orders-algo-pending";
const SPREAD_PENDING_PATH: &str = "/api/v5/sprd/orders-pending";

const OKX_DEMO_REST_HOSTS: &[&str] = &[
    "openapi.okx.com",
    "www.okx.com",
    "us.okx.com",
    "eea.okx.com",
];
const OKX_PRODUCTION_REST_HOSTS: &[&str] = &[
    "openapi.okx.com",
    "www.okx.com",
    "us.okx.com",
    "eea.okx.com",
    "tr.okx.com",
];

pub const LIVE_READINESS_HTTP_ALLOWLIST: &[(&str, &str)] = &[
    ("GET", PUBLIC_TIME_PATH),
    ("GET", SYSTEM_STATUS_PATH),
    ("GET", ACCOUNT_INSTRUMENTS_PATH),
    ("GET", ACCOUNT_TRADE_FEE_PATH),
    ("GET", ACCOUNT_CONFIG_PATH),
    ("GET", ACCOUNT_BALANCE_PATH),
    ("GET", ACCOUNT_POSITIONS_PATH),
];

pub const REGULAR_RECONCILIATION_HTTP_ALLOWLIST: &[(&str, &str)] = &[
    ("GET", PUBLIC_TIME_PATH),
    ("GET", REGULAR_PENDING_PATH),
    ("GET", FILLS_PATH),
    ("GET", ORDER_DETAILS_PATH),
    ("GET", ACCOUNT_BALANCE_PATH),
    ("GET", ACCOUNT_POSITIONS_PATH),
];

pub const FORBIDDEN_OBSERVER_HTTP_ALLOWLIST: &[(&str, &str)] =
    &[("GET", ALGO_PENDING_PATH), ("GET", SPREAD_PENDING_PATH)];

pub const REGULAR_EXECUTION_HTTP_ALLOWLIST: &[(&str, &str)] = &[("POST", CANCEL_ORDER_PATH)];
pub const LIVE_SAFETY_HTTP_ALLOWLIST: &[(&str, &str)] = &[("POST", CANCEL_ALL_AFTER_PATH)];

pub const LIVE_PRIVATE_STATE_CHANNEL_ALLOWLIST: &[&str] =
    &["account", "orders", "positions", "fills"];
pub const LIVE_ORDER_WEBSOCKET_OPERATION_ALLOWLIST: &[&str] = &["order", "cancel-order"];

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("missing or empty OKX credential environment variable {0}")]
    MissingCredential(String),
    #[error("invalid OKX live adapter configuration: {0}")]
    InvalidConfiguration(String),
    #[error("failed to construct OKX wire authority: {0}")]
    Wire(String),
}

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
        .map_err(|error| AdapterError::Wire(error.to_string()))
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
        if connect_timeout.is_zero() || request_timeout.is_zero() {
            return Err(AdapterError::InvalidConfiguration(
                "REST timeouts must be positive".to_string(),
            ));
        }
        let rest_url = rest_url.into();
        validate_rest_origin(&rest_url, demo_trading)?;
        Ok(Self {
            rest_url,
            demo_trading,
            connect_timeout,
            request_timeout,
        })
    }
}

fn validate_rest_origin(rest_url: &str, demo_trading: bool) -> Result<(), AdapterError> {
    let invalid = |message: String| AdapterError::InvalidConfiguration(message);
    let url = url::Url::parse(rest_url)
        .map_err(|error| invalid(format!("REST URL is invalid: {error}")))?;
    let host = url
        .host_str()
        .ok_or_else(|| invalid("REST URL must contain a host".to_string()))?;
    let loopback = is_loopback_host(host);
    let demo_loopback = demo_trading && loopback;

    if url.scheme() != "https" && !(demo_loopback && url.scheme() == "http") {
        return Err(invalid(
            "REST URL must use https (loopback http is demo-test only)".to_string(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid(
            "REST URL must not contain user information".to_string(),
        ));
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err(invalid(
            "REST URL must use exact path / without query or fragment".to_string(),
        ));
    }
    if !demo_loopback && url.port_or_known_default() != Some(443) {
        return Err(invalid("REST URL must use port 443".to_string()));
    }
    if loopback {
        if demo_trading {
            return Ok(());
        }
        return Err(invalid(
            "REST URL loopback origin is demo-test only".to_string(),
        ));
    }

    let allowed_hosts = if demo_trading {
        OKX_DEMO_REST_HOSTS
    } else {
        OKX_PRODUCTION_REST_HOSTS
    };
    if !allowed_hosts.contains(&host.to_ascii_lowercase().as_str()) {
        let environment = if demo_trading { "demo" } else { "production" };
        return Err(invalid(format!(
            "REST URL host is not a documented OKX {environment} origin"
        )));
    }
    Ok(())
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
    async fn get(&self, path: &str) -> Result<Response, reap_okx_wire::Error>;
    async fn post(&self, path: &str, body: &str) -> Result<Response, reap_okx_wire::Error>;
    fn websocket_login(&self) -> Result<String, reap_okx_wire::Error>;
}

#[async_trait]
impl<T> RoleWire for Client<T>
where
    T: Transport + Send + Sync,
{
    async fn public_get(&self, path: &str) -> Result<Response, reap_okx_wire::Error> {
        self.public_get(path).await
    }

    async fn get(&self, path: &str) -> Result<Response, reap_okx_wire::Error> {
        self.get(path).await
    }

    async fn post(&self, path: &str, body: &str) -> Result<Response, reap_okx_wire::Error> {
        self.post(path, body).await
    }

    fn websocket_login(&self) -> Result<String, reap_okx_wire::Error> {
        Client::websocket_login(self).map(|login| login.as_str().to_string())
    }
}

#[derive(Clone)]
pub struct LiveReadiness {
    wire: Arc<dyn RoleWire>,
}

#[derive(Clone)]
pub struct RegularExecution {
    wire: Arc<dyn RoleWire>,
}

#[derive(Clone)]
pub struct RegularReconciliation {
    wire: Arc<dyn RoleWire>,
}

#[derive(Clone)]
pub struct LiveSafety {
    wire: Arc<dyn RoleWire>,
}

#[derive(Clone)]
pub struct ForbiddenOrderObserver {
    wire: Arc<dyn RoleWire>,
}

#[derive(Clone)]
pub struct PrivateStateSessionFactory {
    wire: Arc<dyn RoleWire>,
    allow_fills: bool,
}

#[derive(Clone)]
pub struct RegularOrderSessionFactory {
    wire: Arc<dyn RoleWire>,
}

pub struct ObserveRoles {
    readiness: LiveReadiness,
    reconciliation: RegularReconciliation,
    forbidden: ForbiddenOrderObserver,
    private_state_sessions: PrivateStateSessionFactory,
}

pub struct DemoRoles {
    observe: ObserveRoles,
    execution: RegularExecution,
    safety: LiveSafety,
    regular_order_sessions: RegularOrderSessionFactory,
}

impl ObserveRoles {
    pub fn readiness(&self) -> LiveReadiness {
        self.readiness.clone()
    }

    pub fn reconciliation(&self) -> RegularReconciliation {
        self.reconciliation.clone()
    }

    pub fn forbidden_observer(&self) -> ForbiddenOrderObserver {
        self.forbidden.clone()
    }

    pub fn private_state_sessions(&self) -> PrivateStateSessionFactory {
        self.private_state_sessions.clone()
    }
}

impl DemoRoles {
    pub fn observe(&self) -> &ObserveRoles {
        &self.observe
    }

    pub fn execution(&self) -> RegularExecution {
        self.execution.clone()
    }

    pub fn safety(&self) -> LiveSafety {
        self.safety.clone()
    }

    pub fn regular_order_sessions(&self) -> RegularOrderSessionFactory {
        self.regular_order_sessions.clone()
    }
}

pub fn observe_from_env(
    settings: ConnectionSettings,
    credentials: CredentialEnvNames,
    allow_fills: bool,
) -> Result<ObserveRoles, AdapterError> {
    let wire = production_wire(settings, credentials)?;
    Ok(observe_roles(wire, allow_fills))
}

pub fn demo_from_env(
    settings: ConnectionSettings,
    credentials: CredentialEnvNames,
    allow_fills: bool,
) -> Result<DemoRoles, AdapterError> {
    if !settings.demo_trading {
        return Err(AdapterError::InvalidConfiguration(
            "regular mutation roles can only be constructed for demo trading".to_string(),
        ));
    }
    let wire = production_wire(settings, credentials)?;
    Ok(DemoRoles {
        observe: observe_roles(Arc::clone(&wire), allow_fills),
        execution: RegularExecution {
            wire: Arc::clone(&wire),
        },
        safety: LiveSafety {
            wire: Arc::clone(&wire),
        },
        regular_order_sessions: RegularOrderSessionFactory { wire },
    })
}

fn production_wire(
    settings: ConnectionSettings,
    credentials: CredentialEnvNames,
) -> Result<Arc<dyn RoleWire>, AdapterError> {
    let transport = ReqwestTransport::with_timeouts(
        settings.rest_url,
        settings.connect_timeout,
        settings.request_timeout,
    )
    .map_err(|error| AdapterError::Wire(error.to_string()))?;
    Ok(Arc::new(Client::new(
        transport,
        credentials.read()?,
        settings.demo_trading,
    )))
}

fn observe_roles(wire: Arc<dyn RoleWire>, allow_fills: bool) -> ObserveRoles {
    ObserveRoles {
        readiness: LiveReadiness {
            wire: Arc::clone(&wire),
        },
        reconciliation: RegularReconciliation {
            wire: Arc::clone(&wire),
        },
        forbidden: ForbiddenOrderObserver {
            wire: Arc::clone(&wire),
        },
        private_state_sessions: PrivateStateSessionFactory { wire, allow_fills },
    }
}

impl LiveReadiness {
    pub async fn server_time_ms(&self) -> Result<u64, RestError> {
        let body = public_get_body(&self.wire, PUBLIC_TIME_PATH).await?;
        parse_okx_server_time_response_json(&body)
    }

    pub async fn system_status(&self) -> Result<Vec<OkxSystemStatus>, RestError> {
        let body = public_get_body(&self.wire, SYSTEM_STATUS_PATH).await?;
        parse_okx_system_status_response_json(&body)
    }

    pub async fn account_config(&self) -> Result<OkxAccountConfig, RestError> {
        let body = get_body(&self.wire, ACCOUNT_CONFIG_PATH).await?;
        parse_okx_account_config_response_json(&body)
    }

    pub async fn account_balance_snapshot(&self) -> Result<OkxAccountBalanceSnapshot, RestError> {
        let body = get_body(&self.wire, ACCOUNT_BALANCE_PATH).await?;
        parse_okx_account_balance_response_json(&body)
    }

    pub async fn account_positions_snapshot(
        &self,
        instrument_type: Option<OkxInstrumentType>,
        symbol: Option<&str>,
    ) -> Result<OkxAccountPositionsSnapshot, RestError> {
        let path = query_path(
            ACCOUNT_POSITIONS_PATH,
            [
                ("instType", instrument_type.map(OkxInstrumentType::as_str)),
                ("instId", symbol),
            ],
        );
        let body = get_body(&self.wire, &path).await?;
        parse_okx_account_positions_response_json(&body)
    }

    pub async fn account_instrument(
        &self,
        instrument_type: OkxInstrumentType,
        symbol: &str,
    ) -> Result<OkxInstrument, RestError> {
        validate_text("instId", symbol)?;
        let path = query_path(
            ACCOUNT_INSTRUMENTS_PATH,
            [
                ("instType", Some(instrument_type.as_str())),
                ("instId", Some(symbol)),
            ],
        );
        let body = get_body(&self.wire, &path).await?;
        let mut instruments = parse_okx_account_instruments_response_json(&body)?;
        if instruments.len() != 1 {
            return Err(RestError::InvalidField {
                field: "data",
                value: instruments.len().to_string(),
                message: "exact account instrument response must contain one row".to_string(),
            });
        }
        let instrument = instruments.pop().expect("checked one row");
        if instrument.symbol != symbol || instrument.instrument_type != instrument_type {
            return Err(RestError::InvalidField {
                field: "instId/instType",
                value: format!(
                    "{}/{}",
                    instrument.symbol,
                    instrument.instrument_type.as_str()
                ),
                message: format!(
                    "expected exact {}/{} account instrument response",
                    symbol,
                    instrument_type.as_str()
                ),
            });
        }
        Ok(instrument)
    }

    pub async fn account_trade_fee(
        &self,
        instrument_type: OkxInstrumentType,
        instrument_id: Option<&str>,
        instrument_family: Option<&str>,
        group_id: &str,
    ) -> Result<OkxTradeFeeRate, RestError> {
        let instrument_id = instrument_id.filter(|value| !value.trim().is_empty());
        let instrument_family = instrument_family.filter(|value| !value.trim().is_empty());
        let selector_is_valid = match instrument_type {
            OkxInstrumentType::Spot | OkxInstrumentType::Margin => {
                instrument_id.is_some() && instrument_family.is_none()
            }
            OkxInstrumentType::Swap | OkxInstrumentType::Futures | OkxInstrumentType::Option => {
                instrument_id.is_none() && instrument_family.is_some()
            }
        };
        if !selector_is_valid {
            return Err(RestError::InvalidField {
                field: "instId/instFamily",
                value: format!("instId={instrument_id:?}, instFamily={instrument_family:?}"),
                message: "spot/margin requires instId; derivatives require instFamily".to_string(),
            });
        }
        validate_text("groupId", group_id)?;
        let path = query_path(
            ACCOUNT_TRADE_FEE_PATH,
            [
                ("instType", Some(instrument_type.as_str())),
                ("instId", instrument_id),
                ("instFamily", instrument_family),
            ],
        );
        let body = get_body(&self.wire, &path).await?;
        parse_okx_trade_fee_response_json(&body)?
            .into_iter()
            .find(|rate| {
                rate.instrument_type == instrument_type && rate.group_id == group_id.trim()
            })
            .ok_or_else(|| RestError::InvalidField {
                field: "feeGroup.groupId",
                value: group_id.to_string(),
                message: format!(
                    "trade fee response contained no matching {} group",
                    instrument_type.as_str()
                ),
            })
    }
}

#[async_trait]
impl RegularReconciliationPort for RegularReconciliation {
    async fn regular_pending_orders_page(
        &self,
        instrument_type: Option<&str>,
        symbol: Option<&str>,
        after: Option<&str>,
    ) -> Result<OkxRegularOrderPage, RestError> {
        let path = query_path(
            REGULAR_PENDING_PATH,
            [
                ("instType", instrument_type),
                ("instId", symbol),
                ("after", after),
                ("limit", Some("100")),
            ],
        );
        let body = get_body(&self.wire, &path).await?;
        parse_okx_regular_order_page_response_json(&body)
    }

    async fn recent_fills_page(
        &self,
        instrument_type: Option<&str>,
        symbol: Option<&str>,
        after: Option<&str>,
    ) -> Result<OkxFillPage, RestError> {
        let path = query_path(
            FILLS_PATH,
            [
                ("instType", instrument_type),
                ("instId", symbol),
                ("after", after),
                ("limit", Some("100")),
            ],
        );
        let body = get_body(&self.wire, &path).await?;
        parse_okx_fill_page_response_json(&body)
    }

    async fn account_balance(&self) -> Result<AccountUpdate, RestError> {
        let body = get_body(&self.wire, ACCOUNT_BALANCE_PATH).await?;
        Ok(parse_okx_account_balance_response_json(&body)?.account_update())
    }

    async fn account_positions(&self) -> Result<AccountUpdate, RestError> {
        let body = get_body(&self.wire, ACCOUNT_POSITIONS_PATH).await?;
        Ok(parse_okx_account_positions_response_json(&body)?.account_update())
    }

    async fn order_details(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> Result<reap_venue::RemoteOrder, RestError> {
        let path = query_path(
            ORDER_DETAILS_PATH,
            [
                ("instId", Some(symbol)),
                ("ordId", None),
                ("clOrdId", Some(client_order_id)),
            ],
        );
        let body = get_body(&self.wire, &path).await?;
        Ok(parse_okx_order_details_response_json(&body)?.order)
    }

    async fn server_time_ms(&self) -> Result<u64, RestError> {
        let body = public_get_body(&self.wire, PUBLIC_TIME_PATH).await?;
        parse_okx_server_time_response_json(&body)
    }
}

#[async_trait]
impl RegularExecutionPort for RegularExecution {
    async fn cancel_regular_order(&self, order: &OkxCancelOrder) -> Result<OkxOrderAck, RestError> {
        let body = serialize_cancel(order)?;
        let response = post_body(&self.wire, CANCEL_ORDER_PATH, &body).await?;
        parse_okx_order_ack_response_json(&response, "cancel order")
    }
}

impl LiveSafety {
    pub async fn cancel_all_after(&self, timeout_secs: u64) -> Result<(), RestError> {
        if timeout_secs != 0 && !(10..=120).contains(&timeout_secs) {
            return Err(RestError::InvalidField {
                field: "timeOut",
                value: timeout_secs.to_string(),
                message: "must be 0 or between 10 and 120 seconds".to_string(),
            });
        }
        #[derive(Serialize)]
        struct Body {
            #[serde(rename = "timeOut")]
            timeout_secs: String,
        }
        let body = serde_json::to_string(&Body {
            timeout_secs: timeout_secs.to_string(),
        })?;
        let response = post_body(&self.wire, CANCEL_ALL_AFTER_PATH, &body).await?;
        parse_okx_cancel_all_after_response_json(&response, timeout_secs)
    }
}

impl ForbiddenOrderObserver {
    pub async fn algo_pending_page(
        &self,
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
        let body = get_body(&self.wire, &path).await?;
        parse_okx_algo_order_page_response_json(&body)
    }

    pub async fn spread_pending_page(
        &self,
        end_id: Option<&str>,
    ) -> Result<OkxSpreadOrderPage, RestError> {
        let path = query_path(
            SPREAD_PENDING_PATH,
            [("endId", end_id), ("limit", Some("100"))],
        );
        let body = get_body(&self.wire, &path).await?;
        parse_okx_spread_order_page_response_json(&body)
    }
}

impl PrivateStateSessionFactory {
    pub fn bootstrap_factory(&self) -> BootstrapFactory {
        let wire = Arc::clone(&self.wire);
        let allow_fills = self.allow_fills;
        Arc::new(move |plan| {
            if !plan.private {
                return Ok(Vec::new());
            }
            if plan.venue != Venue::Okx {
                return Err(ConnectionError::LoginFailed(
                    "private OKX session received a non-OKX plan".to_string(),
                ));
            }
            for subscription in &plan.subscriptions {
                let allowed = match subscription.channel {
                    Channel::Account | Channel::Orders | Channel::Positions => true,
                    Channel::Fills => allow_fills,
                    _ => false,
                };
                if !allowed || subscription.symbol.is_some() {
                    return Err(ConnectionError::LoginFailed(format!(
                        "private OKX session rejected channel {:?}",
                        subscription.channel
                    )));
                }
            }
            wire.websocket_login()
                .map(|payload| vec![payload])
                .map_err(|error| ConnectionError::LoginFailed(error.to_string()))
        })
    }
}

impl RegularOrderSessionFactory {
    pub fn login_message(&self) -> Result<String, String> {
        self.wire
            .websocket_login()
            .map_err(|error| error.to_string())
    }

    pub fn place_request(
        &self,
        request_id: &str,
        expiry_ms: u64,
        order: &OkxPlaceOrder,
    ) -> Result<String, OkxWsOrderProtocolError> {
        build_ws_place_request(request_id, expiry_ms, order)
    }

    pub fn cancel_request(
        &self,
        request_id: &str,
        order: &OkxCancelOrder,
    ) -> Result<String, OkxWsOrderProtocolError> {
        build_ws_cancel_request(request_id, order)
    }
}

const MAX_WS_REQUEST_ID_BYTES: usize = 32;

fn build_ws_place_request(
    request_id: &str,
    expiry_ms: u64,
    order: &OkxPlaceOrder,
) -> Result<String, OkxWsOrderProtocolError> {
    validate_ws_request_id(request_id)?;
    let argument = serde_json::from_str(
        &serialize_place(order)
            .map_err(|error| OkxWsOrderProtocolError::InvalidOrder(error.to_string()))?,
    )?;
    serialize_ws_request(request_id, "order", Some(expiry_ms), argument)
}

fn build_ws_cancel_request(
    request_id: &str,
    order: &OkxCancelOrder,
) -> Result<String, OkxWsOrderProtocolError> {
    validate_ws_request_id(request_id)?;
    let argument = serde_json::from_str(
        &serialize_cancel(order)
            .map_err(|error| OkxWsOrderProtocolError::InvalidOrder(error.to_string()))?,
    )?;
    serialize_ws_request(request_id, "cancel-order", None, argument)
}

fn serialize_ws_request(
    request_id: &str,
    operation: &'static str,
    expiry_ms: Option<u64>,
    argument: serde_json::Value,
) -> Result<String, OkxWsOrderProtocolError> {
    #[derive(Serialize)]
    struct Request<'a> {
        id: &'a str,
        op: &'static str,
        #[serde(rename = "expTime", skip_serializing_if = "Option::is_none")]
        expiry_ms: Option<String>,
        args: [serde_json::Value; 1],
    }

    Ok(serde_json::to_string(&Request {
        id: request_id,
        op: operation,
        expiry_ms: expiry_ms.map(|value| value.to_string()),
        args: [argument],
    })?)
}

fn validate_ws_request_id(request_id: &str) -> Result<(), OkxWsOrderProtocolError> {
    if request_id.is_empty()
        || request_id.len() > MAX_WS_REQUEST_ID_BYTES
        || !request_id.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(OkxWsOrderProtocolError::InvalidRequestId(
            request_id.to_string(),
        ));
    }
    Ok(())
}

fn validate_text(field: &'static str, value: &str) -> Result<(), RestError> {
    if value.trim().is_empty() || value.trim() != value {
        return Err(RestError::InvalidField {
            field,
            value: value.to_string(),
            message: "must be non-empty and contain no surrounding whitespace".to_string(),
        });
    }
    Ok(())
}

fn serialize_cancel(order: &OkxCancelOrder) -> Result<String, RestError> {
    if order.exchange_order_id.is_none() && order.client_order_id.is_none() {
        return Err(RestError::InvalidField {
            field: "ordId/clOrdId",
            value: String::new(),
            message: "one identifier is required".to_string(),
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
    Ok(serde_json::to_string(&Body {
        symbol: &order.symbol,
        exchange_order_id: order.exchange_order_id.as_deref(),
        client_order_id: order.client_order_id.as_deref(),
    })?)
}

fn serialize_place(order: &OkxPlaceOrder) -> Result<String, RestError> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Body<'a> {
        #[serde(rename = "instId")]
        symbol: &'a str,
        #[serde(rename = "tdMode")]
        trade_mode: &'static str,
        side: &'static str,
        #[serde(rename = "ordType")]
        order_type: &'static str,
        px: String,
        sz: String,
        #[serde(rename = "clOrdId")]
        client_order_id: &'a str,
        #[serde(rename = "reduceOnly", skip_serializing_if = "Option::is_none")]
        reduce_only: Option<bool>,
        #[serde(rename = "stpMode", skip_serializing_if = "Option::is_none")]
        self_trade_prevention: Option<&'static str>,
    }

    let side = match order.side {
        Side::Buy => "buy",
        Side::Sell => "sell",
    };
    let order_type = match order.time_in_force {
        TimeInForce::Gtc => "limit",
        TimeInForce::Ioc => "ioc",
        TimeInForce::PostOnly => "post_only",
    };
    let trade_mode = match order.trade_mode {
        reap_venue::okx::OkxTradeMode::Cash => "cash",
        reap_venue::okx::OkxTradeMode::Cross => "cross",
        reap_venue::okx::OkxTradeMode::Isolated => "isolated",
    };
    let self_trade_prevention = order.self_trade_prevention.map(|mode| match mode {
        SelfTradePrevention::CancelMaker => "cancel_maker",
        SelfTradePrevention::CancelTaker => "cancel_taker",
        SelfTradePrevention::CancelBoth => "cancel_both",
    });
    Ok(serde_json::to_string(&Body {
        symbol: &order.symbol,
        trade_mode,
        side,
        order_type,
        px: order.price.to_string(),
        sz: order.qty.to_string(),
        client_order_id: &order.client_order_id,
        reduce_only: order.reduce_only.then_some(true),
        self_trade_prevention,
    })?)
}

async fn public_get_body(wire: &Arc<dyn RoleWire>, path: &str) -> Result<Vec<u8>, RestError> {
    response_body(wire.public_get(path).await)
}

async fn get_body(wire: &Arc<dyn RoleWire>, path: &str) -> Result<Vec<u8>, RestError> {
    response_body(wire.get(path).await)
}

async fn post_body(wire: &Arc<dyn RoleWire>, path: &str, body: &str) -> Result<Vec<u8>, RestError> {
    response_body(wire.post(path, body).await)
}

fn response_body(response: Result<Response, reap_okx_wire::Error>) -> Result<Vec<u8>, RestError> {
    let response = response.map_err(|error| RestError::Transport(error.to_string()))?;
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
        if let Some(value) = value.filter(|value| !value.is_empty()) {
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

    use reap_core::{ConnId, FeedPriority, SelfTradePrevention, Side, Subscription, TimeInForce};
    use reap_feed::SocketPlan;
    use reap_venue::okx::OkxTradeMode;

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        PublicGet(String),
        Get(String),
        Post(String, String),
        Login,
    }

    #[derive(Default)]
    struct FakeWire {
        calls: Mutex<Vec<Call>>,
        responses: Mutex<VecDeque<Response>>,
    }

    #[async_trait]
    impl RoleWire for FakeWire {
        async fn public_get(&self, path: &str) -> Result<Response, reap_okx_wire::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::PublicGet(path.to_string()));
            Ok(self.responses.lock().unwrap().pop_front().unwrap())
        }

        async fn get(&self, path: &str) -> Result<Response, reap_okx_wire::Error> {
            self.calls.lock().unwrap().push(Call::Get(path.to_string()));
            Ok(self.responses.lock().unwrap().pop_front().unwrap())
        }

        async fn post(&self, path: &str, body: &str) -> Result<Response, reap_okx_wire::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Post(path.to_string(), body.to_string()));
            Ok(self.responses.lock().unwrap().pop_front().unwrap())
        }

        fn websocket_login(&self) -> Result<String, reap_okx_wire::Error> {
            self.calls.lock().unwrap().push(Call::Login);
            Ok("login".to_string())
        }
    }

    fn response(body: &str) -> Response {
        Response::new(200, body.as_bytes().to_vec())
    }

    fn fake_wire(bodies: &[&str]) -> Arc<FakeWire> {
        let wire = Arc::new(FakeWire::default());
        wire.responses
            .lock()
            .unwrap()
            .extend(bodies.iter().map(|body| response(body)));
        wire
    }

    fn role_wire(wire: &Arc<FakeWire>) -> Arc<dyn RoleWire> {
        wire.clone()
    }

    fn assert_trace(wire: &FakeWire, expected: &[Call]) {
        assert_eq!(&*wire.calls.lock().unwrap(), expected);
        assert!(
            wire.responses.lock().unwrap().is_empty(),
            "test left mock responses unused"
        );
    }

    fn connection_settings(
        rest_url: &str,
        demo_trading: bool,
    ) -> Result<ConnectionSettings, AdapterError> {
        ConnectionSettings::new(
            rest_url,
            demo_trading,
            Duration::from_secs(1),
            Duration::from_secs(2),
        )
    }

    fn invalid_configuration(error: AdapterError) -> String {
        match error {
            AdapterError::InvalidConfiguration(message) => message,
            other => panic!("expected invalid configuration, got {other}"),
        }
    }

    #[test]
    fn connection_settings_accept_only_exact_official_and_demo_loopback_origins() {
        assert_eq!(
            OKX_DEMO_REST_HOSTS,
            &[
                "openapi.okx.com",
                "www.okx.com",
                "us.okx.com",
                "eea.okx.com",
            ]
        );
        assert_eq!(
            OKX_PRODUCTION_REST_HOSTS,
            &[
                "openapi.okx.com",
                "www.okx.com",
                "us.okx.com",
                "eea.okx.com",
                "tr.okx.com",
            ]
        );

        for origin in [
            "https://openapi.okx.com",
            "https://www.okx.com/",
            "https://us.okx.com:443",
            "https://eea.okx.com",
            "http://127.0.0.1:18080",
            "http://localhost:18080",
            "http://[::1]:18080",
            "https://127.0.0.1:18443",
        ] {
            let settings = connection_settings(origin, true).unwrap();
            assert_eq!(settings.rest_url, origin);
            assert!(settings.demo_trading);
        }

        for origin in [
            "https://openapi.okx.com",
            "https://www.okx.com",
            "https://us.okx.com",
            "https://eea.okx.com",
            "https://tr.okx.com",
        ] {
            let settings = connection_settings(origin, false).unwrap();
            assert_eq!(settings.rest_url, origin);
            assert!(!settings.demo_trading);
        }
    }

    #[test]
    fn connection_settings_reject_arbitrary_or_malformed_rest_origins() {
        for (origin, demo_trading, expected) in [
            (
                "https://tr.okx.com",
                true,
                "REST URL host is not a documented OKX demo origin",
            ),
            (
                "https://credentials.example",
                true,
                "REST URL host is not a documented OKX demo origin",
            ),
            (
                "https://openapi.okx.com.evil.example",
                false,
                "REST URL host is not a documented OKX production origin",
            ),
            (
                "http://openapi.okx.com",
                true,
                "REST URL must use https (loopback http is demo-test only)",
            ),
            (
                "https://openapi.okx.com:8443",
                false,
                "REST URL must use port 443",
            ),
            (
                "https://user:secret@openapi.okx.com",
                true,
                "REST URL must not contain user information",
            ),
            (
                "https://openapi.okx.com/api/v5",
                true,
                "REST URL must use exact path / without query or fragment",
            ),
            (
                "https://openapi.okx.com?redirect=1",
                true,
                "REST URL must use exact path / without query or fragment",
            ),
            (
                "https://openapi.okx.com/#fragment",
                true,
                "REST URL must use exact path / without query or fragment",
            ),
            (
                "https://127.0.0.1:443",
                false,
                "REST URL loopback origin is demo-test only",
            ),
        ] {
            let error = connection_settings(origin, demo_trading).unwrap_err();
            assert_eq!(invalid_configuration(error), expected, "origin {origin}");
        }
    }

    #[tokio::test]
    async fn live_readiness_emits_every_exact_allowlisted_request() {
        let wire = fake_wire(&[
            r#"{"code":"0","msg":"","data":[{"ts":"42"}]}"#,
            r#"{"code":"0","msg":"","data":[]}"#,
            r#"{"code":"0","msg":"","data":[{"instId":"BTC-USDT","instType":"SPOT","instFamily":"","groupId":"1","baseCcy":"BTC","quoteCcy":"USDT","settleCcy":"","ctType":"","ctVal":"","ctValCcy":"","tickSz":"0.1","lotSz":"0.001","minSz":"0.001","maxLmtSz":"100","maxMktSz":"1000000","maxLmtAmt":"1000000","maxMktAmt":"1000000","state":"live","upcChg":[]}]}"#,
            r#"{"code":"0","msg":"","data":[{"feeGroup":[{"groupId":"1","maker":"-0.0008","taker":"-0.001"}],"instType":"SPOT","level":"Lv1","ts":"1763979985847"}]}"#,
            r#"{"code":"0","msg":"","data":[{"acctLv":"2","posMode":"net_mode","acctStpMode":"cancel_maker","uid":"7","mainUid":"6","label":"reap-demo","perm":"read_only,trade","ip":"203.0.113.5","enableSpotBorrow":false,"autoLoan":false,"spotBorrowAutoRepay":false}]}"#,
            r#"{"code":"0","msg":"","data":[{"uTime":"100","details":[]}]}"#,
            r#"{"code":"0","msg":"","data":[]}"#,
        ]);
        let readiness = LiveReadiness {
            wire: role_wire(&wire),
        };

        assert_eq!(readiness.server_time_ms().await.unwrap(), 42);
        readiness.system_status().await.unwrap();
        readiness
            .account_instrument(OkxInstrumentType::Spot, "BTC-USDT")
            .await
            .unwrap();
        readiness
            .account_trade_fee(OkxInstrumentType::Spot, Some("BTC-USDT"), None, "1")
            .await
            .unwrap();
        readiness.account_config().await.unwrap();
        readiness.account_balance_snapshot().await.unwrap();
        readiness
            .account_positions_snapshot(Some(OkxInstrumentType::Swap), Some("BTC-USDT-SWAP"))
            .await
            .unwrap();

        assert_trace(
            &wire,
            &[
                Call::PublicGet(PUBLIC_TIME_PATH.to_string()),
                Call::PublicGet(SYSTEM_STATUS_PATH.to_string()),
                Call::Get(format!(
                    "{ACCOUNT_INSTRUMENTS_PATH}?instType=SPOT&instId=BTC-USDT"
                )),
                Call::Get(format!(
                    "{ACCOUNT_TRADE_FEE_PATH}?instType=SPOT&instId=BTC-USDT"
                )),
                Call::Get(ACCOUNT_CONFIG_PATH.to_string()),
                Call::Get(ACCOUNT_BALANCE_PATH.to_string()),
                Call::Get(format!(
                    "{ACCOUNT_POSITIONS_PATH}?instType=SWAP&instId=BTC-USDT-SWAP"
                )),
            ],
        );
    }

    #[tokio::test]
    async fn regular_reconciliation_emits_every_exact_allowlisted_request() {
        let wire = fake_wire(&[
            r#"{"code":"0","msg":"","data":[{"ts":"43"}]}"#,
            r#"{"code":"0","msg":"","data":[]}"#,
            r#"{"code":"0","msg":"","data":[]}"#,
            r#"{"code":"0","msg":"","data":[{"ordId":"exchange-1","clOrdId":"client-1","instId":"BTC-USDT","side":"buy","state":"live","px":"100","sz":"1","accFillSz":"0","avgPx":"","uTime":"1000"}]}"#,
            r#"{"code":"0","msg":"","data":[{"uTime":"100","details":[]}] }"#,
            r#"{"code":"0","msg":"","data":[]}"#,
        ]);
        let reconciliation = RegularReconciliation {
            wire: role_wire(&wire),
        };

        assert_eq!(reconciliation.server_time_ms().await.unwrap(), 43);
        reconciliation
            .regular_pending_orders_page(Some("SPOT"), Some("BTC-USDT"), Some("regular-after"))
            .await
            .unwrap();
        reconciliation
            .recent_fills_page(Some("SPOT"), Some("BTC-USDT"), Some("fill-after"))
            .await
            .unwrap();
        reconciliation
            .order_details("BTC-USDT", "client-1")
            .await
            .unwrap();
        reconciliation.account_balance().await.unwrap();
        reconciliation.account_positions().await.unwrap();

        assert_trace(
            &wire,
            &[
                Call::PublicGet(PUBLIC_TIME_PATH.to_string()),
                Call::Get(format!(
                    "{REGULAR_PENDING_PATH}?instType=SPOT&instId=BTC-USDT&after=regular-after&limit=100"
                )),
                Call::Get(format!(
                    "{FILLS_PATH}?instType=SPOT&instId=BTC-USDT&after=fill-after&limit=100"
                )),
                Call::Get(format!(
                    "{ORDER_DETAILS_PATH}?instId=BTC-USDT&clOrdId=client-1"
                )),
                Call::Get(ACCOUNT_BALANCE_PATH.to_string()),
                Call::Get(ACCOUNT_POSITIONS_PATH.to_string()),
            ],
        );
    }

    #[tokio::test]
    async fn forbidden_observer_emits_all_seven_algo_queries_and_spread_query() {
        let wire = fake_wire(&[r#"{"code":"0","msg":"","data":[]}"#; 8]);
        let observer = ForbiddenOrderObserver {
            wire: role_wire(&wire),
        };

        for (index, query) in OkxAlgoOrderQuery::ALL.into_iter().enumerate() {
            let after = format!("algo-after-{index}");
            observer
                .algo_pending_page(query, Some(&after))
                .await
                .unwrap();
        }
        observer
            .spread_pending_page(Some("spread-after"))
            .await
            .unwrap();

        assert_trace(
            &wire,
            &[
                Call::Get(format!(
                    "{ALGO_PENDING_PATH}?ordType=conditional%2Coco&after=algo-after-0&limit=100"
                )),
                Call::Get(format!(
                    "{ALGO_PENDING_PATH}?ordType=chase&after=algo-after-1&limit=100"
                )),
                Call::Get(format!(
                    "{ALGO_PENDING_PATH}?ordType=trigger&after=algo-after-2&limit=100"
                )),
                Call::Get(format!(
                    "{ALGO_PENDING_PATH}?ordType=move_order_stop&after=algo-after-3&limit=100"
                )),
                Call::Get(format!(
                    "{ALGO_PENDING_PATH}?ordType=iceberg&after=algo-after-4&limit=100"
                )),
                Call::Get(format!(
                    "{ALGO_PENDING_PATH}?ordType=twap&after=algo-after-5&limit=100"
                )),
                Call::Get(format!(
                    "{ALGO_PENDING_PATH}?ordType=smart_iceberg&after=algo-after-6&limit=100"
                )),
                Call::Get(format!(
                    "{SPREAD_PENDING_PATH}?endId=spread-after&limit=100"
                )),
            ],
        );
    }

    #[tokio::test]
    async fn regular_execution_and_live_safety_emit_only_exact_mutations() {
        let wire = fake_wire(&[
            r#"{"code":"0","msg":"","data":[{"ordId":"exchange-1","clOrdId":"client-1","sCode":"0","sMsg":""}]}"#,
            r#"{"code":"0","msg":"","data":[{"triggerTime":"1"}]}"#,
        ]);
        let execution = RegularExecution {
            wire: role_wire(&wire),
        };
        let safety = LiveSafety {
            wire: role_wire(&wire),
        };

        execution
            .cancel_regular_order(&OkxCancelOrder {
                symbol: "BTC-USDT".to_string(),
                exchange_order_id: Some("exchange-1".to_string()),
                client_order_id: Some("client-1".to_string()),
            })
            .await
            .unwrap();
        safety.cancel_all_after(30).await.unwrap();

        assert_trace(
            &wire,
            &[
                Call::Post(
                    CANCEL_ORDER_PATH.to_string(),
                    r#"{"instId":"BTC-USDT","ordId":"exchange-1","clOrdId":"client-1"}"#
                        .to_string(),
                ),
                Call::Post(
                    CANCEL_ALL_AFTER_PATH.to_string(),
                    r#"{"timeOut":"30"}"#.to_string(),
                ),
            ],
        );
    }

    #[test]
    fn private_state_factory_accepts_only_planned_private_channels() {
        fn subscription(channel: Channel, symbol: Option<&str>) -> Subscription {
            Subscription {
                venue: Venue::Okx,
                channel,
                symbol: symbol.map(str::to_string),
                priority: FeedPriority::Critical,
                connections: 1,
            }
        }

        fn plan(private: bool, subscriptions: Vec<Subscription>) -> SocketPlan {
            SocketPlan {
                conn_id: ConnId::new("test-private-state"),
                venue: Venue::Okx,
                private,
                subscriptions,
            }
        }

        let wire = fake_wire(&[]);
        let without_fills = PrivateStateSessionFactory {
            wire: role_wire(&wire),
            allow_fills: false,
        }
        .bootstrap_factory();

        assert_eq!(
            without_fills(&plan(
                false,
                vec![subscription(Channel::Books, Some("BTC-USDT"))],
            ))
            .unwrap(),
            Vec::<String>::new()
        );
        for channel in [Channel::Account, Channel::Orders, Channel::Positions] {
            assert_eq!(
                without_fills(&plan(true, vec![subscription(channel, None)])).unwrap(),
                vec!["login".to_string()]
            );
        }
        for channel in [
            Channel::Fills,
            Channel::Books,
            Channel::Trades,
            Channel::Custom("orders-algo".to_string()),
        ] {
            assert!(matches!(
                without_fills(&plan(true, vec![subscription(channel, None)])),
                Err(ConnectionError::LoginFailed(_))
            ));
        }
        assert!(matches!(
            without_fills(&plan(
                true,
                vec![subscription(Channel::Orders, Some("BTC-USDT"))],
            )),
            Err(ConnectionError::LoginFailed(_))
        ));

        let with_fills = PrivateStateSessionFactory {
            wire: role_wire(&wire),
            allow_fills: true,
        }
        .bootstrap_factory();
        assert_eq!(
            with_fills(&plan(true, vec![subscription(Channel::Fills, None)],)).unwrap(),
            vec!["login".to_string()]
        );

        assert_trace(&wire, &[Call::Login, Call::Login, Call::Login, Call::Login]);
    }

    #[test]
    fn regular_order_session_factory_builds_exact_place_and_cancel_operations() {
        let wire = fake_wire(&[]);
        let factory = RegularOrderSessionFactory {
            wire: role_wire(&wire),
        };

        assert_eq!(factory.login_message().unwrap(), "login");
        let place = factory
            .place_request(
                "place1",
                123_456,
                &OkxPlaceOrder {
                    symbol: "BTC-USDT".to_string(),
                    trade_mode: OkxTradeMode::Cash,
                    side: Side::Buy,
                    time_in_force: TimeInForce::PostOnly,
                    price: 100.5,
                    qty: 0.25,
                    client_order_id: "client-1".to_string(),
                    reduce_only: false,
                    self_trade_prevention: Some(SelfTradePrevention::CancelMaker),
                },
            )
            .unwrap();
        let cancel = factory
            .cancel_request(
                "cancel1",
                &OkxCancelOrder {
                    symbol: "BTC-USDT".to_string(),
                    exchange_order_id: Some("exchange-1".to_string()),
                    client_order_id: Some("client-1".to_string()),
                },
            )
            .unwrap();

        assert_eq!(
            place,
            r#"{"id":"place1","op":"order","expTime":"123456","args":[{"clOrdId":"client-1","instId":"BTC-USDT","ordType":"post_only","px":"100.5","side":"buy","stpMode":"cancel_maker","sz":"0.25","tdMode":"cash"}]}"#
        );
        assert_eq!(
            cancel,
            r#"{"id":"cancel1","op":"cancel-order","args":[{"clOrdId":"client-1","instId":"BTC-USDT","ordId":"exchange-1"}]}"#
        );
        let operation_trace = [&place, &cancel].map(|request| {
            serde_json::from_str::<serde_json::Value>(request).unwrap()["op"].clone()
        });
        assert_eq!(operation_trace, ["order", "cancel-order"]);
        assert_trace(&wire, &[Call::Login]);
    }

    #[test]
    fn role_allowlists_are_exact_and_exclude_forbidden_mutations() {
        assert_eq!(
            LIVE_READINESS_HTTP_ALLOWLIST,
            &[
                ("GET", "/api/v5/public/time"),
                ("GET", "/api/v5/system/status"),
                ("GET", "/api/v5/account/instruments"),
                ("GET", "/api/v5/account/trade-fee"),
                ("GET", "/api/v5/account/config"),
                ("GET", "/api/v5/account/balance"),
                ("GET", "/api/v5/account/positions"),
            ]
        );
        assert_eq!(
            REGULAR_RECONCILIATION_HTTP_ALLOWLIST,
            &[
                ("GET", "/api/v5/public/time"),
                ("GET", "/api/v5/trade/orders-pending"),
                ("GET", "/api/v5/trade/fills"),
                ("GET", "/api/v5/trade/order"),
                ("GET", "/api/v5/account/balance"),
                ("GET", "/api/v5/account/positions"),
            ]
        );
        assert_eq!(
            FORBIDDEN_OBSERVER_HTTP_ALLOWLIST,
            &[
                ("GET", "/api/v5/trade/orders-algo-pending"),
                ("GET", "/api/v5/sprd/orders-pending"),
            ]
        );
        assert_eq!(
            REGULAR_EXECUTION_HTTP_ALLOWLIST,
            &[("POST", "/api/v5/trade/cancel-order")]
        );
        assert_eq!(
            LIVE_SAFETY_HTTP_ALLOWLIST,
            &[("POST", "/api/v5/trade/cancel-all-after")]
        );
        assert_eq!(
            LIVE_PRIVATE_STATE_CHANNEL_ALLOWLIST,
            &["account", "orders", "positions", "fills"]
        );
        assert_eq!(
            LIVE_ORDER_WEBSOCKET_OPERATION_ALLOWLIST,
            &["order", "cancel-order"]
        );

        let permissions = LIVE_READINESS_HTTP_ALLOWLIST
            .iter()
            .chain(REGULAR_RECONCILIATION_HTTP_ALLOWLIST)
            .chain(FORBIDDEN_OBSERVER_HTTP_ALLOWLIST)
            .chain(REGULAR_EXECUTION_HTTP_ALLOWLIST)
            .chain(LIVE_SAFETY_HTTP_ALLOWLIST)
            .copied()
            .collect::<Vec<_>>();
        for forbidden in [
            ("POST", "/api/v5/trade/order"),
            ("POST", "/api/v5/trade/order-algo"),
            ("POST", "/api/v5/trade/cancel-algos"),
            ("POST", "/api/v5/sprd/order"),
            ("POST", "/api/v5/sprd/cancel-order"),
            ("POST", "/api/v5/sprd/mass-cancel"),
            ("POST", "/api/v5/sprd/cancel-all-after"),
            ("POST", "/api/v5/trade/amend-order"),
        ] {
            assert!(!permissions.contains(&forbidden));
        }
    }
}
