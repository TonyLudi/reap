use std::fmt;

use reap_pm_core::PmTokenId;
use reap_polymarket_auth::{AuthenticatedL2Headers, EoaAddress, FixedOrderId, L2HeaderSink};
use reap_polymarket_wire::{MAX_PM_LIVE_BODY_BYTES, PmWireScope};
use reqwest::{
    Client, RequestBuilder, StatusCode, Url,
    header::{ACCEPT, HeaderName, HeaderValue},
    redirect::Policy,
};
use zeroize::Zeroizing;

use crate::{
    PmAccountHttpRole, PmLiveAdapterError, PmPrivateHttpConfig, PmReadOnlySignatureType,
    PmReconciliationHttpRole, config::OriginMode, private_credentials::PmHttpCredentialRole,
};

const FIRST_PAGE_CURSOR: &str = "MA==";
const POLY_ADDRESS: HeaderName = HeaderName::from_static("poly_address");
const POLY_SIGNATURE: HeaderName = HeaderName::from_static("poly_signature");
const POLY_TIMESTAMP: HeaderName = HeaderName::from_static("poly_timestamp");
const POLY_API_KEY: HeaderName = HeaderName::from_static("poly_api_key");
const POLY_PASSPHRASE: HeaderName = HeaderName::from_static("poly_passphrase");

pub(crate) enum PmPrivateRoute<'a> {
    OpenOrders {
        cursor: &'a str,
    },
    Trades {
        cursor: &'a str,
    },
    ExactOrder(FixedOrderId),
    CollateralBalanceAllowance(PmReadOnlySignatureType),
    ConditionalBalanceAllowance {
        token: PmTokenId,
        signature_type: PmReadOnlySignatureType,
    },
}

impl PmPrivateRoute<'_> {
    const fn accepts_not_found(&self) -> bool {
        matches!(self, Self::ExactOrder(_))
    }
}

pub(crate) enum PmPrivateHttpObservation {
    Found(Zeroizing<Vec<u8>>),
    NotFound,
}

pub(crate) struct PmPrivateHttpTransport {
    client: Client,
    origin: Url,
    configured_address: EoaAddress,
}

impl PmPrivateHttpTransport {
    pub(crate) fn new(
        config: &PmPrivateHttpConfig,
        configured_address: EoaAddress,
    ) -> Result<Self, PmLiveAdapterError> {
        Self::build(
            config.origin().clone(),
            config.connect_timeout(),
            config.request_timeout(),
            config.mode(),
            configured_address,
        )
    }

    pub(crate) fn for_account(
        config: &crate::PmPublicHttpConfig,
        configured_address: EoaAddress,
    ) -> Result<Self, PmLiveAdapterError> {
        Self::build(
            config.origin().clone(),
            config.connect_timeout(),
            config.request_timeout(),
            config.mode(),
            configured_address,
        )
    }

    fn build(
        origin: Url,
        connect_timeout: std::time::Duration,
        request_timeout: std::time::Duration,
        mode: OriginMode,
        configured_address: EoaAddress,
    ) -> Result<Self, PmLiveAdapterError> {
        let mut builder = Client::builder()
            .connect_timeout(connect_timeout)
            .timeout(request_timeout)
            .redirect(Policy::none())
            .no_proxy();
        if mode == OriginMode::Production {
            builder = builder.https_only(true);
        }
        let client = builder
            .build()
            .map_err(|_| PmLiveAdapterError::TransportBuild)?;
        Ok(Self {
            client,
            origin,
            configured_address,
        })
    }

    pub(crate) async fn get(
        &self,
        route: PmPrivateRoute<'_>,
        authenticated: AuthenticatedL2Headers,
    ) -> Result<PmPrivateHttpObservation, PmLiveAdapterError> {
        let accepts_not_found = route.accepts_not_found();
        let url = self.route_url(route);
        let request = self.client.get(url).header(ACCEPT, "application/json");
        let mut sink = FixedReadHeaderSink::new(request, self.configured_address);
        authenticated.apply_to(&mut sink)?;
        let mut response = sink.finish()?.send().await.map_err(map_request_error)?;
        let status = response.status();
        if status.is_redirection() {
            return Err(PmLiveAdapterError::Redirect {
                status: status.as_u16(),
            });
        }
        if status == StatusCode::NOT_FOUND && accepts_not_found {
            return Ok(PmPrivateHttpObservation::NotFound);
        }
        if status != StatusCode::OK {
            return Err(PmLiveAdapterError::UnexpectedStatus {
                status: status.as_u16(),
            });
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_PM_LIVE_BODY_BYTES as u64)
        {
            return Err(PmLiveAdapterError::ResponseBodyTooLarge {
                limit: MAX_PM_LIVE_BODY_BYTES,
            });
        }

        let capacity = response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or(0)
            .min(MAX_PM_LIVE_BODY_BYTES);
        let mut body = Zeroizing::new(Vec::with_capacity(capacity));
        while let Some(chunk) = response.chunk().await.map_err(map_body_error)? {
            let next_length = body.len().checked_add(chunk.len()).ok_or(
                PmLiveAdapterError::ResponseBodyTooLarge {
                    limit: MAX_PM_LIVE_BODY_BYTES,
                },
            )?;
            if next_length > MAX_PM_LIVE_BODY_BYTES {
                return Err(PmLiveAdapterError::ResponseBodyTooLarge {
                    limit: MAX_PM_LIVE_BODY_BYTES,
                });
            }
            body.extend_from_slice(&chunk);
        }
        Ok(PmPrivateHttpObservation::Found(body))
    }

    fn route_url(&self, route: PmPrivateRoute<'_>) -> Url {
        let mut url = self.origin.clone();
        match route {
            PmPrivateRoute::OpenOrders { cursor } => {
                url.set_path("/data/orders");
                url.query_pairs_mut().append_pair("next_cursor", cursor);
            }
            PmPrivateRoute::Trades { cursor } => {
                url.set_path("/data/trades");
                url.query_pairs_mut().append_pair("next_cursor", cursor);
            }
            PmPrivateRoute::ExactOrder(order_id) => {
                // clob-client-v2's fixed endpoint constant remains
                // `/data/order/`; some generated docs currently drift to a
                // plural spelling. Authentication and transport intentionally
                // pin the client endpoint byte-for-byte.
                url.set_path(&format!("/data/order/{order_id}"));
            }
            PmPrivateRoute::CollateralBalanceAllowance(signature_type) => {
                url.set_path("/balance-allowance");
                url.query_pairs_mut()
                    .append_pair("asset_type", "COLLATERAL")
                    .append_pair("signature_type", signature_type.query_value());
            }
            PmPrivateRoute::ConditionalBalanceAllowance {
                token,
                signature_type,
            } => {
                url.set_path("/balance-allowance");
                url.query_pairs_mut()
                    .append_pair("asset_type", "CONDITIONAL")
                    .append_pair("token_id", &token.units().to_string())
                    .append_pair("signature_type", signature_type.query_value());
            }
        }
        url
    }
}

/// Move-only custody for one exact authenticated account transport.
///
/// The owner lends mutually exclusive, short-lived read capabilities. It has
/// no generic request API and cannot authorize a mutation.
pub struct PmAuthenticatedHttpOwner {
    authority: PmHttpCredentialRole,
    transport: PmPrivateHttpTransport,
    exact_order_scope: PmWireScope,
    configured_address: EoaAddress,
    balance_signature_type: PmReadOnlySignatureType,
}

impl PmAuthenticatedHttpOwner {
    pub(crate) const fn from_authority(
        transport: PmPrivateHttpTransport,
        exact_order_scope: PmWireScope,
        configured_address: EoaAddress,
        balance_signature_type: PmReadOnlySignatureType,
        authority: PmHttpCredentialRole,
    ) -> Self {
        Self {
            authority,
            transport,
            exact_order_scope,
            configured_address,
            balance_signature_type,
        }
    }

    /// Direct credential construction exists only for in-crate loopback
    /// evidence. Production must split [`crate::PmPrivateConnectivityOwner`].
    #[cfg(test)]
    pub fn new(
        config: PmPrivateHttpConfig,
        credentials: reap_polymarket_auth::L2Credentials,
    ) -> Result<(Self, crate::PmCredentialAuthoritySupervisor), PmLiveAdapterError> {
        Self::new_with_signature_type(config, PmReadOnlySignatureType::Eoa, credentials)
    }

    #[cfg(test)]
    pub(crate) fn new_with_signature_type(
        config: PmPrivateHttpConfig,
        balance_signature_type: PmReadOnlySignatureType,
        credentials: reap_polymarket_auth::L2Credentials,
    ) -> Result<(Self, crate::PmCredentialAuthoritySupervisor), PmLiveAdapterError> {
        let exact_order_scope = config.exact_order_scope();
        let configured_address = credentials.address();
        let transport = PmPrivateHttpTransport::new(&config, configured_address)?;
        let (authority, supervisor) =
            crate::private_credentials::test_http_credential_role(credentials)?;
        Ok((
            Self::from_authority(
                transport,
                exact_order_scope,
                configured_address,
                balance_signature_type,
                authority,
            ),
            supervisor,
        ))
    }

    pub fn reconciliation(&mut self) -> PmReconciliationHttpRole<'_> {
        PmReconciliationHttpRole::new(
            &mut self.authority,
            &self.transport,
            self.exact_order_scope,
            self.configured_address,
        )
    }

    pub fn account(&mut self) -> PmAccountHttpRole<'_> {
        PmAccountHttpRole::new(
            &mut self.authority,
            &self.transport,
            self.exact_order_scope.token(),
            self.balance_signature_type,
        )
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }
}

impl fmt::Debug for PmAuthenticatedHttpOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmAuthenticatedHttpOwner([REDACTED])")
    }
}

struct FixedReadHeaderSink {
    request: Option<RequestBuilder>,
    configured_address: EoaAddress,
}

impl FixedReadHeaderSink {
    const fn new(request: RequestBuilder, configured_address: EoaAddress) -> Self {
        Self {
            request: Some(request),
            configured_address,
        }
    }

    fn finish(mut self) -> Result<RequestBuilder, PmLiveAdapterError> {
        self.request
            .take()
            .ok_or(PmLiveAdapterError::InvalidAuthenticatedHeaders)
    }
}

impl L2HeaderSink for FixedReadHeaderSink {
    type Error = PmLiveAdapterError;

    fn set_polymarket_l2_headers(
        &mut self,
        poly_address: &str,
        poly_signature: &str,
        poly_timestamp: &str,
        poly_api_key: &str,
        poly_passphrase: &str,
    ) -> Result<(), Self::Error> {
        if poly_address != self.configured_address.to_string() {
            return Err(PmLiveAdapterError::InvalidAuthenticatedHeaders);
        }
        let request = self
            .request
            .take()
            .ok_or(PmLiveAdapterError::InvalidAuthenticatedHeaders)?;
        self.request = Some(
            request
                .header(POLY_ADDRESS, header_value(poly_address)?)
                .header(POLY_SIGNATURE, header_value(poly_signature)?)
                .header(POLY_TIMESTAMP, header_value(poly_timestamp)?)
                .header(POLY_API_KEY, header_value(poly_api_key)?)
                .header(POLY_PASSPHRASE, header_value(poly_passphrase)?),
        );
        Ok(())
    }
}

fn header_value(value: &str) -> Result<HeaderValue, PmLiveAdapterError> {
    let mut value = HeaderValue::from_str(value)
        .map_err(|_| PmLiveAdapterError::InvalidAuthenticatedHeaders)?;
    value.set_sensitive(true);
    Ok(value)
}

pub(crate) const fn first_page_cursor() -> &'static str {
    FIRST_PAGE_CURSOR
}

fn map_request_error(error: reqwest::Error) -> PmLiveAdapterError {
    if error.is_timeout() {
        PmLiveAdapterError::RequestTimeout
    } else {
        PmLiveAdapterError::RequestFailed
    }
}

fn map_body_error(error: reqwest::Error) -> PmLiveAdapterError {
    if error.is_timeout() {
        PmLiveAdapterError::RequestTimeout
    } else {
        PmLiveAdapterError::ResponseBodyRead
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use reap_pm_core::{PmConditionId, PmMarketId, PmTokenId, U256};
    use reap_polymarket_auth::{L2CredentialInput, L2Credentials};
    use reap_polymarket_wire::PmWireScope;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::mpsc,
        task::JoinHandle,
        time::sleep,
    };
    use zeroize::Zeroize;

    use super::*;
    use crate::{PmExactOrderObservation, PmPrivateHttpConfig};

    const ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    const FOREIGN_MAKER: &str = "0x2222222222222222222222222222222222222222";
    const API_KEY: &str = "00000000-0000-4000-8000-000000000001";
    const FOREIGN_API_KEY: &str = "00000000-0000-4000-8000-000000000002";
    const SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    const PASSPHRASE: &str = "synthetic-passphrase";
    const AUTH_SECONDS: u64 = 1_780_449_126;
    const CONDITION: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
    const FOREIGN_CONDITION: &str =
        "0x4444444444444444444444444444444444444444444444444444444444444444";
    const QUESTION: &str = "0x9999999999999999999999999999999999999999999999999999999999999999";
    const ORDER_ID: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OTHER_ORDER_ID: &str =
        "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const SPENDER: &str = "0x3333333333333333333333333333333333333333";

    struct MockResponse {
        status: u16,
        body: Vec<u8>,
        delay: Duration,
        location: Option<&'static str>,
        content_length: Option<usize>,
    }

    impl MockResponse {
        fn ok(body: impl Into<Vec<u8>>) -> Self {
            let body = body.into();
            Self {
                status: 200,
                content_length: Some(body.len()),
                body,
                delay: Duration::ZERO,
                location: None,
            }
        }

        fn status(status: u16) -> Self {
            Self {
                status,
                body: b"{}".to_vec(),
                delay: Duration::ZERO,
                location: None,
                content_length: Some(2),
            }
        }
    }

    async fn mock_server(
        responses: Vec<MockResponse>,
    ) -> (String, mpsc::UnboundedReceiver<String>, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (requests_tx, requests_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            for response in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut raw = Vec::new();
                let mut chunk = [0_u8; 1024];
                loop {
                    let read = stream.read(&mut chunk).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    raw.extend_from_slice(&chunk[..read]);
                    if raw.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8(raw).unwrap();
                let _ = requests_tx.send(request);
                sleep(response.delay).await;
                let reason = match response.status {
                    200 => "OK",
                    302 => "Found",
                    401 => "Unauthorized",
                    404 => "Not Found",
                    503 => "Service Unavailable",
                    _ => "Mock",
                };
                let mut headers = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nConnection: close\r\n",
                    response.status, reason
                );
                if let Some(length) = response.content_length {
                    headers.push_str(&format!("Content-Length: {length}\r\n"));
                }
                if let Some(location) = response.location {
                    headers.push_str(&format!("Location: {location}\r\n"));
                }
                headers.push_str("\r\n");
                if stream.write_all(headers.as_bytes()).await.is_ok() {
                    let _ = stream.write_all(&response.body).await;
                }
            }
        });
        (format!("http://{address}"), requests_rx, task)
    }

    fn scope() -> PmWireScope {
        PmWireScope::new(
            PmConditionId::parse(CONDITION).unwrap(),
            PmMarketId::parse(QUESTION).unwrap(),
            PmTokenId::new(U256::from_u64(123)).unwrap(),
        )
    }

    fn credentials() -> L2Credentials {
        L2Credentials::bind(
            ADDRESS,
            L2CredentialInput::new(API_KEY.into(), SECRET.into(), PASSPHRASE.into()),
        )
        .unwrap()
    }

    fn local_owner(
        origin: &str,
        request_timeout: Duration,
    ) -> (
        crate::PmAuthenticatedHttpOwner,
        crate::PmCredentialAuthoritySupervisor,
    ) {
        let config = PmPrivateHttpConfig::local_evidence(
            origin,
            Duration::from_millis(100),
            request_timeout,
            scope(),
        )
        .unwrap();
        crate::PmAuthenticatedHttpOwner::new(config, credentials()).unwrap()
    }

    fn local_owner_for_signature_type(
        origin: &str,
        request_timeout: Duration,
        signature_type: PmReadOnlySignatureType,
    ) -> (
        crate::PmAuthenticatedHttpOwner,
        crate::PmCredentialAuthoritySupervisor,
    ) {
        let config = PmPrivateHttpConfig::local_evidence(
            origin,
            Duration::from_millis(100),
            request_timeout,
            scope(),
        )
        .unwrap();
        crate::PmAuthenticatedHttpOwner::new_with_signature_type(
            config,
            signature_type,
            credentials(),
        )
        .unwrap()
    }

    fn order(id: &str, condition: &str, token: &str, maker: &str, owner: &str) -> String {
        format!(
            r#"{{"id":"{id}","market":"{condition}","asset_id":"{token}","side":"BUY","original_size":"10.000000","size_matched":"0","price":"0.420000","status":"LIVE","maker_address":"{maker}","owner":"{owner}","expiration":"0","created_at":1700000000}}"#
        )
    }

    fn trade(condition: &str, token: &str, maker: &str, owner: &str) -> String {
        format!(
            r#"{{"id":"trade-1","market":"{condition}","asset_id":"{token}","side":"SELL","size":"2.500000","price":"0.420000","status":"CONFIRMED","match_time":"1700000000002","last_update":"1700000000003","order_id":"{ORDER_ID}","maker_orders":[],"maker_address":"{maker}","owner":"{owner}"}}"#
        )
    }

    fn page(item: &str, cursor: &str) -> String {
        format!(r#"{{"data":[{item}],"next_cursor":"{cursor}","limit":128,"count":1}}"#)
    }

    fn empty_page(cursor: &str) -> String {
        format!(r#"{{"data":[],"next_cursor":"{cursor}","limit":128,"count":0}}"#)
    }

    fn balance() -> String {
        format!(r#"{{"balance":"1000","allowances":{{"{SPENDER}":"900"}}}}"#)
    }

    fn timestamp() -> crate::PmReadServerTime {
        crate::product_clock::test_support_read_server_time(AUTH_SECONDS)
    }

    #[tokio::test]
    async fn exact_routes_headers_pagination_and_account_wide_retention() {
        let foreign_order = order(ORDER_ID, FOREIGN_CONDITION, "999", FOREIGN_MAKER, API_KEY);
        let foreign_trade = trade(FOREIGN_CONDITION, "999", FOREIGN_MAKER, API_KEY);
        let responses = vec![
            MockResponse::ok(page(&foreign_order, "cursor+/=")),
            MockResponse::ok(empty_page("LTE=")),
            MockResponse::ok(page(&foreign_trade, "LTE=")),
            MockResponse::status(404),
            MockResponse::ok(balance()),
            MockResponse::ok(balance()),
        ];
        let (origin, mut requests, task) = mock_server(responses).await;
        let (mut owner, supervisor) = local_owner(&origin, Duration::from_secs(1));

        let mut reconciliation = owner.reconciliation();
        let first = reconciliation.begin_open_orders(timestamp()).await.unwrap();
        let crate::PmOpenOrdersCutProgress::Incomplete(incomplete) = first else {
            panic!("first cursor must remain incomplete")
        };
        assert_eq!(incomplete.pages_received(), 1);
        let terminal = reconciliation
            .continue_open_orders(timestamp(), incomplete)
            .await
            .unwrap();
        let crate::PmOpenOrdersCutProgress::Complete(terminal) = terminal else {
            panic!("terminal cursor must complete cut")
        };
        let observed = &terminal.pages()[0].orders()[0];
        assert_eq!(observed.condition().to_string(), FOREIGN_CONDITION);
        assert_eq!(observed.token().units(), U256::from_u64(999));
        assert_eq!(observed.maker().to_string(), FOREIGN_MAKER);
        let trades = reconciliation.begin_trades(timestamp()).await.unwrap();
        let crate::PmTradesCutProgress::Complete(trades) = trades else {
            panic!("terminal trade cursor must complete cut")
        };
        assert_eq!(
            trades.pages()[0].trades()[0].condition().to_string(),
            FOREIGN_CONDITION
        );
        assert_eq!(
            trades.pages()[0].trades()[0].maker().unwrap().to_string(),
            FOREIGN_MAKER
        );
        let absent = reconciliation
            .exact_local_order_detail(timestamp(), FixedOrderId::parse(ORDER_ID).unwrap())
            .await
            .unwrap();
        assert_eq!(absent, PmExactOrderObservation::Absent);
        let mut account = owner.account();
        let collateral = account
            .collateral_balance_allowance(timestamp())
            .await
            .unwrap();
        assert_eq!(
            collateral.exact_allowance(reap_pm_core::EvmAddress::parse(SPENDER).unwrap()),
            Some(U256::from_u64(900))
        );
        let conditional = account
            .conditional_balance_allowance(timestamp())
            .await
            .unwrap();
        assert_eq!(
            conditional.asset(),
            crate::PmAccountAsset::Conditional(scope().token())
        );

        let mut captured = Vec::new();
        for _ in 0..6 {
            captured.push(requests.recv().await.unwrap());
        }
        let request_lines = captured
            .iter()
            .map(|request| request.lines().next().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            request_lines,
            [
                "GET /data/orders?next_cursor=MA%3D%3D HTTP/1.1",
                "GET /data/orders?next_cursor=cursor%2B%2F%3D HTTP/1.1",
                "GET /data/trades?next_cursor=MA%3D%3D HTTP/1.1",
                &format!("GET /data/order/{ORDER_ID} HTTP/1.1"),
                "GET /balance-allowance?asset_type=COLLATERAL&signature_type=0 HTTP/1.1",
                "GET /balance-allowance?asset_type=CONDITIONAL&token_id=123&signature_type=0 HTTP/1.1",
            ]
        );
        for request in &captured[..3] {
            let first_line = request.lines().next().unwrap();
            assert!(!first_line.contains("market="));
            assert!(!first_line.contains("asset_id="));
            assert!(!first_line.contains("after="));
        }
        let first_headers = captured[0].to_ascii_lowercase();
        assert!(first_headers.contains(&format!("poly_address: {}", ADDRESS.to_ascii_lowercase())));
        assert!(
            first_headers.contains("poly_signature: -prhfdru6jmzz04syaatdprblz8zwfpynigpmfrqvee=")
        );
        assert!(
            captured[1]
                .to_ascii_lowercase()
                .contains("poly_signature: -prhfdru6jmzz04syaatdprblz8zwfpynigpmfrqvee=")
        );
        assert!(first_headers.contains(&format!("poly_timestamp: {AUTH_SECONDS}")));
        assert!(first_headers.contains(&format!("poly_api_key: {API_KEY}")));
        assert!(first_headers.contains(&format!("poly_passphrase: {PASSPHRASE}")));
        task.await.unwrap();
        supervisor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn proxy_account_reads_use_signature_type_one_and_keep_signer_headers() {
        let (origin, mut requests, task) = mock_server(vec![
            MockResponse::ok(balance()),
            MockResponse::ok(balance()),
        ])
        .await;
        let (mut owner, supervisor) = local_owner_for_signature_type(
            &origin,
            Duration::from_secs(1),
            PmReadOnlySignatureType::Proxy,
        );

        let mut account = owner.account();
        account
            .collateral_balance_allowance(timestamp())
            .await
            .unwrap();
        account
            .conditional_balance_allowance(timestamp())
            .await
            .unwrap();

        let collateral = requests.recv().await.unwrap();
        let conditional = requests.recv().await.unwrap();
        assert_eq!(
            collateral.lines().next().unwrap(),
            "GET /balance-allowance?asset_type=COLLATERAL&signature_type=1 HTTP/1.1"
        );
        assert_eq!(
            conditional.lines().next().unwrap(),
            "GET /balance-allowance?asset_type=CONDITIONAL&token_id=123&signature_type=1 HTTP/1.1"
        );
        for request in [collateral, conditional] {
            let headers = request.to_ascii_lowercase();
            assert!(headers.contains(&format!("poly_address: {}", ADDRESS.to_ascii_lowercase())));
        }

        task.await.unwrap();
        supervisor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn exact_local_detail_is_strict_and_404_is_typed_absence() {
        let correct = order(ORDER_ID, CONDITION, "123", ADDRESS, API_KEY);
        let wrong_id = order(OTHER_ORDER_ID, CONDITION, "123", ADDRESS, API_KEY);
        let wrong_maker = order(ORDER_ID, CONDITION, "123", FOREIGN_MAKER, API_KEY);
        let wrong_scope = order(ORDER_ID, FOREIGN_CONDITION, "999", ADDRESS, API_KEY);
        let responses = vec![
            MockResponse::ok(correct),
            MockResponse::ok(wrong_id),
            MockResponse::ok(wrong_maker),
            MockResponse::ok(wrong_scope),
            MockResponse::status(404),
        ];
        let (origin, _requests, task) = mock_server(responses).await;
        let (mut owner, supervisor) = local_owner(&origin, Duration::from_secs(1));
        let mut role = owner.reconciliation();
        let id = FixedOrderId::parse(ORDER_ID).unwrap();
        assert!(matches!(
            role.exact_local_order_detail(timestamp(), id)
                .await
                .unwrap(),
            PmExactOrderObservation::Present(_)
        ));
        assert_eq!(
            role.exact_local_order_detail(timestamp(), id).await,
            Err(PmLiveAdapterError::ExactOrderIdentityMismatch)
        );
        assert_eq!(
            role.exact_local_order_detail(timestamp(), id).await,
            Err(PmLiveAdapterError::ExactOrderMakerMismatch)
        );
        assert_eq!(
            role.exact_local_order_detail(timestamp(), id).await,
            Err(PmLiveAdapterError::ExactOrderScopeMismatch)
        );
        assert_eq!(
            role.exact_local_order_detail(timestamp(), id).await,
            Ok(PmExactOrderObservation::Absent)
        );
        task.await.unwrap();
        supervisor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn owner_malformed_status_and_redirect_fail_closed() {
        let foreign_owner = order(ORDER_ID, CONDITION, "123", ADDRESS, FOREIGN_API_KEY);
        let mut redirect = MockResponse::status(302);
        redirect.location = Some("/data/orders");
        let responses = vec![
            MockResponse::ok(page(&foreign_owner, "LTE=")),
            MockResponse::ok(b"{".to_vec()),
            MockResponse::status(401),
            MockResponse::status(503),
            redirect,
        ];
        let (origin, _requests, task) = mock_server(responses).await;
        let (mut owner, supervisor) = local_owner(&origin, Duration::from_secs(1));
        let mut role = owner.reconciliation();
        assert!(matches!(
            role.begin_open_orders(timestamp()).await,
            Err(PmLiveAdapterError::CredentialOwnerMismatch)
        ));
        assert!(matches!(
            role.begin_open_orders(timestamp()).await,
            Err(PmLiveAdapterError::PrivateWire(_))
        ));
        assert!(matches!(
            role.begin_open_orders(timestamp()).await,
            Err(PmLiveAdapterError::UnexpectedStatus { status: 401 })
        ));
        assert!(matches!(
            role.begin_open_orders(timestamp()).await,
            Err(PmLiveAdapterError::UnexpectedStatus { status: 503 })
        ));
        assert!(matches!(
            role.begin_open_orders(timestamp()).await,
            Err(PmLiveAdapterError::Redirect { status: 302 })
        ));
        task.await.unwrap();
        supervisor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn timeout_and_both_body_oversize_modes_fail_closed() {
        let advertised = MockResponse {
            status: 200,
            body: Vec::new(),
            delay: Duration::ZERO,
            location: None,
            content_length: Some(MAX_PM_LIVE_BODY_BYTES + 1),
        };
        let streamed = MockResponse {
            status: 200,
            body: vec![b'x'; MAX_PM_LIVE_BODY_BYTES + 1],
            delay: Duration::ZERO,
            location: None,
            content_length: None,
        };
        let delayed = MockResponse {
            status: 200,
            body: empty_page("LTE=").into_bytes(),
            delay: Duration::from_millis(200),
            location: None,
            content_length: None,
        };
        let (origin, _requests, task) = mock_server(vec![advertised, streamed, delayed]).await;
        let (mut owner, supervisor) = local_owner(&origin, Duration::from_millis(40));
        let mut role = owner.reconciliation();
        for _ in 0..2 {
            assert!(matches!(
                role.begin_open_orders(timestamp()).await,
                Err(PmLiveAdapterError::ResponseBodyTooLarge {
                    limit: MAX_PM_LIVE_BODY_BYTES
                })
            ));
        }
        assert!(matches!(
            role.begin_open_orders(timestamp()).await,
            Err(PmLiveAdapterError::RequestTimeout)
        ));
        task.await.unwrap();
        supervisor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn private_configuration_and_debug_are_redacted() {
        assert!(
            PmPrivateHttpConfig::production(
                Duration::from_millis(100),
                Duration::from_secs(1),
                scope()
            )
            .is_ok()
        );
        assert!(
            PmPrivateHttpConfig::local_evidence(
                "http://localhost:18080",
                Duration::from_millis(100),
                Duration::from_secs(1),
                scope()
            )
            .is_err()
        );

        let config = PmPrivateHttpConfig::local_evidence(
            "http://127.0.0.1:18080",
            Duration::from_millis(100),
            Duration::from_secs(1),
            scope(),
        )
        .unwrap();
        let (owner, supervisor) =
            crate::PmAuthenticatedHttpOwner::new(config, credentials()).unwrap();
        let debug = format!("{owner:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains(API_KEY));
        assert!(!debug.contains(PASSPHRASE));
        assert!(!owner.production_order_entry_authorized());
        supervisor.shutdown().await.unwrap();
    }

    #[test]
    fn authenticated_raw_body_carrier_zeroizes_credential_owner_bytes() {
        let observation =
            PmPrivateHttpObservation::Found(Zeroizing::new(API_KEY.as_bytes().to_vec()));
        let PmPrivateHttpObservation::Found(mut body) = observation else {
            panic!("synthetic private body")
        };
        assert!(
            body.windows(API_KEY.len())
                .any(|window| window == API_KEY.as_bytes())
        );
        body.zeroize();
        assert!(body.is_empty());
    }
}
