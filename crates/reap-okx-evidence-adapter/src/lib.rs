#![forbid(unsafe_code)]

use std::fmt;

use async_trait::async_trait;
use reap_evidence_core::{
    AccountBalanceResponse, AccountBillsPageResponse, AccountConfigResponse,
    AccountPositionsResponse, EvidenceClientFactory, EvidenceClientFactoryError,
    EvidenceCredentialEnvironment, EvidenceHttpConfig, EvidenceReadError, EvidenceReadOnly,
    IndexTickerResponse, OKX_ACCOUNT_BALANCE_ENDPOINT, OKX_ACCOUNT_BILLS_ENDPOINT,
    OKX_ACCOUNT_CONFIG_ENDPOINT, OKX_ACCOUNT_POSITIONS_ENDPOINT, OKX_INDEX_TICKER_ENDPOINT,
    OKX_PUBLIC_TIME_ENDPOINT, OKX_RECENT_FILLS_ENDPOINT, OKX_REGULAR_OPEN_ORDERS_ENDPOINT,
    OKX_REGULAR_ORDER_DETAILS_ENDPOINT, RecentFillsPageResponse, RegularOpenOrdersResponse,
    RegularOrderDetailsResponse,
};
use reap_okx_wire::{
    Client as WireClient, Credentials as WireCredentials, Error as WireError,
    ReqwestTransport as WireReqwestTransport, Response as WireResponse, Transport as WireTransport,
};
use reap_venue::okx::{
    RestError, parse_okx_account_balance_response_json, parse_okx_account_config_response_json,
    parse_okx_account_positions_response_json, parse_okx_bill_page_response_json,
    parse_okx_fill_page_response_json, parse_okx_index_ticker_response_json,
    parse_okx_open_orders_response_json, parse_okx_order_details_response_json,
    parse_okx_server_time_response_json,
};
use url::{Host, Url, form_urlencoded};

/// The only production constructor for authenticated offline evidence reads.
///
/// Construction is deliberately split into credential lookup and transport
/// creation so existing collectors can retain their current provenance and
/// filesystem failure ordering.
#[derive(Debug, Clone, Copy, Default)]
pub struct OkxEvidenceClientFactory;

impl OkxEvidenceClientFactory {
    pub const fn new() -> Self {
        Self
    }
}

/// Opaque role-bound credentials prepared for an evidence client.
///
/// There are intentionally no getters, signing methods, conversions, or clone
/// implementation on this type.
pub struct PreparedOkxEvidenceCredentials {
    credentials: RawCredentials,
}

impl fmt::Debug for PreparedOkxEvidenceCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedOkxEvidenceCredentials")
            .field("credentials", &"[REDACTED]")
            .finish()
    }
}

/// Opaque read-only OKX evidence client.
///
/// Its public behavior is solely the `EvidenceReadOnly` port. Raw requests,
/// credentials, signing, transport, and endpoint selection remain private.
pub struct OkxEvidenceClient {
    inner: EvidenceClient<WireReqwestTransport>,
}

impl EvidenceClientFactory for OkxEvidenceClientFactory {
    type PreparedCredentials = PreparedOkxEvidenceCredentials;
    type Client = OkxEvidenceClient;

    fn prepare_credentials(
        &self,
        environment: &EvidenceCredentialEnvironment,
    ) -> Result<Self::PreparedCredentials, EvidenceClientFactoryError> {
        Ok(PreparedOkxEvidenceCredentials {
            credentials: load_credentials_with(environment, |name| std::env::var(name))?,
        })
    }

    fn connect(
        &self,
        prepared: Self::PreparedCredentials,
        config: &EvidenceHttpConfig,
    ) -> Result<Self::Client, EvidenceClientFactoryError> {
        // Preserve the current collector order: transport construction happens
        // after credential lookup and provenance hashing, before the signer is
        // assembled into the endpoint-specific client.
        validate_rest_origin(config.rest_url(), config.demo_trading())?;
        let transport = WireReqwestTransport::with_timeouts(
            config.rest_url(),
            config.connect_timeout(),
            config.request_timeout(),
        )
        .map_err(map_factory_wire_error)?;
        let credentials = WireCredentials::new(
            prepared.credentials.api_key,
            prepared.credentials.secret_key,
            prepared.credentials.passphrase,
        )
        .map_err(map_factory_wire_error)?;
        Ok(OkxEvidenceClient {
            inner: EvidenceClient {
                wire: WireClient::new(transport, credentials, config.demo_trading()),
            },
        })
    }
}

fn validate_rest_origin(
    rest_url: &str,
    demo_trading: bool,
) -> Result<(), EvidenceClientFactoryError> {
    const OFFICIAL_HOSTS: &[&str] = &[
        "openapi.okx.com",
        "www.okx.com",
        "us.okx.com",
        "eea.okx.com",
        "tr.okx.com",
    ];

    let url = Url::parse(rest_url).map_err(|error| {
        EvidenceClientFactoryError::InvalidConfiguration(format!(
            "REST URL must be an absolute approved origin: {error}"
        ))
    })?;
    if !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(EvidenceClientFactoryError::InvalidConfiguration(
            "REST URL must contain only an origin (no credentials, path, query, or fragment)"
                .to_string(),
        ));
    }

    let host = url.host().ok_or_else(|| {
        EvidenceClientFactoryError::InvalidConfiguration("REST URL must contain a host".to_string())
    })?;
    let loopback = match &host {
        Host::Domain(host) => host.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(address) => address.is_loopback(),
        Host::Ipv6(address) => address.is_loopback(),
    };
    let official = url.scheme() == "https"
        && url.port().is_none()
        && matches!(
            &host,
            Host::Domain(host)
                if OFFICIAL_HOSTS
                    .iter()
                    .any(|allowed| host.eq_ignore_ascii_case(allowed))
        );
    let test_proxy = demo_trading
        && loopback
        && matches!(url.scheme(), "http" | "https")
        && url.port().is_some();
    if official || test_proxy {
        Ok(())
    } else {
        Err(EvidenceClientFactoryError::InvalidConfiguration(
            "REST URL must be an approved OKX HTTPS origin or a demo-only loopback test origin"
                .to_string(),
        ))
    }
}

#[async_trait]
impl EvidenceReadOnly for OkxEvidenceClient {
    type Error = EvidenceReadError;

    async fn server_time_ms(&self) -> Result<u64, Self::Error> {
        self.inner.server_time_ms().await
    }

    async fn account_config(&self) -> Result<AccountConfigResponse, Self::Error> {
        self.inner.account_config().await
    }

    async fn account_balance(&self) -> Result<AccountBalanceResponse, Self::Error> {
        self.inner.account_balance().await
    }

    async fn account_positions(&self) -> Result<AccountPositionsResponse, Self::Error> {
        self.inner.account_positions().await
    }

    async fn index_ticker(&self, symbol: &str) -> Result<IndexTickerResponse, Self::Error> {
        self.inner.index_ticker(symbol).await
    }

    async fn recent_fills_page(
        &self,
        after: Option<&str>,
    ) -> Result<RecentFillsPageResponse, Self::Error> {
        self.inner.recent_fills_page(after).await
    }

    async fn account_bills_page(
        &self,
        begin_ms: u64,
        end_ms: u64,
        after: Option<&str>,
    ) -> Result<AccountBillsPageResponse, Self::Error> {
        self.inner.account_bills_page(begin_ms, end_ms, after).await
    }

    async fn regular_order_details(
        &self,
        symbol: &str,
        exchange_order_id: &str,
        client_order_id: &str,
    ) -> Result<RegularOrderDetailsResponse, Self::Error> {
        self.inner
            .regular_order_details(symbol, exchange_order_id, client_order_id)
            .await
    }

    async fn regular_open_orders(&self) -> Result<RegularOpenOrdersResponse, Self::Error> {
        self.inner.regular_open_orders().await
    }
}

struct RawCredentials {
    api_key: String,
    secret_key: String,
    passphrase: String,
}

fn load_credentials_with<E>(
    environment: &EvidenceCredentialEnvironment,
    mut read: impl FnMut(&str) -> Result<String, E>,
) -> Result<RawCredentials, EvidenceClientFactoryError> {
    let mut required = |name: &str| {
        read(name).map_err(|_| EvidenceClientFactoryError::MissingCredential {
            account_id: environment.account_id().to_string(),
            name: name.to_string(),
        })
    };
    Ok(RawCredentials {
        api_key: required(environment.api_key_environment())?,
        secret_key: required(environment.secret_key_environment())?,
        passphrase: required(environment.passphrase_environment())?,
    })
}

struct EvidenceClient<T> {
    wire: WireClient<T>,
}

impl<T> EvidenceClient<T>
where
    T: WireTransport,
{
    async fn server_time_ms(&self) -> Result<u64, EvidenceReadError> {
        let body = self.public_get(OKX_PUBLIC_TIME_ENDPOINT).await?;
        parse_okx_server_time_response_json(body.as_bytes()).map_err(map_rest_error)
    }

    async fn account_config(&self) -> Result<AccountConfigResponse, EvidenceReadError> {
        let path = OKX_ACCOUNT_CONFIG_ENDPOINT.to_string();
        let body = self.signed_get(&path).await?;
        parse_okx_account_config_response_json(body.as_bytes()).map_err(map_rest_error)?;
        Ok(AccountConfigResponse::new(path, body))
    }

    async fn account_balance(&self) -> Result<AccountBalanceResponse, EvidenceReadError> {
        let path = OKX_ACCOUNT_BALANCE_ENDPOINT.to_string();
        let body = self.signed_get(&path).await?;
        parse_okx_account_balance_response_json(body.as_bytes()).map_err(map_rest_error)?;
        Ok(AccountBalanceResponse::new(path, body))
    }

    async fn account_positions(&self) -> Result<AccountPositionsResponse, EvidenceReadError> {
        let path = OKX_ACCOUNT_POSITIONS_ENDPOINT.to_string();
        let body = self.signed_get(&path).await?;
        parse_okx_account_positions_response_json(body.as_bytes()).map_err(map_rest_error)?;
        Ok(AccountPositionsResponse::new(path, body))
    }

    async fn index_ticker(&self, symbol: &str) -> Result<IndexTickerResponse, EvidenceReadError> {
        let symbol = symbol.trim();
        if symbol.is_empty() {
            return Err(EvidenceReadError::InvalidField {
                field: "instId",
                value: symbol.to_string(),
                message: "must be non-empty".to_string(),
            });
        }
        let path = query_path(OKX_INDEX_TICKER_ENDPOINT, [("instId", Some(symbol))]);
        let body = self.public_get(&path).await?;
        let ticker =
            parse_okx_index_ticker_response_json(body.as_bytes()).map_err(map_rest_error)?;
        if ticker.symbol != symbol {
            return Err(EvidenceReadError::InvalidField {
                field: "instId",
                value: ticker.symbol,
                message: format!("does not match requested {symbol}"),
            });
        }
        Ok(IndexTickerResponse::new(path, body))
    }

    async fn recent_fills_page(
        &self,
        after: Option<&str>,
    ) -> Result<RecentFillsPageResponse, EvidenceReadError> {
        let path = query_path(
            OKX_RECENT_FILLS_ENDPOINT,
            [("after", after), ("limit", Some("100"))],
        );
        let body = self.signed_get(&path).await?;
        parse_okx_fill_page_response_json(body.as_bytes()).map_err(map_rest_error)?;
        Ok(RecentFillsPageResponse::new(path, body))
    }

    async fn account_bills_page(
        &self,
        begin_ms: u64,
        end_ms: u64,
        after: Option<&str>,
    ) -> Result<AccountBillsPageResponse, EvidenceReadError> {
        if begin_ms == 0 || end_ms == 0 || begin_ms > end_ms {
            return Err(EvidenceReadError::InvalidField {
                field: "begin/end",
                value: format!("{begin_ms}/{end_ms}"),
                message: "must be a positive inclusive window".to_string(),
            });
        }
        let begin = begin_ms.to_string();
        let end = end_ms.to_string();
        let path = query_path(
            OKX_ACCOUNT_BILLS_ENDPOINT,
            [
                ("begin", Some(begin.as_str())),
                ("end", Some(end.as_str())),
                ("after", after),
                ("limit", Some("100")),
            ],
        );
        let body = self.signed_get(&path).await?;
        parse_okx_bill_page_response_json(body.as_bytes()).map_err(map_rest_error)?;
        Ok(AccountBillsPageResponse::new(path, body))
    }

    async fn regular_order_details(
        &self,
        symbol: &str,
        exchange_order_id: &str,
        client_order_id: &str,
    ) -> Result<RegularOrderDetailsResponse, EvidenceReadError> {
        let path = query_path(
            OKX_REGULAR_ORDER_DETAILS_ENDPOINT,
            [
                ("instId", Some(symbol)),
                ("ordId", Some(exchange_order_id)),
                ("clOrdId", Some(client_order_id)),
            ],
        );
        let body = self.signed_get(&path).await?;
        parse_okx_order_details_response_json(body.as_bytes()).map_err(map_rest_error)?;
        Ok(RegularOrderDetailsResponse::new(path, body))
    }

    async fn regular_open_orders(&self) -> Result<RegularOpenOrdersResponse, EvidenceReadError> {
        let path = OKX_REGULAR_OPEN_ORDERS_ENDPOINT.to_string();
        let body = self.signed_get(&path).await?;
        parse_okx_open_orders_response_json(body.as_bytes()).map_err(map_rest_error)?;
        Ok(RegularOpenOrdersResponse::new(path, body))
    }

    async fn public_get(&self, path: &str) -> Result<String, EvidenceReadError> {
        let response = self.wire.public_get(path).await.map_err(map_wire_error)?;
        response_body(response)
    }

    async fn signed_get(&self, path: &str) -> Result<String, EvidenceReadError> {
        let response = self.wire.get(path).await.map_err(map_wire_error)?;
        response_body(response)
    }
}

fn response_body(response: WireResponse) -> Result<String, EvidenceReadError> {
    let status = response.status();
    let body = String::from_utf8_lossy(response.body()).into_owned();
    if !response.is_success() {
        return Err(EvidenceReadError::Transport(format!(
            "HTTP status {status}: {body}"
        )));
    }
    Ok(body)
}

fn query_path<'a>(
    base: &str,
    parameters: impl IntoIterator<Item = (&'a str, Option<&'a str>)>,
) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (name, value) in parameters {
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

fn map_factory_wire_error(error: WireError) -> EvidenceClientFactoryError {
    match error {
        WireError::Transport(message) => EvidenceClientFactoryError::Transport(message),
        other => EvidenceClientFactoryError::Transport(other.to_string()),
    }
}

fn map_wire_error(error: WireError) -> EvidenceReadError {
    match error {
        WireError::InvalidSigningKey => {
            EvidenceReadError::Authentication("invalid HMAC key".to_string())
        }
        WireError::LoginSerialization(error) => EvidenceReadError::Serialization(error.to_string()),
        WireError::Transport(message) => EvidenceReadError::Transport(message),
        other => EvidenceReadError::Other(other.to_string()),
    }
}

fn map_rest_error(error: RestError) -> EvidenceReadError {
    match error {
        RestError::Serialization(error) => EvidenceReadError::Serialization(error.to_string()),
        RestError::Transport(message) => EvidenceReadError::Transport(message),
        RestError::Api { code, message } => EvidenceReadError::Api { code, message },
        RestError::EmptyData { operation } => EvidenceReadError::EmptyData { operation },
        RestError::InvalidField {
            field,
            value,
            message,
        } => EvidenceReadError::InvalidField {
            field,
            value,
            message,
        },
        other => EvidenceReadError::Other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use reap_evidence_core::OKX_EVIDENCE_ENDPOINT_ALLOWLIST;
    use reap_okx_wire::{Method as WireMethod, Request as WireRequest};

    use super::*;

    fn http_config(rest_url: &str, demo_trading: bool) -> EvidenceHttpConfig {
        EvidenceHttpConfig::new(
            rest_url,
            demo_trading,
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(1),
        )
    }

    #[test]
    fn evidence_factory_accepts_only_approved_rest_origins() {
        for (origin, demo) in [
            ("https://openapi.okx.com", false),
            ("https://www.okx.com", true),
            ("https://us.okx.com", false),
            ("https://eea.okx.com", true),
            ("https://tr.okx.com", false),
            ("http://127.0.0.1:18080", true),
            ("https://[::1]:18443", true),
        ] {
            assert!(
                validate_rest_origin(http_config(origin, demo).rest_url(), demo).is_ok(),
                "expected {origin} to be allowed"
            );
        }

        for (origin, demo) in [
            ("https://credentials.example", true),
            ("http://openapi.okx.com", true),
            ("https://user@openapi.okx.com", true),
            ("https://openapi.okx.com/api/v5", true),
            ("https://openapi.okx.com?redirect=1", true),
            ("http://127.0.0.1:18080", false),
            ("http://localhost", true),
        ] {
            assert!(
                validate_rest_origin(http_config(origin, demo).rest_url(), demo).is_err(),
                "expected {origin} to be rejected"
            );
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RecordedRequest {
        method: WireMethod,
        path: String,
        body: String,
    }

    #[derive(Clone)]
    struct MockTransport {
        responses: Arc<Mutex<VecDeque<WireResponse>>>,
        requests: Arc<Mutex<Vec<RecordedRequest>>>,
    }

    #[async_trait]
    impl WireTransport for MockTransport {
        async fn execute(&self, request: WireRequest) -> Result<WireResponse, WireError> {
            self.requests.lock().unwrap().push(RecordedRequest {
                method: request.method(),
                path: request.path().to_string(),
                body: request.body().to_string(),
            });
            Ok(self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("mock response for every allowed request"))
        }
    }

    fn client(
        responses: impl IntoIterator<Item = &'static str>,
    ) -> (
        EvidenceClient<MockTransport>,
        Arc<Mutex<Vec<RecordedRequest>>>,
    ) {
        client_with_responses(
            responses
                .into_iter()
                .map(|body| WireResponse::new(200, body.as_bytes().to_vec())),
        )
    }

    fn client_with_responses(
        responses: impl IntoIterator<Item = WireResponse>,
    ) -> (
        EvidenceClient<MockTransport>,
        Arc<Mutex<Vec<RecordedRequest>>>,
    ) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let transport = MockTransport {
            responses: Arc::new(Mutex::new(responses.into_iter().collect())),
            requests: Arc::clone(&requests),
        };
        let credentials = WireCredentials::new("key", "secret", "pass").unwrap();
        (
            EvidenceClient {
                wire: WireClient::new(transport, credentials, true),
            },
            requests,
        )
    }

    #[tokio::test]
    async fn exact_evidence_allowlist_is_get_only_and_parses_every_response() {
        let account_config = r#"{"code":"0","msg":"","data":[{"acctLv":"2","posMode":"net_mode","acctStpMode":"cancel_maker","uid":"7","mainUid":"6","label":"reap-demo","perm":"trade,read_only","ip":"203.0.113.5","enableSpotBorrow":false,"autoLoan":false,"spotBorrowAutoRepay":false}]}"#;
        let account_balance = r#"{"code":"0","msg":"","data":[{"uTime":"1000","totalEq":"11000","details":[{"ccy":"USDT","uTime":"999","cashBal":"9000","availBal":"8000","eq":"10000","liab":"0"}]}]}"#;
        let index = r#"{"code":"0","msg":"","data":[{"instId":"BTC-USD","idxPx":"50000.25","ts":"1597026383085"}]}"#;
        let order = r#"{"code":"0","msg":"","data":[{"ordId":"123","clOrdId":"reap1","instId":"BTC-USDT","side":"buy","state":"canceled","px":"100","sz":"1","accFillSz":"0","avgPx":"","uTime":"1000","cancelSource":"20","cancelSourceReason":"Cancel all after triggered"}]}"#;
        let (client, requests) = client([
            r#"{"code":"0","msg":"","data":[{"ts":"1597026383085"}]}"#,
            account_config,
            account_balance,
            r#"{"code":"0","msg":"","data":[]}"#,
            index,
            r#"{"code":"0","msg":"","data":[]}"#,
            r#"{"code":"0","msg":"","data":[]}"#,
            order,
            r#"{"code":"0","msg":"","data":[]}"#,
        ]);

        assert_eq!(client.server_time_ms().await.unwrap(), 1_597_026_383_085);
        assert_eq!(
            client.account_config().await.unwrap().response_body(),
            account_config
        );
        assert_eq!(
            client.account_balance().await.unwrap().response_body(),
            account_balance
        );
        client.account_positions().await.unwrap();
        assert_eq!(
            client
                .index_ticker("BTC-USD")
                .await
                .unwrap()
                .response_body(),
            index
        );
        client.recent_fills_page(Some("fill cursor")).await.unwrap();
        client
            .account_bills_page(1_000, 2_000, Some("bill cursor"))
            .await
            .unwrap();
        assert_eq!(
            client
                .regular_order_details("BTC-USDT", "123", "reap1")
                .await
                .unwrap()
                .response_body(),
            order
        );
        client.regular_open_orders().await.unwrap();

        let requests = requests.lock().unwrap();
        let paths = requests
            .iter()
            .map(|request| request.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            [
                "/api/v5/public/time",
                "/api/v5/account/config",
                "/api/v5/account/balance",
                "/api/v5/account/positions",
                "/api/v5/market/index-tickers?instId=BTC-USD",
                "/api/v5/trade/fills?after=fill+cursor&limit=100",
                "/api/v5/account/bills?begin=1000&end=2000&after=bill+cursor&limit=100",
                "/api/v5/trade/order?instId=BTC-USDT&ordId=123&clOrdId=reap1",
                "/api/v5/trade/orders-pending",
            ]
        );
        assert!(
            requests
                .iter()
                .all(|request| request.method == WireMethod::Get)
        );
        assert!(requests.iter().all(|request| request.body.is_empty()));
        for request in requests.iter() {
            let base = request.path.split('?').next().unwrap();
            assert!(OKX_EVIDENCE_ENDPOINT_ALLOWLIST.contains(&base));
        }
    }

    #[tokio::test]
    async fn request_validation_fails_before_the_private_wire() {
        let (client, requests) = client([]);
        assert!(matches!(
            client.index_ticker("   ").await,
            Err(EvidenceReadError::InvalidField {
                field: "instId",
                ..
            })
        ));
        assert!(matches!(
            client.account_bills_page(0, 2_000, None).await,
            Err(EvidenceReadError::InvalidField {
                field: "begin/end",
                ..
            })
        ));
        assert!(requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn parser_failures_are_preserved_after_the_exact_allowed_read() {
        let (client, requests) =
            client([r#"{"code":"0","msg":"","data":[{"instId":"ETH-USD","idxPx":"1","ts":"1"}]}"#]);
        assert!(matches!(
            client.index_ticker("BTC-USD").await,
            Err(EvidenceReadError::InvalidField {
                field: "instId",
                ..
            })
        ));
        assert_eq!(
            requests.lock().unwrap()[0].path,
            "/api/v5/market/index-tickers?instId=BTC-USD"
        );
    }

    #[tokio::test]
    async fn non_success_status_preserves_the_existing_transport_error() {
        let (client, requests) =
            client_with_responses([WireResponse::new(429, b"rate limited".to_vec())]);
        assert_eq!(
            client.server_time_ms().await.unwrap_err(),
            EvidenceReadError::Transport("HTTP status 429: rate limited".to_string())
        );
        assert_eq!(requests.lock().unwrap()[0].path, OKX_PUBLIC_TIME_ENDPOINT);
    }

    #[test]
    fn credential_lookup_is_ordered_and_stops_at_the_first_failure() {
        let environment =
            EvidenceCredentialEnvironment::new("main", "API_KEY", "SECRET", "PASSPHRASE");
        let names = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&names);
        let error = load_credentials_with(&environment, move |name| {
            observed.lock().unwrap().push(name.to_string());
            if name == "SECRET" {
                Err(())
            } else {
                Ok(format!("value-for-{name}"))
            }
        })
        .err()
        .unwrap();
        assert_eq!(
            error,
            EvidenceClientFactoryError::MissingCredential {
                account_id: "main".to_string(),
                name: "SECRET".to_string(),
            }
        );
        assert_eq!(*names.lock().unwrap(), ["API_KEY", "SECRET"]);
    }
}
