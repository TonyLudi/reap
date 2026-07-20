//! Role-specific OKX authority for the Chaos live runtime.
//!
//! The exported handles are deliberately non-interchangeable. None exposes
//! credentials, signatures, a transport, an arbitrary path, or conversion to
//! the lower-level wire client.

mod order_ws;

use order_ws::OrderCommandWebsocketTransport;
pub use order_ws::{
    OrderCommandWebsocketConfig, OrderCommandWebsocketLifecycle, OrderCommandWebsocketStatus,
    OrderCommandWebsocketStatusKind,
};

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::net::IpAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use reap_core::{AccountUpdate, Channel, ConnId, SelfTradePrevention, Side, TimeInForce, Venue};
use reap_feed::{BootstrapFactory, ConnectionError, PrivateLoginBootstrap, SocketPlan};
use reap_okx_wire::{Client, Credentials, ReqwestTransport, Response, Transport};
use reap_order::{
    CancelOrderTransportError, GatewayError, OkxOrderGateway, OrderTransportError, PacingPolicy,
    PreparedRegularCancel, PreparedRegularSubmit, RegularApprovalScope,
    RegularExecution as RegularExecutionPort, RegularReconciliation as RegularReconciliationPort,
};
use reap_venue::okx::{
    OkxAccountBalanceSnapshot, OkxAccountConfig, OkxAccountPositionsSnapshot, OkxAlgoOrderPage,
    OkxAlgoOrderQuery, OkxCancelOrder, OkxExactDecimal, OkxFillPage, OkxInstrument,
    OkxInstrumentType, OkxOrderAck, OkxPlaceOrder, OkxRegularOrderPage, OkxSpreadOrderPage,
    OkxSystemStatus, OkxTradeFeeRate, OkxTradeMode, OkxWsOrderProtocolError, RestError,
    parse_okx_account_balance_response_json, parse_okx_account_config_response_json,
    parse_okx_account_instruments_response_json, parse_okx_account_positions_response_json,
    parse_okx_algo_order_page_response_json, parse_okx_cancel_all_after_response_json,
    parse_okx_fill_page_response_json, parse_okx_order_ack_response_json,
    parse_okx_order_details_response_json, parse_okx_regular_order_page_response_json,
    parse_okx_server_time_response_json, parse_okx_spread_order_page_response_json,
    parse_okx_system_status_response_json, parse_okx_trade_fee_response_json,
};
use serde::{Serialize, Serializer};
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
const OKX_DEMO_PRIVATE_WEBSOCKET_HOSTS: &[&str] =
    &["wspap.okx.com", "wsuspap.okx.com", "wseeapap.okx.com"];
const OKX_PRODUCTION_PRIVATE_WEBSOCKET_HOSTS: &[&str] =
    &["ws.okx.com", "wsus.okx.com", "wseea.okx.com"];

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
    #[error("failed to start OKX order command websocket authority: {0}")]
    OrderCommandWebsocket(String),
    #[error("failed to construct OKX regular order gateway: {0}")]
    OrderGateway(String),
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

fn validate_private_websocket_url(
    private_websocket_url: &str,
    demo_trading: bool,
) -> Result<(), AdapterError> {
    let invalid = |message: String| AdapterError::InvalidConfiguration(message);
    let url = url::Url::parse(private_websocket_url)
        .map_err(|error| invalid(format!("private websocket URL is invalid: {error}")))?;
    let host = url
        .host_str()
        .ok_or_else(|| invalid("private websocket URL must contain a host".to_string()))?;
    let loopback = is_loopback_host(host);
    let demo_loopback = demo_trading && loopback;

    if url.scheme() != "wss" && !(demo_loopback && url.scheme() == "ws") {
        return Err(invalid(
            "private websocket URL must use wss (loopback ws is demo-test only)".to_string(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid(
            "private websocket URL must not contain user information".to_string(),
        ));
    }
    if url.path() != "/ws/v5/private" || url.query().is_some() || url.fragment().is_some() {
        return Err(invalid(
            "private websocket URL must use exact path /ws/v5/private without query or fragment"
                .to_string(),
        ));
    }
    if !demo_loopback && url.port() != Some(8443) {
        return Err(invalid(
            "private websocket URL must use explicit port 8443".to_string(),
        ));
    }
    if loopback {
        if demo_trading {
            return Ok(());
        }
        return Err(invalid(
            "private websocket URL loopback destination is demo-test only".to_string(),
        ));
    }

    let allowed_hosts = if demo_trading {
        OKX_DEMO_PRIVATE_WEBSOCKET_HOSTS
    } else {
        OKX_PRODUCTION_PRIVATE_WEBSOCKET_HOSTS
    };
    if !allowed_hosts.contains(&host.to_ascii_lowercase().as_str()) {
        let environment = if demo_trading { "demo" } else { "production" };
        return Err(invalid(format!(
            "private websocket URL host is not a documented OKX {environment} destination"
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

struct RegularExecution {
    wire: Arc<dyn RoleWire>,
    expected_account_id: String,
    order_transport: Arc<OrderCommandTransportSlot>,
}

#[derive(Clone)]
pub struct RegularReconciliation {
    wire: Arc<dyn RoleWire>,
}

pub struct LiveSafety {
    wire: Arc<dyn RoleWire>,
}

#[derive(Clone)]
pub struct ForbiddenOrderObserver {
    wire: Arc<dyn RoleWire>,
}

pub struct PrivateStateSessionFactory {
    wire: Arc<dyn RoleWire>,
    allow_fills: bool,
    demo_trading: bool,
}

struct RegularOrderSessionFactory {
    wire: Arc<dyn RoleWire>,
    expected_account_id: String,
    demo_trading: bool,
}

#[derive(Default)]
struct OrderCommandTransportSlot {
    transport: OnceLock<OrderCommandWebsocketTransport>,
}

impl OrderCommandTransportSlot {
    fn install(&self, transport: OrderCommandWebsocketTransport) -> Result<(), AdapterError> {
        self.transport.set(transport).map_err(|_| {
            AdapterError::OrderCommandWebsocket(
                "regular order command transport was already installed".to_string(),
            )
        })
    }

    fn get(&self) -> Option<&OrderCommandWebsocketTransport> {
        self.transport.get()
    }
}

pub struct ObserveRoles {
    readiness: LiveReadiness,
    reconciliation: RegularReconciliation,
    forbidden: ForbiddenOrderObserver,
    private_state_sessions: Option<PrivateStateSessionFactory>,
}

pub struct DemoRoles {
    observe: ObserveRoles,
    regular_order_gateway: Option<RegularOrderGatewayRoles>,
    safety: Option<LiveSafety>,
}

struct RegularOrderGatewayRoles {
    execution: RegularExecution,
    order_sessions: RegularOrderSessionFactory,
    order_transport: Arc<OrderCommandTransportSlot>,
}

pub struct BoundRegularOrderGateway {
    gateway: OkxOrderGateway,
    order_sessions: RegularOrderSessionFactory,
    order_transport: Arc<OrderCommandTransportSlot>,
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

    pub fn take_private_state_sessions(&mut self) -> Option<PrivateStateSessionFactory> {
        self.private_state_sessions.take()
    }
}

impl DemoRoles {
    pub fn observe(&self) -> &ObserveRoles {
        &self.observe
    }

    pub fn take_bound_order_gateway(
        &mut self,
        trade_modes: HashMap<String, OkxTradeMode>,
        pacing: PacingPolicy,
    ) -> Result<BoundRegularOrderGateway, AdapterError> {
        let roles = self.regular_order_gateway.take().ok_or_else(|| {
            AdapterError::OrderGateway(
                "demo bound regular order gateway authority was already consumed".to_string(),
            )
        })?;
        let RegularOrderGatewayRoles {
            execution,
            order_sessions,
            order_transport,
        } = roles;
        let account_id = execution.expected_account_id.clone();
        let gateway = OkxOrderGateway::new(
            account_id,
            Box::new(execution),
            Arc::new(self.observe.reconciliation()),
            trade_modes,
            pacing,
        )
        .map_err(|error| AdapterError::OrderGateway(error.to_string()))?;
        Ok(BoundRegularOrderGateway {
            gateway,
            order_sessions,
            order_transport,
        })
    }

    pub fn take_safety(&mut self) -> Option<LiveSafety> {
        self.safety.take()
    }

    pub fn take_private_state_sessions(&mut self) -> Option<PrivateStateSessionFactory> {
        self.observe.take_private_state_sessions()
    }
}

impl BoundRegularOrderGateway {
    pub fn account_id(&self) -> &str {
        self.gateway.account_id()
    }

    pub fn take_approval_scope(&mut self) -> Result<RegularApprovalScope, GatewayError> {
        self.gateway.take_approval_scope()
    }
}

pub fn observe_from_env(
    settings: ConnectionSettings,
    credentials: CredentialEnvNames,
    allow_fills: bool,
) -> Result<ObserveRoles, AdapterError> {
    let demo_trading = settings.demo_trading;
    let wire = production_wire(settings, credentials)?;
    Ok(observe_roles(wire, allow_fills, demo_trading))
}

pub fn demo_from_env(
    settings: ConnectionSettings,
    credentials: CredentialEnvNames,
    expected_account_id: impl Into<String>,
    allow_fills: bool,
) -> Result<DemoRoles, AdapterError> {
    if !settings.demo_trading {
        return Err(AdapterError::InvalidConfiguration(
            "regular mutation roles can only be constructed for demo trading".to_string(),
        ));
    }
    let expected_account_id = expected_account_id.into();
    if expected_account_id.trim().is_empty() || expected_account_id.trim() != expected_account_id {
        return Err(AdapterError::InvalidConfiguration(
            "regular mutation role account id must be non-empty and trimmed".to_string(),
        ));
    }
    let demo_trading = settings.demo_trading;
    let wire = production_wire(settings, credentials)?;
    let order_transport = Arc::new(OrderCommandTransportSlot::default());
    let safety_wire = Arc::clone(&wire);
    Ok(DemoRoles {
        observe: observe_roles(Arc::clone(&wire), allow_fills, demo_trading),
        regular_order_gateway: Some(RegularOrderGatewayRoles {
            execution: RegularExecution {
                wire: Arc::clone(&wire),
                expected_account_id: expected_account_id.clone(),
                order_transport: Arc::clone(&order_transport),
            },
            order_sessions: RegularOrderSessionFactory {
                wire,
                expected_account_id,
                demo_trading,
            },
            order_transport,
        }),
        safety: Some(LiveSafety { wire: safety_wire }),
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

fn observe_roles(wire: Arc<dyn RoleWire>, allow_fills: bool, demo_trading: bool) -> ObserveRoles {
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
        private_state_sessions: Some(PrivateStateSessionFactory {
            wire,
            allow_fills,
            demo_trading,
        }),
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
    async fn cancel_regular_order(
        &self,
        cancel: PreparedRegularCancel,
    ) -> Result<OkxOrderAck, CancelOrderTransportError> {
        let Some(transport) = self.order_transport.get() else {
            return Err(CancelOrderTransportError::pre_send_unavailable(
                "regular order command websocket is not installed",
                cancel,
            ));
        };
        transport.cancel_order(cancel).await
    }

    async fn place_regular_order(
        &self,
        order: PreparedRegularSubmit,
    ) -> Result<OkxOrderAck, OrderTransportError> {
        let transport = self.order_transport.get().ok_or_else(|| {
            OrderTransportError::Unavailable(
                "regular order command websocket is not installed".to_string(),
            )
        })?;
        transport.place_order(order).await
    }

    async fn cancel_regular_order_via_rest(
        &self,
        cancel: PreparedRegularCancel,
    ) -> Result<OkxOrderAck, OrderTransportError> {
        ensure_rest_account(&self.expected_account_id, cancel.account_id())
            .map_err(|error| OrderTransportError::InvalidRequest(error.to_string()))?;
        let order = regular_cancel_order(&cancel);
        let body = serialize_cancel(&order)
            .map_err(|error| OrderTransportError::InvalidRequest(error.to_string()))?;
        let response = post_body(&self.wire, CANCEL_ORDER_PATH, &body)
            .await
            .map_err(|error| {
                OrderTransportError::Ambiguous(format!(
                    "REST cancel failed after request dispatch: {error}"
                ))
            })?;
        parse_regular_cancel_acknowledgement(&response)
    }
}

#[derive(serde::Deserialize)]
struct RegularCancelAckEnvelope {
    code: String,
    #[serde(rename = "msg")]
    message: String,
    data: Vec<serde_json::Value>,
}

fn parse_regular_cancel_acknowledgement(
    response: &[u8],
) -> Result<OkxOrderAck, OrderTransportError> {
    let envelope: RegularCancelAckEnvelope = serde_json::from_slice(response).map_err(|error| {
        OrderTransportError::Ambiguous(format!(
            "REST cancel acknowledgement was invalid after request dispatch: {error}"
        ))
    })?;
    if !is_okx_response_code(&envelope.code) {
        return Err(OrderTransportError::Ambiguous(format!(
            "REST cancel acknowledgement returned invalid top-level code {:?} after request dispatch",
            envelope.code
        )));
    }
    if envelope.code != "0" {
        return Err(OrderTransportError::Rejected {
            code: envelope.code,
            message: envelope.message,
        });
    }
    if envelope.data.len() != 1 {
        return Err(OrderTransportError::Ambiguous(format!(
            "REST cancel acknowledgement contained {} data rows; expected exactly one",
            envelope.data.len()
        )));
    }
    let row_code = envelope.data[0]
        .get("sCode")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            OrderTransportError::Ambiguous(
                "REST cancel acknowledgement row is missing string sCode after request dispatch"
                    .to_string(),
            )
        })?;
    if !is_okx_response_code(row_code) {
        return Err(OrderTransportError::Ambiguous(format!(
            "REST cancel acknowledgement row has invalid sCode {row_code:?} after request dispatch"
        )));
    }

    let acknowledgement = parse_okx_order_ack_response_json(response, "cancel order").map_err(
        |error| match error {
            RestError::Api { code, message } => OrderTransportError::Rejected { code, message },
            error => OrderTransportError::Ambiguous(format!(
                "REST cancel acknowledgement was invalid after request dispatch: {error}"
            )),
        },
    )?;
    if acknowledgement.exchange_order_id.trim().is_empty()
        || acknowledgement.exchange_order_id.trim() != acknowledgement.exchange_order_id
        || acknowledgement.exchange_order_id == "0"
    {
        return Err(OrderTransportError::Ambiguous(format!(
            "REST cancel acknowledgement returned invalid exchange order id {:?}",
            acknowledgement.exchange_order_id
        )));
    }
    Ok(acknowledgement)
}

fn is_okx_response_code(code: &str) -> bool {
    !code.is_empty() && code.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
async fn post_regular_cancel(
    wire: &Arc<dyn RoleWire>,
    order: &OkxCancelOrder,
) -> Result<OkxOrderAck, RestError> {
    let body = serialize_cancel(order)?;
    let response = post_body(wire, CANCEL_ORDER_PATH, &body).await?;
    parse_okx_order_ack_response_json(&response, "cancel order")
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
    pub fn bootstrap_factory(
        self,
        expected_account_id: impl Into<String>,
        expected_plan: SocketPlan,
        private_websocket_url: impl Into<String>,
    ) -> Result<BootstrapFactory, AdapterError> {
        let expected_account_id = expected_account_id.into();
        if expected_account_id.trim().is_empty()
            || expected_account_id.trim() != expected_account_id
        {
            return Err(AdapterError::InvalidConfiguration(
                "private state session account id must be non-empty and trimmed".to_string(),
            ));
        }
        let private_websocket_url = private_websocket_url.into();
        validate_private_websocket_url(&private_websocket_url, self.demo_trading)?;
        validate_private_state_plan(&expected_plan, self.allow_fills).map_err(|error| {
            AdapterError::InvalidConfiguration(format!(
                "invalid private state socket plan for account {expected_account_id}: {error}"
            ))
        })?;
        let expected_conn_id = ConnId::new(format!("okx-private-{expected_account_id}-r0"));
        if expected_plan.conn_id != expected_conn_id {
            return Err(AdapterError::InvalidConfiguration(format!(
                "private state socket plan for account {expected_account_id} must use connection id {expected_conn_id}, received {}",
                expected_plan.conn_id
            )));
        }
        let wire = Arc::clone(&self.wire);
        let bound_plan = expected_plan;
        Ok(BootstrapFactory::bind_private_websocket(
            private_websocket_url,
            move |plan| {
                if plan != &bound_plan {
                    return Err(ConnectionError::InvalidSubscriptionPlan(format!(
                        "private OKX session plan mismatch: expected {}, received {}",
                        bound_plan.conn_id, plan.conn_id
                    )));
                }
                let payload = wire
                    .websocket_login()
                    .map_err(|error| ConnectionError::LoginFailed(error.to_string()))?;
                PrivateLoginBootstrap::parse(payload)
            },
        ))
    }
}

fn validate_private_state_plan(
    plan: &reap_feed::SocketPlan,
    allow_fills: bool,
) -> Result<(), ConnectionError> {
    if !plan.private {
        return Err(ConnectionError::LoginFailed(
            "private OKX session received a public plan".to_string(),
        ));
    }
    if plan.venue != Venue::Okx {
        return Err(ConnectionError::LoginFailed(
            "private OKX session received a non-OKX plan".to_string(),
        ));
    }
    if plan.subscriptions.is_empty() {
        return Err(ConnectionError::LoginFailed(
            "private OKX session received an empty subscription plan".to_string(),
        ));
    }
    let mut seen_channels = HashSet::new();
    for subscription in &plan.subscriptions {
        if subscription.venue != Venue::Okx
            || subscription.priority != reap_core::FeedPriority::Critical
            || subscription.connections != 1
        {
            return Err(ConnectionError::LoginFailed(format!(
                "private OKX session rejected malformed channel {:?}",
                subscription.channel
            )));
        }
        let allowed = match subscription.channel {
            Channel::Account | Channel::Orders | Channel::Positions => true,
            Channel::Fills => allow_fills,
            _ => false,
        };
        if !allowed
            || subscription.symbol.is_some()
            || !seen_channels.insert(subscription.channel.clone())
        {
            return Err(ConnectionError::LoginFailed(format!(
                "private OKX session rejected channel {:?}",
                subscription.channel
            )));
        }
    }
    let reference_only = HashSet::from([Channel::Positions]);
    let mut executing = HashSet::from([Channel::Account, Channel::Orders, Channel::Positions]);
    if allow_fills {
        executing.insert(Channel::Fills);
    }
    if seen_channels != reference_only && seen_channels != executing {
        let expected = if allow_fills {
            "{Positions} or {Account, Orders, Positions, Fills}"
        } else {
            "{Positions} or {Account, Orders, Positions}"
        };
        return Err(ConnectionError::LoginFailed(format!(
            "private OKX session channel set must be exactly {expected}"
        )));
    }
    Ok(())
}

impl RegularOrderSessionFactory {
    fn login_message(&self) -> Result<String, String> {
        self.wire
            .websocket_login()
            .map_err(|error| error.to_string())
    }

    fn place_request(
        &self,
        request_id: &str,
        expiry_ms: u64,
        prepared: PreparedRegularSubmit,
    ) -> Result<String, OkxWsOrderProtocolError> {
        ensure_ws_account(&self.expected_account_id, prepared.account_id())?;
        build_ws_place_request(request_id, expiry_ms, &regular_place_order(&prepared))
    }

    fn cancel_request(
        &self,
        request_id: &str,
        prepared: &PreparedRegularCancel,
    ) -> Result<String, OkxWsOrderProtocolError> {
        ensure_ws_account(&self.expected_account_id, prepared.account_id())?;
        build_ws_cancel_request(request_id, &regular_cancel_order(prepared))
    }
}

fn ensure_rest_account(expected: &str, actual: &str) -> Result<(), RestError> {
    if actual == expected {
        return Ok(());
    }
    Err(RestError::InvalidField {
        field: "accountId",
        value: actual.to_string(),
        message: format!("regular order belongs to account {actual}, expected {expected}"),
    })
}

fn ensure_ws_account(expected: &str, actual: &str) -> Result<(), OkxWsOrderProtocolError> {
    if actual == expected {
        return Ok(());
    }
    Err(OkxWsOrderProtocolError::InvalidOrder(format!(
        "regular order belongs to account {actual}, expected {expected}"
    )))
}

fn regular_place_order(prepared: &PreparedRegularSubmit) -> OkxPlaceOrder {
    let order = prepared.order();
    OkxPlaceOrder {
        symbol: order.symbol.clone(),
        trade_mode: prepared.trade_mode(),
        side: order.side,
        time_in_force: order.time_in_force,
        price: *prepared.canonical_price(),
        qty: *prepared.canonical_qty(),
        client_order_id: prepared.client_order_id().to_string(),
        reduce_only: order.reduce_only,
        self_trade_prevention: order.self_trade_prevention,
    }
}

fn regular_cancel_order(prepared: &PreparedRegularCancel) -> OkxCancelOrder {
    OkxCancelOrder {
        symbol: prepared.symbol().to_string(),
        exchange_order_id: None,
        client_order_id: Some(prepared.client_order_id().to_string()),
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
    struct CanonicalDecimal<'a>(&'a OkxExactDecimal);

    impl Serialize for CanonicalDecimal<'_> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.collect_str(self.0)
        }
    }

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
        px: CanonicalDecimal<'a>,
        sz: CanonicalDecimal<'a>,
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
        px: CanonicalDecimal(&order.price),
        sz: CanonicalDecimal(&order.qty),
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

    use reap_core::{
        ConnId, FeedPriority, NormalizedEvent, SelfTradePrevention, Side, Subscription, TimeInForce,
    };
    use reap_feed::{
        ConnectionAttemptPacer, ConnectionStatusKind, OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS,
        ReconnectPolicy, SocketPlan, spawn_supervised_feed,
    };
    use reap_order::{
        OkxOrderGateway, OwnedRegularOrders, PacingPolicy, PrivateStateReducer,
        RegularExecutionPolicy, RegularExecutionProfile, SubmitPreparation,
    };
    use reap_risk::{InstrumentOrderLimits, InstrumentRiskModel};
    use reap_strategy::{
        ChaosConfig, ChaosStrategy, InstrumentConfig, InstrumentKindConfig, RiskGroupConfig,
    };
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
            Ok(r#"{"op":"login","args":[{"apiKey":"key","passphrase":"pass","timestamp":"1538054050","sign":"signature"}]}"#.to_string())
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

    fn order_session_factory(
        wire: &Arc<FakeWire>,
        account_id: &str,
        demo_trading: bool,
    ) -> RegularOrderSessionFactory {
        RegularOrderSessionFactory {
            wire: role_wire(wire),
            expected_account_id: account_id.to_string(),
            demo_trading,
        }
    }

    fn order_command_config(
        account_id: &str,
        websocket_url: &str,
        connection_attempt_pacer: ConnectionAttemptPacer,
    ) -> OrderCommandWebsocketConfig {
        OrderCommandWebsocketConfig::new(
            account_id,
            websocket_url,
            8,
            Duration::from_secs(1),
            Duration::from_secs(1),
            connection_attempt_pacer,
            ReconnectPolicy::default(),
        )
        .unwrap()
    }

    fn order_start_error(
        factory: RegularOrderSessionFactory,
        config: OrderCommandWebsocketConfig,
    ) -> AdapterError {
        factory
            .validate_start(&config)
            .expect_err("invalid order command websocket configuration was accepted")
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

    #[test]
    fn demo_roles_reject_empty_or_untrimmed_expected_account_ids_before_credentials_are_read() {
        let credentials = || CredentialEnvNames::new("KEY_ENV", "SECRET_ENV", "PASS_ENV").unwrap();
        for account_id in ["", " ", " main", "main ", "\tmain"] {
            let error = demo_from_env(
                connection_settings("http://127.0.0.1:18080", true).unwrap(),
                credentials(),
                account_id,
                false,
            )
            .err()
            .expect("invalid expected account id was accepted");
            assert!(
                invalid_configuration(error).contains("non-empty and trimmed"),
                "unexpected validation for {account_id:?}"
            );
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
            expected_account_id: "main".to_string(),
            order_transport: Arc::new(OrderCommandTransportSlot::default()),
        };
        let safety = LiveSafety {
            wire: role_wire(&wire),
        };

        post_regular_cancel(
            &execution.wire,
            &OkxCancelOrder {
                symbol: "BTC-USDT".to_string(),
                exchange_order_id: None,
                client_order_id: Some("client-1".to_string()),
            },
        )
        .await
        .unwrap();
        safety.cancel_all_after(30).await.unwrap();

        assert_trace(
            &wire,
            &[
                Call::Post(
                    CANCEL_ORDER_PATH.to_string(),
                    r#"{"instId":"BTC-USDT","clOrdId":"client-1"}"#.to_string(),
                ),
                Call::Post(
                    CANCEL_ALL_AFTER_PATH.to_string(),
                    r#"{"timeOut":"30"}"#.to_string(),
                ),
            ],
        );
    }

    #[test]
    fn rest_regular_cancel_requires_exactly_one_acknowledgement_row() {
        for (body, expected_rows) in [
            (r#"{"code":"0","msg":"","data":[]}"#, 0),
            (
                r#"{"code":"0","msg":"","data":[{"ordId":"exchange-1","clOrdId":"client-1","sCode":"0","sMsg":""},{"ordId":"exchange-2","clOrdId":"client-1","sCode":"0","sMsg":""}]}"#,
                2,
            ),
        ] {
            let error = parse_regular_cancel_acknowledgement(body.as_bytes()).unwrap_err();
            let OrderTransportError::Ambiguous(message) = error else {
                panic!("expected ambiguous acknowledgement, got {error}");
            };
            assert!(
                message.contains(&format!("contained {expected_rows} data rows")),
                "unexpected ambiguity message: {message}"
            );
        }
    }

    #[test]
    fn rest_regular_cancel_preserves_exchange_rejections() {
        assert_eq!(
            parse_regular_cancel_acknowledgement(
                br#"{"code":"51000","msg":"top-level rejection","data":[]}"#
            ),
            Err(OrderTransportError::Rejected {
                code: "51000".to_string(),
                message: "top-level rejection".to_string(),
            })
        );
        assert_eq!(
            parse_regular_cancel_acknowledgement(
                br#"{"code":"0","msg":"","data":[{"ordId":"","clOrdId":"client-1","sCode":"51603","sMsg":"order not found"}]}"#
            ),
            Err(OrderTransportError::Rejected {
                code: "51603".to_string(),
                message: "order not found".to_string(),
            })
        );
    }

    #[test]
    fn rest_regular_cancel_treats_malformed_or_invalid_acceptance_as_ambiguous() {
        for body in [
            "{not-json}",
            r#"{"code":"","msg":"","data":[]}"#,
            r#"{"code":"invalid","msg":"","data":[]}"#,
            r#"{"code":"0","msg":"","data":[42]}"#,
            r#"{"code":"0","msg":"","data":[{"ordId":"exchange-1","clOrdId":"client-1"}]}"#,
            r#"{"code":"0","msg":"","data":[{"ordId":"exchange-1","clOrdId":"client-1","sCode":"","sMsg":""}]}"#,
            r#"{"code":"0","msg":"","data":[{"ordId":"exchange-1","clOrdId":"client-1","sCode":"invalid","sMsg":""}]}"#,
            r#"{"code":"0","msg":"","data":[{"ordId":"exchange-1","clOrdId":"client-1","sCode":0,"sMsg":""}]}"#,
            r#"{"code":"0","msg":"","data":[{"ordId":"","clOrdId":"client-1","sCode":"0","sMsg":""}]}"#,
            r#"{"code":"0","msg":"","data":[{"ordId":"0","clOrdId":"client-1","sCode":"0","sMsg":""}]}"#,
            r#"{"code":"0","msg":"","data":[{"ordId":" exchange-1","clOrdId":"client-1","sCode":"0","sMsg":""}]}"#,
            r#"{"code":"0","msg":"","data":[{"ordId":"exchange-1 ","clOrdId":"client-1","sCode":"0","sMsg":""}]}"#,
        ] {
            assert!(
                matches!(
                    parse_regular_cancel_acknowledgement(body.as_bytes()),
                    Err(OrderTransportError::Ambiguous(_))
                ),
                "invalid REST cancel acknowledgement was not ambiguous: {body}"
            );
        }

        assert_eq!(
            parse_regular_cancel_acknowledgement(
                br#"{"code":"0","msg":"","data":[{"ordId":"exchange-1","clOrdId":"client-1","sCode":"0","sMsg":""}]}"#
            ),
            Ok(OkxOrderAck {
                exchange_order_id: "exchange-1".to_string(),
                client_order_id: "client-1".to_string(),
            })
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

        let candidate_channels = [
            Channel::Account,
            Channel::Orders,
            Channel::Positions,
            Channel::Fills,
        ];
        for allow_fills in [false, true] {
            for mask in 0_u8..(1 << candidate_channels.len()) {
                let subscriptions = candidate_channels
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, channel)| subscription(channel.clone(), None))
                    .collect();
                let accepted =
                    validate_private_state_plan(&plan(true, subscriptions), allow_fills).is_ok();
                let reference_only = mask == 0b0100;
                let executing = mask == if allow_fills { 0b1111 } else { 0b0111 };
                assert_eq!(
                    accepted,
                    reference_only || executing,
                    "unexpected private channel-set validation for mask {mask:04b}, allow_fills={allow_fills}"
                );
            }
        }

        assert!(matches!(
            validate_private_state_plan(
                &plan(false, vec![subscription(Channel::Books, Some("BTC-USDT"))],),
                false,
            ),
            Err(ConnectionError::LoginFailed(_))
        ));
        let executing = || {
            vec![
                subscription(Channel::Account, None),
                subscription(Channel::Orders, None),
                subscription(Channel::Positions, None),
            ]
        };
        validate_private_state_plan(
            &plan(true, vec![subscription(Channel::Positions, None)]),
            false,
        )
        .unwrap();
        validate_private_state_plan(&plan(true, executing()), false).unwrap();
        for channel in [Channel::Account, Channel::Orders] {
            assert!(matches!(
                validate_private_state_plan(&plan(true, vec![subscription(channel, None)]), false),
                Err(ConnectionError::LoginFailed(_))
            ));
        }
        for channel in [
            Channel::Fills,
            Channel::Books,
            Channel::Trades,
            Channel::Custom("orders-algo".to_string()),
        ] {
            assert!(matches!(
                validate_private_state_plan(&plan(true, vec![subscription(channel, None)]), false),
                Err(ConnectionError::LoginFailed(_))
            ));
        }
        assert!(matches!(
            validate_private_state_plan(
                &plan(true, vec![subscription(Channel::Orders, Some("BTC-USDT"))]),
                false,
            ),
            Err(ConnectionError::LoginFailed(_))
        ));
        let mut executing_with_fills = executing();
        executing_with_fills.push(subscription(Channel::Fills, None));
        validate_private_state_plan(&plan(true, executing_with_fills), true).unwrap();
        validate_private_state_plan(
            &plan(true, vec![subscription(Channel::Positions, None)]),
            true,
        )
        .unwrap();
        assert!(matches!(
            validate_private_state_plan(&plan(true, executing()), true),
            Err(ConnectionError::LoginFailed(_))
        ));
    }

    #[test]
    fn private_bootstrap_urls_are_exact_and_environment_scoped() {
        for destination in [
            "wss://wspap.okx.com:8443/ws/v5/private",
            "wss://wsuspap.okx.com:8443/ws/v5/private",
            "wss://wseeapap.okx.com:8443/ws/v5/private",
            "ws://127.0.0.1:18082/ws/v5/private",
            "ws://[::1]:18082/ws/v5/private",
        ] {
            validate_private_websocket_url(destination, true).unwrap();
        }
        for destination in [
            "wss://ws.okx.com:8443/ws/v5/private",
            "wss://wsus.okx.com:8443/ws/v5/private",
            "wss://wseea.okx.com:8443/ws/v5/private",
        ] {
            validate_private_websocket_url(destination, false).unwrap();
        }
        for (destination, demo_trading, expected) in [
            (
                "wss://attacker.example:8443/ws/v5/private",
                false,
                "documented OKX production",
            ),
            (
                "wss://ws.okx.com:8443/ws/v5/private",
                true,
                "documented OKX demo",
            ),
            (
                "wss://wspap.okx.com:8443/ws/v5/private",
                false,
                "documented OKX production",
            ),
            (
                "wss://ws.okx.com/ws/v5/private",
                false,
                "explicit port 8443",
            ),
            (
                "wss://ws.okx.com:9443/ws/v5/private",
                false,
                "explicit port 8443",
            ),
            (
                "wss://ws.okx.com:8443/ws/v5/public",
                false,
                "exact path /ws/v5/private",
            ),
            (
                "wss://ws.okx.com:8443/ws/v5/private?redirect=1",
                false,
                "without query or fragment",
            ),
            (
                "wss://key@ws.okx.com:8443/ws/v5/private",
                false,
                "must not contain user information",
            ),
            ("ws://ws.okx.com:8443/ws/v5/private", false, "must use wss"),
            (
                "wss://127.0.0.1:8443/ws/v5/private",
                false,
                "demo-test only",
            ),
        ] {
            let error = validate_private_websocket_url(destination, demo_trading).unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "expected {expected:?} for {destination:?}, got {error}"
            );
        }
    }

    #[test]
    fn order_command_start_validates_role_account_environment_endpoint_and_pacing() {
        let wire = fake_wire(&[]);
        let loopback = "ws://127.0.0.1:18082/ws/v5/private";

        let error = order_start_error(
            order_session_factory(&wire, "main", false),
            order_command_config(
                "main",
                loopback,
                ConnectionAttemptPacer::new(Duration::ZERO),
            ),
        );
        assert!(error.to_string().contains("demo-trading only"), "{error}");

        let error = order_start_error(
            order_session_factory(&wire, "main", true),
            order_command_config(
                "other",
                loopback,
                ConnectionAttemptPacer::new(Duration::ZERO),
            ),
        );
        assert!(
            error.to_string().contains("does not match credential role"),
            "{error}"
        );

        let error = order_start_error(
            order_session_factory(&wire, "main", true),
            order_command_config(
                "main",
                "ws://127.0.0.1:18082/ws/v5/public",
                ConnectionAttemptPacer::new(Duration::ZERO),
            ),
        );
        assert!(
            error.to_string().contains("exact path /ws/v5/private"),
            "{error}"
        );

        let official = "wss://wspap.okx.com:8443/ws/v5/private";
        let error = order_start_error(
            order_session_factory(&wire, "main", true),
            order_command_config(
                "main",
                official,
                ConnectionAttemptPacer::new(Duration::ZERO),
            ),
        );
        assert!(
            error.to_string().contains("pacing must be at least"),
            "{error}"
        );

        let error = order_start_error(
            order_session_factory(&wire, "main", true),
            order_command_config(
                "main",
                official,
                ConnectionAttemptPacer::new(Duration::from_millis(
                    OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS,
                )),
            ),
        );
        assert!(error.to_string().contains("process-shared"), "{error}");
        assert_trace(&wire, &[]);
    }

    #[test]
    fn demo_bound_gateway_authority_is_released_exactly_once() {
        let wire = fake_wire(&[]);
        let role = role_wire(&wire);
        let order_transport = Arc::new(OrderCommandTransportSlot::default());
        let mut roles = DemoRoles {
            observe: observe_roles(Arc::clone(&role), false, true),
            regular_order_gateway: Some(RegularOrderGatewayRoles {
                execution: RegularExecution {
                    wire: Arc::clone(&role),
                    expected_account_id: "main".to_string(),
                    order_transport: Arc::clone(&order_transport),
                },
                order_sessions: RegularOrderSessionFactory {
                    wire: Arc::clone(&role),
                    expected_account_id: "main".to_string(),
                    demo_trading: true,
                },
                order_transport,
            }),
            safety: Some(LiveSafety { wire: role }),
        };

        assert!(
            roles
                .take_bound_order_gateway(std::collections::HashMap::new(), PacingPolicy::default())
                .is_ok()
        );
        assert!(
            roles
                .take_bound_order_gateway(std::collections::HashMap::new(), PacingPolicy::default())
                .is_err()
        );
        assert!(roles.take_safety().is_some());
        assert!(roles.take_safety().is_none());
        assert!(roles.take_private_state_sessions().is_some());
        assert!(roles.take_private_state_sessions().is_none());
        assert_trace(&wire, &[]);
    }

    #[tokio::test]
    async fn command_transport_and_session_authority_are_released_after_teardown() {
        let wire = fake_wire(&[]);
        let wire_weak = Arc::downgrade(&wire);
        let role = role_wire(&wire);
        let order_transport = Arc::new(OrderCommandTransportSlot::default());
        let order_transport_weak = Arc::downgrade(&order_transport);
        let mut roles = DemoRoles {
            observe: observe_roles(Arc::clone(&role), false, true),
            regular_order_gateway: Some(RegularOrderGatewayRoles {
                execution: RegularExecution {
                    wire: Arc::clone(&role),
                    expected_account_id: "main".to_string(),
                    order_transport: Arc::clone(&order_transport),
                },
                order_sessions: RegularOrderSessionFactory {
                    wire: Arc::clone(&role),
                    expected_account_id: "main".to_string(),
                    demo_trading: true,
                },
                order_transport,
            }),
            safety: Some(LiveSafety {
                wire: Arc::clone(&role),
            }),
        };
        let bound = roles
            .take_bound_order_gateway(std::collections::HashMap::new(), PacingPolicy::default())
            .unwrap();
        drop(roles);
        drop(role);
        drop(wire);

        let (gateway, lifecycle, status) = bound
            .start_and_install(order_command_config(
                "main",
                "ws://127.0.0.1:1/ws/v5/private",
                ConnectionAttemptPacer::new(Duration::ZERO),
            ))
            .unwrap();
        assert!(order_transport_weak.upgrade().is_some());
        assert!(wire_weak.upgrade().is_some());

        drop(status);
        drop(gateway);
        assert!(
            order_transport_weak.upgrade().is_none(),
            "the command lifecycle/session factory must not retain the gateway transport slot"
        );
        assert!(
            wire_weak.upgrade().is_some(),
            "the running lifecycle must retain its session authority"
        );

        lifecycle.shutdown().await.unwrap();
        assert!(
            wire_weak.upgrade().is_none(),
            "shutdown must release the final credentialed session authority"
        );
    }

    #[test]
    fn observe_private_state_authority_is_released_exactly_once() {
        let wire = fake_wire(&[]);
        let mut roles = observe_roles(role_wire(&wire), false, true);

        assert!(roles.take_private_state_sessions().is_some());
        assert!(roles.take_private_state_sessions().is_none());
        assert_trace(&wire, &[]);
    }

    #[test]
    fn private_state_factory_binds_the_explicit_account_to_the_expected_plan_identity() {
        let wire = fake_wire(&[]);
        let plan = SocketPlan {
            conn_id: ConnId::new("okx-private-main-r0"),
            venue: Venue::Okx,
            private: true,
            subscriptions: vec![Subscription::private(
                Venue::Okx,
                Channel::Positions,
                FeedPriority::Critical,
            )],
        };
        let result = PrivateStateSessionFactory {
            wire: role_wire(&wire),
            allow_fills: false,
            demo_trading: true,
        }
        .bootstrap_factory("other", plan, "ws://127.0.0.1:9/ws/v5/private");

        let error = result
            .err()
            .expect("account/connection-plan mismatch was accepted");
        assert!(
            invalid_configuration(error).contains("must use connection id okx-private-other-r0")
        );
        assert_trace(&wire, &[]);
    }

    #[tokio::test]
    async fn private_state_role_cannot_sign_for_a_different_websocket_destination() {
        let plan = SocketPlan {
            conn_id: ConnId::new("okx-private-main-r0"),
            venue: Venue::Okx,
            private: true,
            subscriptions: [Channel::Account, Channel::Orders, Channel::Positions]
                .into_iter()
                .map(|channel| Subscription::private(Venue::Okx, channel, FeedPriority::Critical))
                .collect(),
        };
        let bound_url = "ws://127.0.0.1:9/ws/v5/private";
        let selected_url = "ws://127.0.0.1:10/ws/v5/private";
        let wire = fake_wire(&[]);
        let factory = PrivateStateSessionFactory {
            wire: role_wire(&wire),
            allow_fills: false,
            demo_trading: true,
        }
        .bootstrap_factory("main", plan.clone(), bound_url)
        .unwrap();
        let mut feed = spawn_supervised_feed(
            Arc::new(reap_venue::okx::OkxAdapter::new(
                "ws://127.0.0.1:8/ws/v5/public",
                selected_url,
            )),
            vec![plan.clone()],
            factory,
            4,
            ConnectionAttemptPacer::new(Duration::ZERO),
            ReconnectPolicy::default(),
        );

        let status = tokio::time::timeout(Duration::from_secs(2), feed.status.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.kind, ConnectionStatusKind::Fatal);
        assert!(status.reason.contains(bound_url));
        assert!(status.reason.contains(selected_url));
        feed.shutdown().await;
        assert_trace(&wire, &[]);

        let wire = fake_wire(&[]);
        let factory = PrivateStateSessionFactory {
            wire: role_wire(&wire),
            allow_fills: false,
            demo_trading: true,
        }
        .bootstrap_factory("main", plan.clone(), bound_url)
        .unwrap();
        let mut feed = spawn_supervised_feed(
            Arc::new(reap_venue::okx::OkxAdapter::new(
                "ws://127.0.0.1:8/ws/v5/public",
                bound_url,
            )),
            vec![plan],
            factory,
            4,
            ConnectionAttemptPacer::new(Duration::ZERO),
            ReconnectPolicy::default(),
        );

        for _ in 0..2 {
            let status = tokio::time::timeout(Duration::from_secs(2), feed.status.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(status.kind, ConnectionStatusKind::Disconnected);
        }
        feed.shutdown().await;
        assert_trace(&wire, &[Call::Login, Call::Login]);
    }

    #[tokio::test]
    async fn private_state_bootstrap_rejects_a_different_packed_plan_before_login() {
        let expected_plan = SocketPlan {
            conn_id: ConnId::new("okx-private-main-r0"),
            venue: Venue::Okx,
            private: true,
            subscriptions: [Channel::Account, Channel::Orders, Channel::Positions]
                .into_iter()
                .map(|channel| Subscription::private(Venue::Okx, channel, FeedPriority::Critical))
                .collect(),
        };
        let mut selected_plan = expected_plan.clone();
        selected_plan.subscriptions.pop();
        let private_url = "ws://127.0.0.1:9/ws/v5/private";
        let wire = fake_wire(&[]);
        let factory = PrivateStateSessionFactory {
            wire: role_wire(&wire),
            allow_fills: false,
            demo_trading: true,
        }
        .bootstrap_factory("main", expected_plan, private_url)
        .unwrap();
        let mut feed = spawn_supervised_feed(
            Arc::new(reap_venue::okx::OkxAdapter::new(
                "ws://127.0.0.1:8/ws/v5/public",
                private_url,
            )),
            vec![selected_plan],
            factory,
            4,
            ConnectionAttemptPacer::new(Duration::ZERO),
            ReconnectPolicy::default(),
        );

        let status = tokio::time::timeout(Duration::from_secs(2), feed.status.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.kind, ConnectionStatusKind::Fatal);
        assert!(status.reason.contains("plan mismatch"));
        feed.shutdown().await;
        assert_trace(&wire, &[]);
    }

    fn typed_strategy_prepared_submit() -> (PreparedRegularSubmit, Arc<FakeWire>, String) {
        let mut strategy = ChaosStrategy::new(ChaosConfig {
            ref_symbol: "BTC-USDT".to_string(),
            delta_limit_usd: 50_000.0,
            active_hedge_threshold_usd: 1_000.0,
            min_hedge_interval_ms: 0,
            risk_groups: vec![RiskGroupConfig {
                name: "main".to_string(),
                symbols: vec!["BTC-USDT".to_string(), "BTC-PERP".to_string()],
                soft_delta_limit_usd: 25_000.0,
                hard_delta_limit_usd: 40_000.0,
                live_order_limit_usd: 100_000.0,
                ..RiskGroupConfig::default()
            }],
            instruments: vec![
                InstrumentConfig {
                    symbol: "BTC-USDT".to_string(),
                    risk_group: "main".to_string(),
                    kind: InstrumentKindConfig::Spot,
                    tick_size: 0.1,
                    lot_size: 0.0001,
                    min_trade_size: 0.0001,
                    max_order_size_usd: 5_000.0,
                    min_order_size_usd: 100.0,
                    max_order_size: 1.0,
                    ..InstrumentConfig::default()
                },
                InstrumentConfig {
                    symbol: "BTC-PERP".to_string(),
                    risk_group: "main".to_string(),
                    kind: InstrumentKindConfig::Future,
                    tick_size: 0.1,
                    lot_size: 1.0,
                    min_trade_size: 1.0,
                    contract_value: 0.001,
                    max_order_size_usd: 5_000.0,
                    min_order_size_usd: 100.0,
                    max_order_size: 200.0,
                    min_position: -10_000.0,
                    max_position: 10_000.0,
                    ..InstrumentConfig::default()
                },
            ],
            ..ChaosConfig::default()
        })
        .unwrap();
        let intent = include_str!("../../../fixtures/normalized/chaos_quote_hedge.jsonl")
            .lines()
            .filter(|line| !line.trim().is_empty())
            .flat_map(|line| {
                let event = serde_json::from_str::<NormalizedEvent>(line).unwrap();
                strategy.on_execution_event(&event.into_strategy_event())
            })
            .find(|intent| {
                intent
                    .as_quote()
                    .is_some_and(|quote| quote.symbol() == "BTC-USDT")
            })
            .expect("fixture must emit a typed Chaos quote");
        let wire = fake_wire(&[]);
        let role = role_wire(&wire);
        let order_transport = Arc::new(OrderCommandTransportSlot::default());
        let mut gateway = OkxOrderGateway::new(
            "main",
            Box::new(RegularExecution {
                wire: Arc::clone(&role),
                expected_account_id: "main".to_string(),
                order_transport: Arc::clone(&order_transport),
            }),
            Arc::new(RegularReconciliation {
                wire: Arc::clone(&role),
            }),
            std::collections::HashMap::from([("BTC-USDT".to_string(), OkxTradeMode::Cash)]),
            PacingPolicy::default(),
        )
        .unwrap();
        let (profile_set, client_order_id_generator) = gateway
            .take_approval_scope()
            .unwrap()
            .bind_profiles_and_client_id_generator(
                [RegularExecutionProfile::new(
                    "BTC-USDT",
                    "main",
                    InstrumentRiskModel::Spot,
                    InstrumentOrderLimits {
                        max_limit_quantity: 1_000_000.0,
                        max_limit_notional_usd: None,
                    },
                    0.1,
                    0.0001,
                    0.0001,
                    reap_venue::okx::OkxRegularOrderRules::from_exchange_decimals(
                        "0.1", "0.0001", "0.0001",
                    )
                    .unwrap(),
                    true,
                    false,
                    true,
                )],
                "typed",
                1,
            )
            .unwrap();
        let policy = RegularExecutionPolicy::from_profile_sets([profile_set]).unwrap();
        let approved = policy.authorize_submit(intent).unwrap();
        let mut owned = OwnedRegularOrders::default();
        let mut private_state = PrivateStateReducer::new();
        let generated_client_order_id = client_order_id_generator.next(1);
        let expected_client_order_id = generated_client_order_id.as_str().to_string();
        let (_, reserved) = owned
            .reserve_local(approved, generated_client_order_id, &mut private_state, 1)
            .unwrap();
        let SubmitPreparation::Ready(prepared) =
            gateway.prepare_submit("typed-decision", reserved).unwrap()
        else {
            panic!("fresh typed decision must require transport");
        };
        (prepared, wire, expected_client_order_id)
    }

    #[test]
    fn typed_strategy_authority_is_serialized_only_after_gateway_preparation() {
        let (prepared, wire, expected_client_order_id) = typed_strategy_prepared_submit();
        assert_eq!(prepared.account_id(), "main");
        let rest_shaped: serde_json::Value =
            serde_json::from_str(&serialize_place(&regular_place_order(&prepared)).unwrap())
                .unwrap();

        let factory = RegularOrderSessionFactory {
            wire: role_wire(&wire),
            expected_account_id: "main".to_string(),
            demo_trading: true,
        };
        let request = factory.place_request("typed1", 123_456, prepared).unwrap();
        let request: serde_json::Value = serde_json::from_str(&request).unwrap();
        assert_eq!(request["op"], "order");
        assert_eq!(request["expTime"], "123456");
        assert_eq!(request["args"][0]["instId"], "BTC-USDT");
        assert_eq!(request["args"][0]["tdMode"], "cash");
        assert_eq!(request["args"][0]["clOrdId"], expected_client_order_id);
        assert_eq!(request["args"][0]["ordType"], "post_only");
        assert_eq!(request["args"][0]["px"], rest_shaped["px"]);
        assert_eq!(request["args"][0]["sz"], rest_shaped["sz"]);
        assert!(request["args"][0].get("account_id").is_none());
        assert_trace(&wire, &[]);
    }

    mod goal_d_serializer_benchmark;

    #[test]
    #[ignore = "release-only Goal D prepared-request serialization benchmark"]
    fn goal_d_prepared_serializer_benchmark() {
        goal_d_serializer_benchmark::run();
    }

    #[test]
    fn exact_regular_numbers_are_byte_identical_in_rest_shaped_and_websocket_fields() {
        let cases = [
            ("1", "1"),
            ("0.10", "0.1"),
            ("0.05", "0.05"),
            ("1e-4", "0.0001"),
            ("1e-12", "0.000000000001"),
            ("9007199254740991", "9007199254740991"),
        ];

        for (source, canonical) in cases {
            let order = OkxPlaceOrder {
                symbol: "BTC-USDT".to_string(),
                trade_mode: OkxTradeMode::Cash,
                side: Side::Buy,
                time_in_force: TimeInForce::PostOnly,
                price: OkxExactDecimal::parse(source).unwrap(),
                qty: OkxExactDecimal::parse(source).unwrap(),
                client_order_id: "client-1".to_string(),
                reduce_only: false,
                self_trade_prevention: None,
            };
            let rest_shaped: serde_json::Value =
                serde_json::from_str(&serialize_place(&order).unwrap()).unwrap();
            let websocket: serde_json::Value =
                serde_json::from_str(&build_ws_place_request("exact1", 1, &order).unwrap())
                    .unwrap();
            let websocket_argument = &websocket["args"][0];

            assert_eq!(rest_shaped["px"], canonical, "{source}");
            assert_eq!(rest_shaped["sz"], canonical, "{source}");
            assert_eq!(
                rest_shaped["px"].as_str().unwrap().as_bytes(),
                websocket_argument["px"].as_str().unwrap().as_bytes(),
                "{source}"
            );
            assert_eq!(
                rest_shaped["sz"].as_str().unwrap().as_bytes(),
                websocket_argument["sz"].as_str().unwrap().as_bytes(),
                "{source}"
            );
        }
    }

    #[test]
    fn regular_order_session_factory_builds_exact_place_and_cancel_operations() {
        let wire = fake_wire(&[]);
        let factory = RegularOrderSessionFactory {
            wire: role_wire(&wire),
            expected_account_id: "main".to_string(),
            demo_trading: true,
        };

        assert_eq!(
            factory.login_message().unwrap(),
            r#"{"op":"login","args":[{"apiKey":"key","passphrase":"pass","timestamp":"1538054050","sign":"signature"}]}"#
        );
        let place = build_ws_place_request(
            "place1",
            123_456,
            &OkxPlaceOrder {
                symbol: "BTC-USDT".to_string(),
                trade_mode: OkxTradeMode::Cash,
                side: Side::Buy,
                time_in_force: TimeInForce::PostOnly,
                price: OkxExactDecimal::parse("100.5").unwrap(),
                qty: OkxExactDecimal::parse("0.25").unwrap(),
                client_order_id: "client-1".to_string(),
                reduce_only: false,
                self_trade_prevention: Some(SelfTradePrevention::CancelMaker),
            },
        )
        .unwrap();
        let cancel = build_ws_cancel_request(
            "cancel1",
            &OkxCancelOrder {
                symbol: "BTC-USDT".to_string(),
                exchange_order_id: None,
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
            r#"{"id":"cancel1","op":"cancel-order","args":[{"clOrdId":"client-1","instId":"BTC-USDT"}]}"#
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
