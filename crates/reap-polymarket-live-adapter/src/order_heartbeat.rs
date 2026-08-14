//! Credential-wide Polymarket order-heartbeat safety role.
//!
//! The current official V2 clients use `POST /v1/heartbeats` every five
//! seconds. A successful initial request sends an empty identifier and returns
//! the identifier required by later requests. HTTP 400 may return the current
//! identifier; this module admits one immediate correction but does not treat
//! that response as an armed heartbeat. Any other failure is fatal so the
//! venue-side expiry remains the final cancellation backstop.

use std::{fmt, net::SocketAddr, time::Duration};

use async_trait::async_trait;
use reap_polymarket_auth::{AuthenticatedOrderHeartbeatRequest, FixedOrderHeartbeatRequestSink};
use reap_polymarket_wire::{PmOrderHeartbeatId, parse_order_heartbeat_response};
use reqwest::{
    Client, Request, StatusCode, Url,
    header::{ACCEPT, CONNECTION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue},
    redirect::Policy,
};
use thiserror::Error;
use tokio::sync::watch;
use zeroize::Zeroizing;

#[cfg(any(test, feature = "loopback-evidence"))]
use crate::PmLoopbackMutationConfig;
use crate::PmProductionMutationConfig;

pub const PM_ORDER_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const MAX_HEARTBEAT_RESPONSE_BYTES: usize = 4 * 1_024;
const POLY_ADDRESS: HeaderName = HeaderName::from_static("poly_address");
const POLY_SIGNATURE: HeaderName = HeaderName::from_static("poly_signature");
const POLY_TIMESTAMP: HeaderName = HeaderName::from_static("poly_timestamp");
const POLY_API_KEY: HeaderName = HeaderName::from_static("poly_api_key");
const POLY_PASSPHRASE: HeaderName = HeaderName::from_static("poly_passphrase");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmOrderHeartbeatError {
    #[error("Polymarket order-heartbeat HTTP client construction failed")]
    TransportBuild,
    #[error("Polymarket order-heartbeat authenticated request was invalid")]
    InvalidAuthenticatedRequest,
    #[error("Polymarket order-heartbeat request timed out")]
    RequestTimeout,
    #[error("Polymarket order-heartbeat transport failed")]
    TransportFailure,
    #[error("Polymarket order-heartbeat connected peer did not match the fixed peer")]
    ConnectedPeerMismatch,
    #[error("Polymarket order-heartbeat response redirected")]
    Redirect,
    #[error("Polymarket order-heartbeat response exceeded its byte bound")]
    ResponseTooLarge,
    #[error("Polymarket order-heartbeat response body failed")]
    ResponseBodyFailure,
    #[error("Polymarket order-heartbeat response was malformed")]
    MalformedResponse,
    #[error("Polymarket order-heartbeat was rejected with HTTP {0}")]
    Rejected(u16),
    #[error("Polymarket order-heartbeat authority failed")]
    AuthorityFailure,
    #[error("Polymarket order-heartbeat stale-ID correction was rejected")]
    StaleIdCorrectionFailed,
}

/// The only two response shapes the heartbeat loop can continue from.
pub enum PmOrderHeartbeatReply {
    Accepted(PmOrderHeartbeatId),
    StaleIdentifier(PmOrderHeartbeatId),
}

impl fmt::Debug for PmOrderHeartbeatReply {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accepted(_) => formatter.write_str("PmOrderHeartbeatReply::Accepted([REDACTED])"),
            Self::StaleIdentifier(_) => {
                formatter.write_str("PmOrderHeartbeatReply::StaleIdentifier([REDACTED])")
            }
        }
    }
}

/// Purpose-closed production heartbeat transport.
pub struct PmOrderHeartbeatProductionRole {
    edge: OrderHeartbeatHttpEdge,
}

impl PmOrderHeartbeatProductionRole {
    pub fn new(config: PmProductionMutationConfig) -> Result<Self, PmOrderHeartbeatError> {
        Ok(Self {
            edge: OrderHeartbeatHttpEdge::production(config)?,
        })
    }

    pub async fn send(
        &mut self,
        request: AuthenticatedOrderHeartbeatRequest,
    ) -> Result<PmOrderHeartbeatReply, PmOrderHeartbeatError> {
        self.edge.send(request).await
    }
}

impl fmt::Debug for PmOrderHeartbeatProductionRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmOrderHeartbeatProductionRole([FIXED PRODUCTION])")
    }
}

/// Literal-loopback-only heartbeat transport for protocol and failure tests.
#[cfg(any(test, feature = "loopback-evidence"))]
pub struct PmOrderHeartbeatLoopbackRole {
    edge: OrderHeartbeatHttpEdge,
}

#[cfg(any(test, feature = "loopback-evidence"))]
impl PmOrderHeartbeatLoopbackRole {
    pub fn new(config: PmLoopbackMutationConfig) -> Result<Self, PmOrderHeartbeatError> {
        Ok(Self {
            edge: OrderHeartbeatHttpEdge::loopback(config)?,
        })
    }

    pub async fn send(
        &mut self,
        request: AuthenticatedOrderHeartbeatRequest,
    ) -> Result<PmOrderHeartbeatReply, PmOrderHeartbeatError> {
        self.edge.send(request).await
    }
}

/// Credential/time authority used by the continuous heartbeat loop. The
/// implementation receives only the last parser-admitted identifier and must
/// return the fixed authenticated request carrier.
#[async_trait]
pub trait PmOrderHeartbeatAuthority: Send {
    async fn authenticate(
        &mut self,
        previous: Option<PmOrderHeartbeatId>,
    ) -> Result<AuthenticatedOrderHeartbeatRequest, PmOrderHeartbeatError>;
}

#[async_trait]
pub trait PmOrderHeartbeatTransport: Send {
    async fn send_heartbeat(
        &mut self,
        request: AuthenticatedOrderHeartbeatRequest,
    ) -> Result<PmOrderHeartbeatReply, PmOrderHeartbeatError>;
}

#[async_trait]
impl PmOrderHeartbeatTransport for PmOrderHeartbeatProductionRole {
    async fn send_heartbeat(
        &mut self,
        request: AuthenticatedOrderHeartbeatRequest,
    ) -> Result<PmOrderHeartbeatReply, PmOrderHeartbeatError> {
        self.send(request).await
    }
}

#[cfg(any(test, feature = "loopback-evidence"))]
#[async_trait]
impl PmOrderHeartbeatTransport for PmOrderHeartbeatLoopbackRole {
    async fn send_heartbeat(
        &mut self,
        request: AuthenticatedOrderHeartbeatRequest,
    ) -> Result<PmOrderHeartbeatReply, PmOrderHeartbeatError> {
        self.send(request).await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmOrderHeartbeatStop {
    ShutdownRequested,
}

/// Run the five-second deadman refresh loop. The first request is immediate.
/// A stale-ID response is corrected once, immediately, and only the following
/// HTTP 200 is considered accepted. Missed ticks use `Delay`, never bursts.
pub async fn run_pm_order_heartbeat_loop<A, T>(
    mut authority: A,
    mut transport: T,
    mut shutdown: watch::Receiver<bool>,
) -> Result<PmOrderHeartbeatStop, PmOrderHeartbeatError>
where
    A: PmOrderHeartbeatAuthority,
    T: PmOrderHeartbeatTransport,
{
    let mut previous = None;
    let mut interval = tokio::time::interval(PM_ORDER_HEARTBEAT_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(PmOrderHeartbeatStop::ShutdownRequested);
                }
            }
            _ = interval.tick() => {
                let request = authority.authenticate(previous.take()).await?;
                match transport.send_heartbeat(request).await? {
                    PmOrderHeartbeatReply::Accepted(next) => previous = Some(next),
                    PmOrderHeartbeatReply::StaleIdentifier(current) => {
                        let corrected = authority.authenticate(Some(current)).await?;
                        match transport.send_heartbeat(corrected).await? {
                            PmOrderHeartbeatReply::Accepted(next) => previous = Some(next),
                            PmOrderHeartbeatReply::StaleIdentifier(_) => {
                                return Err(PmOrderHeartbeatError::StaleIdCorrectionFailed);
                            }
                        }
                    }
                }
            }
        }
    }
}

struct OrderHeartbeatHttpEdge {
    client: Client,
    origin: Url,
    expected_peer: Option<SocketAddr>,
}

impl OrderHeartbeatHttpEdge {
    #[cfg(any(test, feature = "loopback-evidence"))]
    fn loopback(config: PmLoopbackMutationConfig) -> Result<Self, PmOrderHeartbeatError> {
        let (origin, connect_timeout, request_timeout) = config.into_http_parts();
        let client = Client::builder()
            .connect_timeout(connect_timeout)
            .timeout(request_timeout)
            .retry(reqwest::retry::never())
            .redirect(Policy::none())
            .no_proxy()
            .http1_only()
            .pool_max_idle_per_host(0)
            .build()
            .map_err(|_| PmOrderHeartbeatError::TransportBuild)?;
        Ok(Self {
            client,
            origin,
            expected_peer: None,
        })
    }

    fn production(config: PmProductionMutationConfig) -> Result<Self, PmOrderHeartbeatError> {
        let (origin, connect_timeout, request_timeout, fixed_peer, local_egress) =
            config.into_http_parts();
        let mut builder = Client::builder()
            .connect_timeout(connect_timeout)
            .timeout(request_timeout)
            .retry(reqwest::retry::never())
            .redirect(Policy::none())
            .no_proxy()
            .https_only(true)
            .pool_max_idle_per_host(0)
            .resolve(fixed_peer.dns_name(), fixed_peer.peer_addr());
        #[cfg(target_os = "linux")]
        {
            builder = builder
                .interface(local_egress.interface_name())
                .local_address(local_egress.local_source_ip());
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (builder, local_egress);
            return Err(PmOrderHeartbeatError::TransportBuild);
        }
        let client = builder
            .build()
            .map_err(|_| PmOrderHeartbeatError::TransportBuild)?;
        Ok(Self {
            client,
            origin,
            expected_peer: Some(fixed_peer.peer_addr()),
        })
    }

    async fn send(
        &self,
        authenticated: AuthenticatedOrderHeartbeatRequest,
    ) -> Result<PmOrderHeartbeatReply, PmOrderHeartbeatError> {
        let mut builder = HeartbeatRequestBuilder {
            client: &self.client,
            origin: &self.origin,
        };
        let request = authenticated
            .dispatch(&mut builder)
            .map_err(|_| PmOrderHeartbeatError::InvalidAuthenticatedRequest)?;
        let mut response = self.client.execute(request).await.map_err(|error| {
            if error.is_timeout() {
                PmOrderHeartbeatError::RequestTimeout
            } else {
                PmOrderHeartbeatError::TransportFailure
            }
        })?;
        let status = response.status();
        if self.expected_peer.is_some() && response.remote_addr() != self.expected_peer {
            return Err(PmOrderHeartbeatError::ConnectedPeerMismatch);
        }
        if status.is_redirection() {
            return Err(PmOrderHeartbeatError::Redirect);
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_HEARTBEAT_RESPONSE_BYTES as u64)
        {
            return Err(PmOrderHeartbeatError::ResponseTooLarge);
        }
        let mut body = Zeroizing::new(Vec::new());
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| PmOrderHeartbeatError::ResponseBodyFailure)?
        {
            if body.len().saturating_add(chunk.len()) > MAX_HEARTBEAT_RESPONSE_BYTES {
                return Err(PmOrderHeartbeatError::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        let id = parse_order_heartbeat_response(&body)
            .map_err(|_| PmOrderHeartbeatError::MalformedResponse)?;
        match status {
            StatusCode::OK => Ok(PmOrderHeartbeatReply::Accepted(id)),
            StatusCode::BAD_REQUEST => Ok(PmOrderHeartbeatReply::StaleIdentifier(id)),
            _ => Err(PmOrderHeartbeatError::Rejected(status.as_u16())),
        }
    }
}

struct HeartbeatRequestBuilder<'a> {
    client: &'a Client,
    origin: &'a Url,
}

impl FixedOrderHeartbeatRequestSink for HeartbeatRequestBuilder<'_> {
    type Output = Request;
    type Error = ();

    fn send_order_heartbeat(
        &mut self,
        poly_address: &str,
        poly_signature: &str,
        poly_timestamp: &str,
        poly_api_key: &str,
        poly_passphrase: &str,
        exact_body: &[u8],
    ) -> Result<Self::Output, Self::Error> {
        let mut url = self.origin.clone();
        url.set_path("/v1/heartbeats");
        self.client
            .post(url)
            .headers(fixed_headers(
                poly_address,
                poly_signature,
                poly_timestamp,
                poly_api_key,
                poly_passphrase,
            )?)
            .body(exact_body.to_vec())
            .build()
            .map_err(|_| ())
    }
}

fn fixed_headers(
    poly_address: &str,
    poly_signature: &str,
    poly_timestamp: &str,
    poly_api_key: &str,
    poly_passphrase: &str,
) -> Result<HeaderMap, ()> {
    let mut headers = HeaderMap::with_capacity(8);
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(CONNECTION, HeaderValue::from_static("close"));
    for (name, value) in [
        (POLY_ADDRESS, poly_address),
        (POLY_SIGNATURE, poly_signature),
        (POLY_TIMESTAMP, poly_timestamp),
        (POLY_API_KEY, poly_api_key),
        (POLY_PASSPHRASE, poly_passphrase),
    ] {
        headers.insert(name, HeaderValue::from_str(value).map_err(|_| ())?);
    }
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Arc};

    use reap_polymarket_auth::{L2CredentialInput, L2Credentials, L2Timestamp};
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::TcpListener,
        sync::Mutex,
    };

    use super::*;

    const ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

    fn credentials() -> L2Credentials {
        L2Credentials::bind(
            ADDRESS,
            L2CredentialInput::new(
                "00000000-0000-4000-8000-000000000001".into(),
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into(),
                "synthetic-passphrase".into(),
            ),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn loopback_role_sends_only_exact_v1_heartbeat_and_compact_initial_body() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let server_observed = Arc::clone(&observed);
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0_u8; 1024];
            loop {
                let read = stream.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&chunk[..read]);
                if let Some(header_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let header_end = header_end + 4;
                    let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|value| value.parse::<usize>().ok())
                        })
                        .unwrap();
                    if bytes.len() >= header_end + length {
                        break;
                    }
                }
            }
            *server_observed.lock().await = bytes;
            let body = br#"{"heartbeat_id":"hb-1"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.write_all(body).await.unwrap();
        });

        let config = PmLoopbackMutationConfig::loopback_evidence(
            &format!("http://{address}"),
            Duration::from_secs(1),
            Duration::from_secs(2),
        )
        .unwrap();
        let mut role = PmOrderHeartbeatLoopbackRole::new(config).unwrap();
        let request = credentials()
            .authenticate_initial_order_heartbeat(
                L2Timestamp::from_unix_seconds(1_780_449_126).unwrap(),
            )
            .unwrap();
        let reply = role.send(request).await.unwrap();
        assert!(matches!(reply, PmOrderHeartbeatReply::Accepted(_)));
        server.await.unwrap();

        let bytes = observed.lock().await;
        let request = std::str::from_utf8(&bytes).unwrap();
        assert!(request.starts_with("POST /v1/heartbeats HTTP/1.1\r\n"));
        assert!(request.ends_with(r#"{"heartbeat_id":""}"#));
        for header in [
            "poly_address:",
            "poly_signature:",
            "poly_timestamp:",
            "poly_api_key:",
            "poly_passphrase:",
        ] {
            assert!(request.to_ascii_lowercase().contains(header));
        }
    }

    struct ScriptedAuthority {
        credentials: L2Credentials,
        seen: Arc<Mutex<Vec<Option<String>>>>,
    }

    #[async_trait]
    impl PmOrderHeartbeatAuthority for ScriptedAuthority {
        async fn authenticate(
            &mut self,
            previous: Option<PmOrderHeartbeatId>,
        ) -> Result<AuthenticatedOrderHeartbeatRequest, PmOrderHeartbeatError> {
            self.seen
                .lock()
                .await
                .push(previous.as_ref().map(|value| value.as_str().to_owned()));
            let timestamp = L2Timestamp::from_unix_seconds(1_780_449_126).unwrap();
            previous
                .as_ref()
                .map_or_else(
                    || {
                        self.credentials
                            .authenticate_initial_order_heartbeat(timestamp)
                    },
                    |id| self.credentials.authenticate_order_heartbeat(timestamp, id),
                )
                .map_err(|_| PmOrderHeartbeatError::AuthorityFailure)
        }
    }

    struct ScriptedTransport {
        replies: VecDeque<PmOrderHeartbeatReply>,
        shutdown: watch::Sender<bool>,
    }

    #[async_trait]
    impl PmOrderHeartbeatTransport for ScriptedTransport {
        async fn send_heartbeat(
            &mut self,
            _request: AuthenticatedOrderHeartbeatRequest,
        ) -> Result<PmOrderHeartbeatReply, PmOrderHeartbeatError> {
            let reply = self.replies.pop_front().unwrap();
            if self.replies.is_empty() {
                self.shutdown.send_replace(true);
            }
            Ok(reply)
        }
    }

    #[tokio::test]
    async fn loop_corrects_one_stale_identifier_immediately_then_stops_cleanly() {
        let stale = parse_order_heartbeat_response(br#"{"heartbeat_id":"current"}"#).unwrap();
        let accepted = parse_order_heartbeat_response(br#"{"heartbeat_id":"next"}"#).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let authority = ScriptedAuthority {
            credentials: credentials(),
            seen: Arc::clone(&seen),
        };
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);
        let transport = ScriptedTransport {
            replies: VecDeque::from([
                PmOrderHeartbeatReply::StaleIdentifier(stale),
                PmOrderHeartbeatReply::Accepted(accepted),
            ]),
            shutdown: shutdown_sender,
        };

        assert_eq!(
            run_pm_order_heartbeat_loop(authority, transport, shutdown_receiver)
                .await
                .unwrap(),
            PmOrderHeartbeatStop::ShutdownRequested
        );
        assert_eq!(*seen.lock().await, vec![None, Some("current".to_owned())]);
    }
}
