#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;

pub const OKX_PUBLIC_TIME_ENDPOINT: &str = "/api/v5/public/time";
pub const OKX_ACCOUNT_CONFIG_ENDPOINT: &str = "/api/v5/account/config";
pub const OKX_ACCOUNT_BALANCE_ENDPOINT: &str = "/api/v5/account/balance";
pub const OKX_ACCOUNT_POSITIONS_ENDPOINT: &str = "/api/v5/account/positions";
pub const OKX_INDEX_TICKER_ENDPOINT: &str = "/api/v5/market/index-tickers";
pub const OKX_RECENT_FILLS_ENDPOINT: &str = "/api/v5/trade/fills";
pub const OKX_ACCOUNT_BILLS_ENDPOINT: &str = "/api/v5/account/bills";
pub const OKX_REGULAR_ORDER_DETAILS_ENDPOINT: &str = "/api/v5/trade/order";
pub const OKX_REGULAR_OPEN_ORDERS_ENDPOINT: &str = "/api/v5/trade/orders-pending";

/// The complete HTTP endpoint allowlist for offline evidence collection.
///
/// Every operation is read-only. This list intentionally excludes system
/// status, authenticated metadata, all order writes, algo/spread endpoints,
/// websocket login, and arbitrary request execution.
pub const OKX_EVIDENCE_ENDPOINT_ALLOWLIST: &[&str] = &[
    OKX_PUBLIC_TIME_ENDPOINT,
    OKX_ACCOUNT_CONFIG_ENDPOINT,
    OKX_ACCOUNT_BALANCE_ENDPOINT,
    OKX_ACCOUNT_POSITIONS_ENDPOINT,
    OKX_INDEX_TICKER_ENDPOINT,
    OKX_RECENT_FILLS_ENDPOINT,
    OKX_ACCOUNT_BILLS_ENDPOINT,
    OKX_REGULAR_ORDER_DETAILS_ENDPOINT,
    OKX_REGULAR_OPEN_ORDERS_ENDPOINT,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceCredentialEnvironment {
    account_id: String,
    api_key_environment: String,
    secret_key_environment: String,
    passphrase_environment: String,
}

impl EvidenceCredentialEnvironment {
    pub fn new(
        account_id: impl Into<String>,
        api_key_environment: impl Into<String>,
        secret_key_environment: impl Into<String>,
        passphrase_environment: impl Into<String>,
    ) -> Self {
        Self {
            account_id: account_id.into(),
            api_key_environment: api_key_environment.into(),
            secret_key_environment: secret_key_environment.into(),
            passphrase_environment: passphrase_environment.into(),
        }
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn api_key_environment(&self) -> &str {
        &self.api_key_environment
    }

    pub fn secret_key_environment(&self) -> &str {
        &self.secret_key_environment
    }

    pub fn passphrase_environment(&self) -> &str {
        &self.passphrase_environment
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceHttpConfig {
    rest_url: String,
    demo_trading: bool,
    connect_timeout: Duration,
    request_timeout: Duration,
}

impl EvidenceHttpConfig {
    pub fn new(
        rest_url: impl Into<String>,
        demo_trading: bool,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Self {
        Self {
            rest_url: rest_url.into(),
            demo_trading,
            connect_timeout,
            request_timeout,
        }
    }

    pub fn rest_url(&self) -> &str {
        &self.rest_url
    }

    pub fn demo_trading(&self) -> bool {
        self.demo_trading
    }

    pub fn connect_timeout(&self) -> Duration {
        self.connect_timeout
    }

    pub fn request_timeout(&self) -> Duration {
        self.request_timeout
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EvidenceClientFactoryError {
    #[error("account {account_id} credential environment variable {name} is not set")]
    MissingCredential { account_id: String, name: String },
    #[error("invalid evidence client configuration: {0}")]
    InvalidConfiguration(String),
    #[error("HTTP transport failed: {0}")]
    Transport(String),
}

/// Read/parse failures retain the existing OKX REST error text so collector
/// errors and operator diagnostics remain stable while raw wire types become
/// private.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EvidenceReadError {
    #[error("request authentication failed: {0}")]
    Authentication(String),
    #[error("request serialization failed: {0}")]
    Serialization(String),
    #[error("HTTP transport failed: {0}")]
    Transport(String),
    #[error("OKX API error {code}: {message}")]
    Api { code: String, message: String },
    #[error("OKX returned no data for {operation}")]
    EmptyData { operation: &'static str },
    #[error("invalid OKX response field {field}={value:?}: {message}")]
    InvalidField {
        field: &'static str,
        value: String,
        message: String,
    },
    #[error("{0}")]
    Other(String),
}

#[derive(Clone, PartialEq, Eq)]
pub struct EvidenceResponse<Kind> {
    request_path: String,
    response_body: String,
    marker: PhantomData<fn() -> Kind>,
}

impl<Kind> EvidenceResponse<Kind> {
    /// Constructs a credential-free response contract. This is public so role
    /// fakes can be implemented without receiving production signing access.
    pub fn new(request_path: impl Into<String>, response_body: impl Into<String>) -> Self {
        Self {
            request_path: request_path.into(),
            response_body: response_body.into(),
            marker: PhantomData,
        }
    }

    pub fn request_path(&self) -> &str {
        &self.request_path
    }

    pub fn response_body(&self) -> &str {
        &self.response_body
    }

    pub fn into_parts(self) -> (String, String) {
        (self.request_path, self.response_body)
    }
}

impl<Kind> fmt::Debug for EvidenceResponse<Kind> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EvidenceResponse")
            .field("request_path", &self.request_path)
            .field("response_body", &self.response_body)
            .finish()
    }
}

macro_rules! response_kind {
    ($kind:ident, $response:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $kind {}
        pub type $response = EvidenceResponse<$kind>;
    };
}

response_kind!(AccountConfigKind, AccountConfigResponse);
response_kind!(AccountBalanceKind, AccountBalanceResponse);
response_kind!(AccountPositionsKind, AccountPositionsResponse);
response_kind!(IndexTickerKind, IndexTickerResponse);
response_kind!(RecentFillsPageKind, RecentFillsPageResponse);
response_kind!(AccountBillsPageKind, AccountBillsPageResponse);
response_kind!(RegularOrderDetailsKind, RegularOrderDetailsResponse);
response_kind!(RegularOpenOrdersKind, RegularOpenOrdersResponse);

/// The credential-free consumer port for the current offline evidence commands.
///
/// It deliberately has no generic request method, signer access, mutation,
/// websocket operation, endpoint selector, or live metadata operation.
#[async_trait]
pub trait EvidenceReadOnly: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    async fn server_time_ms(&self) -> Result<u64, Self::Error>;

    async fn account_config(&self) -> Result<AccountConfigResponse, Self::Error>;

    async fn account_balance(&self) -> Result<AccountBalanceResponse, Self::Error>;

    async fn account_positions(&self) -> Result<AccountPositionsResponse, Self::Error>;

    async fn index_ticker(&self, symbol: &str) -> Result<IndexTickerResponse, Self::Error>;

    async fn recent_fills_page(
        &self,
        after: Option<&str>,
    ) -> Result<RecentFillsPageResponse, Self::Error>;

    async fn account_bills_page(
        &self,
        begin_ms: u64,
        end_ms: u64,
        after: Option<&str>,
    ) -> Result<AccountBillsPageResponse, Self::Error>;

    async fn regular_order_details(
        &self,
        symbol: &str,
        exchange_order_id: &str,
        client_order_id: &str,
    ) -> Result<RegularOrderDetailsResponse, Self::Error>;

    async fn regular_open_orders(&self) -> Result<RegularOpenOrdersResponse, Self::Error>;
}

/// Two-stage construction keeps current collector failure ordering intact:
/// output/config/journal work, credential lookup, provenance hashing, transport
/// construction, then network access.
pub trait EvidenceClientFactory {
    type PreparedCredentials;
    type Client: EvidenceReadOnly;

    fn prepare_credentials(
        &self,
        environment: &EvidenceCredentialEnvironment,
    ) -> Result<Self::PreparedCredentials, EvidenceClientFactoryError>;

    fn connect(
        &self,
        prepared: Self::PreparedCredentials,
        config: &EvidenceHttpConfig,
    ) -> Result<Self::Client, EvidenceClientFactoryError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_allowlist_is_exact_and_read_only() {
        assert_eq!(
            OKX_EVIDENCE_ENDPOINT_ALLOWLIST,
            [
                "/api/v5/public/time",
                "/api/v5/account/config",
                "/api/v5/account/balance",
                "/api/v5/account/positions",
                "/api/v5/market/index-tickers",
                "/api/v5/trade/fills",
                "/api/v5/account/bills",
                "/api/v5/trade/order",
                "/api/v5/trade/orders-pending",
            ]
        );
        assert!(
            OKX_EVIDENCE_ENDPOINT_ALLOWLIST
                .iter()
                .all(|endpoint| !endpoint.contains("cancel") && !endpoint.contains("algo"))
        );
    }

    #[test]
    fn response_contract_retains_exact_bytes_and_path() {
        let response = RecentFillsPageResponse::new(
            "/api/v5/trade/fills?limit=100",
            "{\"code\":\"0\",\"data\":[]}",
        );
        assert_eq!(response.request_path(), "/api/v5/trade/fills?limit=100");
        assert_eq!(response.response_body(), "{\"code\":\"0\",\"data\":[]}");
    }
}
