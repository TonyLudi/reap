//! Low-level authenticated OKX wire access.
//!
//! This crate deliberately exposes transport-shaped GET, POST, and WebSocket
//! login operations rather than exchange endpoint semantics. Role adapters are
//! responsible for endpoint allowlists and are the only intended consumers.
//! Credentials, signatures, and authenticated headers cannot be recovered
//! through the public API.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::{SecondsFormat, Utc};
use hmac::{Hmac, Mac};
use reqwest::header::CONTENT_TYPE;
use serde::Serialize;
use sha2::Sha256;
use thiserror::Error;
use zeroize::Zeroizing;

const OK_ACCESS_KEY: &str = "OK-ACCESS-KEY";
const OK_ACCESS_PASSPHRASE: &str = "OK-ACCESS-PASSPHRASE";
const OK_ACCESS_TIMESTAMP: &str = "OK-ACCESS-TIMESTAMP";
const OK_ACCESS_SIGN: &str = "OK-ACCESS-SIGN";
const OK_SIMULATED_TRADING: &str = "x-simulated-trading";
const JSON_CONTENT_TYPE: &str = "application/json";
const WEBSOCKET_LOGIN_PATH: &str = "/users/self/verify";

type HmacSha256 = Hmac<Sha256>;

/// Failures produced solely by the authenticated wire boundary.
#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid OKX REST base URL")]
    InvalidBaseUrl,
    #[error("invalid origin-form OKX request path")]
    InvalidPath,
    #[error("invalid HMAC signing key")]
    InvalidSigningKey,
    #[error("failed to serialize OKX WebSocket login")]
    LoginSerialization(#[from] serde_json::Error),
    #[error("OKX HTTP transport failed: {0}")]
    Transport(String),
}

/// Opaque OKX credential material.
///
/// The values are zeroized on drop and intentionally have no getters.
pub struct Credentials {
    api_key: Zeroizing<String>,
    secret_key: Zeroizing<String>,
    passphrase: Zeroizing<String>,
}

impl Credentials {
    pub fn new(
        api_key: impl Into<String>,
        secret_key: impl Into<String>,
        passphrase: impl Into<String>,
    ) -> Result<Self, Error> {
        let api_key = api_key.into();
        let secret_key = secret_key.into();
        let passphrase = passphrase.into();

        Ok(Self {
            api_key: Zeroizing::new(api_key),
            secret_key: Zeroizing::new(secret_key),
            passphrase: Zeroizing::new(passphrase),
        })
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("api_key", &"[REDACTED]")
            .field("secret_key", &"[REDACTED]")
            .field("passphrase", &"[REDACTED]")
            .finish()
    }
}

/// HTTP methods admitted by the wire boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

impl Method {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

/// A raw HTTP response returned to an endpoint-specific role adapter.
#[derive(Clone, PartialEq, Eq)]
pub struct Response {
    status: u16,
    body: Vec<u8>,
}

impl Response {
    /// Constructs a response, primarily for role-adapter transport fakes.
    pub fn new(status: u16, body: Vec<u8>) -> Self {
        Self { status, body }
    }

    pub const fn status(&self) -> u16 {
        self.status
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub fn into_body(self) -> Vec<u8> {
        self.body
    }
}

impl fmt::Debug for Response {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Response")
            .field("status", &self.status)
            .field("body", &"[REDACTED]")
            .field("body_len", &self.body.len())
            .finish()
    }
}

struct AuthenticationHeaders {
    api_key: Zeroizing<String>,
    passphrase: Zeroizing<String>,
    timestamp: String,
    signature: Zeroizing<String>,
    demo_trading: bool,
}

fn demo_header(demo_trading: bool) -> Option<(&'static str, &'static str)> {
    demo_trading.then_some((OK_SIMULATED_TRADING, "1"))
}

/// An authenticated request that can only be constructed by [`Client`].
///
/// Custom transports may inspect the non-authentication request shape for
/// deterministic tests. Authentication headers remain private.
pub struct Request {
    method: Method,
    path: String,
    body: String,
    authentication: Option<AuthenticationHeaders>,
}

impl Request {
    pub const fn method(&self) -> Method {
        self.method
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn body(&self) -> &str {
        &self.body
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Request")
            .field("method", &self.method)
            .field("path", &self.path)
            .field("body", &"[REDACTED]")
            .field("body_len", &self.body.len())
            .field("authentication", &"[REDACTED]")
            .finish()
    }
}

/// Transport seam for the authenticated wire.
///
/// The public seam permits adapter-level fakes without making authenticated
/// header construction public.
#[async_trait]
pub trait Transport: Send + Sync {
    async fn execute(&self, request: Request) -> Result<Response, Error>;
}

/// The production signed-request executor.
#[derive(Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
    base_url: String,
}

impl ReqwestTransport {
    pub fn new(base_url: impl Into<String>) -> Result<Self, Error> {
        Self::with_timeouts(base_url, Duration::from_secs(2), Duration::from_secs(5))
    }

    pub fn with_timeouts(
        base_url: impl Into<String>,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, Error> {
        let base_url = normalize_base_url(base_url.into())?;
        let client = reqwest::Client::builder()
            .connect_timeout(connect_timeout)
            .timeout(request_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .tcp_nodelay(true)
            .build()
            .map_err(|error| Error::Transport(error.to_string()))?;
        Ok(Self { client, base_url })
    }
}

impl fmt::Debug for ReqwestTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReqwestTransport")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

fn normalize_base_url(base_url: String) -> Result<String, Error> {
    let parsed = reqwest::Url::parse(&base_url).map_err(|_| Error::InvalidBaseUrl)?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(Error::InvalidBaseUrl);
    }
    Ok(base_url.trim_end_matches('/').to_string())
}

#[async_trait]
impl Transport for ReqwestTransport {
    async fn execute(&self, request: Request) -> Result<Response, Error> {
        let Request {
            method,
            path,
            body,
            authentication,
        } = request;

        let url = format!("{}{path}", self.base_url);
        let method = match method {
            Method::Get => reqwest::Method::GET,
            Method::Post => reqwest::Method::POST,
        };
        let mut builder = self.client.request(method, url);
        if let Some(AuthenticationHeaders {
            api_key,
            passphrase,
            timestamp,
            signature,
            demo_trading,
        }) = authentication
        {
            builder = builder
                .header(OK_ACCESS_KEY, api_key.as_str())
                .header(OK_ACCESS_PASSPHRASE, passphrase.as_str())
                .header(OK_ACCESS_TIMESTAMP, timestamp)
                .header(OK_ACCESS_SIGN, signature.as_str())
                .header(CONTENT_TYPE, JSON_CONTENT_TYPE);
            if let Some((name, value)) = demo_header(demo_trading) {
                builder = builder.header(name, value);
            }
        }
        if !body.is_empty() {
            builder = builder.body(body);
        }

        let response = builder
            .send()
            .await
            .map_err(|error| Error::Transport(error.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .bytes()
            .await
            .map_err(|error| Error::Transport(error.to_string()))?
            .to_vec();
        Ok(Response::new(status, body))
    }
}

/// A zeroizing WebSocket login payload with redacted debug output.
pub struct WebSocketLogin {
    payload: Zeroizing<String>,
}

impl WebSocketLogin {
    /// Borrows the serialized payload for immediate transmission.
    pub fn as_str(&self) -> &str {
        self.payload.as_str()
    }
}

impl fmt::Debug for WebSocketLogin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebSocketLogin")
            .field("payload", &"[REDACTED]")
            .field("payload_len", &self.payload.len())
            .finish()
    }
}

/// Authenticated OKX wire access used behind endpoint-specific role adapters.
pub struct Client<T> {
    credentials: Credentials,
    demo_trading: bool,
    transport: T,
}

impl<T> Client<T> {
    pub fn new(transport: T, credentials: Credentials, demo_trading: bool) -> Self {
        Self {
            credentials,
            demo_trading,
            transport,
        }
    }

    /// Constructs the constrained private-session login operation.
    pub fn websocket_login(&self) -> Result<WebSocketLogin, Error> {
        self.websocket_login_at(&Utc::now().timestamp().to_string())
    }

    fn websocket_login_at(&self, timestamp_seconds: &str) -> Result<WebSocketLogin, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct LoginArgument<'a> {
            api_key: &'a str,
            passphrase: &'a str,
            timestamp: &'a str,
            sign: &'a str,
        }

        #[derive(Serialize)]
        struct Login<'a> {
            op: &'static str,
            args: [LoginArgument<'a>; 1],
        }

        let sign = Zeroizing::new(self.signature(
            timestamp_seconds,
            Method::Get,
            WEBSOCKET_LOGIN_PATH,
            "",
        )?);
        let payload = serde_json::to_string(&Login {
            op: "login",
            args: [LoginArgument {
                api_key: self.credentials.api_key.as_str(),
                passphrase: self.credentials.passphrase.as_str(),
                timestamp: timestamp_seconds,
                sign: sign.as_str(),
            }],
        })?;
        Ok(WebSocketLogin {
            payload: Zeroizing::new(payload),
        })
    }

    fn signature(
        &self,
        timestamp: &str,
        method: Method,
        path: &str,
        body: &str,
    ) -> Result<String, Error> {
        let mut mac = HmacSha256::new_from_slice(self.credentials.secret_key.as_bytes())
            .map_err(|_| Error::InvalidSigningKey)?;
        mac.update(timestamp.as_bytes());
        mac.update(method.as_str().as_bytes());
        mac.update(path.as_bytes());
        mac.update(body.as_bytes());
        Ok(STANDARD.encode(mac.finalize().into_bytes()))
    }

    fn build_request(
        &self,
        timestamp: &str,
        method: Method,
        path: &str,
        body: &str,
    ) -> Result<Request, Error> {
        validate_path(path)?;
        let signature = self.signature(timestamp, method, path, body)?;
        Ok(Request {
            method,
            path: path.to_string(),
            body: body.to_string(),
            authentication: Some(AuthenticationHeaders {
                api_key: Zeroizing::new(self.credentials.api_key.to_string()),
                passphrase: Zeroizing::new(self.credentials.passphrase.to_string()),
                timestamp: timestamp.to_string(),
                signature: Zeroizing::new(signature),
                demo_trading: self.demo_trading,
            }),
        })
    }

    fn build_public_get_request(&self, path: &str) -> Result<Request, Error> {
        validate_path(path)?;
        Ok(Request {
            method: Method::Get,
            path: path.to_string(),
            body: String::new(),
            authentication: None,
        })
    }
}

impl<T: Transport> Client<T> {
    /// Executes an unsigned GET for an adapter-approved public path.
    pub async fn public_get(&self, path: &str) -> Result<Response, Error> {
        let request = self.build_public_get_request(path)?;
        self.transport.execute(request).await
    }

    /// Executes a signed GET for an adapter-approved path.
    pub async fn get(&self, path: &str) -> Result<Response, Error> {
        self.execute_at(rest_timestamp(), Method::Get, path, "")
            .await
    }

    /// Executes a signed GET at an adapter-supplied exchange-adjusted timestamp.
    ///
    /// This exists for role workflows that first sample exchange time and must
    /// preserve one explicit timestamp across request construction.
    pub async fn get_at(&self, timestamp: &str, path: &str) -> Result<Response, Error> {
        self.execute_at(timestamp.to_string(), Method::Get, path, "")
            .await
    }

    /// Executes a signed POST for an adapter-approved path and exact JSON body.
    pub async fn post(&self, path: &str, body: &str) -> Result<Response, Error> {
        self.execute_at(rest_timestamp(), Method::Post, path, body)
            .await
    }

    /// Executes a signed POST at an adapter-supplied exchange-adjusted timestamp.
    pub async fn post_at(
        &self,
        timestamp: &str,
        path: &str,
        body: &str,
    ) -> Result<Response, Error> {
        self.execute_at(timestamp.to_string(), Method::Post, path, body)
            .await
    }

    async fn execute_at(
        &self,
        timestamp: String,
        method: Method,
        path: &str,
        body: &str,
    ) -> Result<Response, Error> {
        let request = self.build_request(&timestamp, method, path, body)?;
        self.transport.execute(request).await
    }
}

impl<T> fmt::Debug for Client<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Client")
            .field("transport", &std::any::type_name::<T>())
            .field("credentials", &"[REDACTED]")
            .field("demo_trading", &self.demo_trading)
            .finish()
    }
}

fn rest_timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn validate_path(path: &str) -> Result<(), Error> {
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.contains('#')
        || path.chars().any(char::is_control)
    {
        return Err(Error::InvalidPath);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RecordedRequest {
        method: Method,
        path: String,
        body: String,
    }

    #[derive(Clone, Default)]
    struct RecordingTransport {
        requests: Arc<Mutex<Vec<RecordedRequest>>>,
    }

    #[async_trait]
    impl Transport for RecordingTransport {
        async fn execute(&self, request: Request) -> Result<Response, Error> {
            self.requests.lock().unwrap().push(RecordedRequest {
                method: request.method(),
                path: request.path().to_string(),
                body: request.body().to_string(),
            });
            Ok(Response::new(200, br#"{"code":"0"}"#.to_vec()))
        }
    }

    fn credentials() -> Credentials {
        Credentials::new("key", "actual-secret-material", "pass").unwrap()
    }

    #[test]
    fn signs_exact_prehash_and_redacts_all_authentication_debug() {
        let client = Client::new(RecordingTransport::default(), credentials(), false);
        let timestamp = "2020-12-08T09:08:57.715Z";
        let path = "/api/v5/account/balance?ccy=BTC";
        let signature = client.signature(timestamp, Method::Get, path, "").unwrap();
        assert_eq!(signature, "GpEEDu/mXv8KcL4vAOIcuA7n2MGtmaxOnshqzPQ3tQI=");

        let request = client
            .build_request(timestamp, Method::Get, path, "")
            .unwrap();
        let authentication = request.authentication.as_ref().unwrap();
        assert_eq!(authentication.api_key.as_str(), "key");
        assert_eq!(authentication.passphrase.as_str(), "pass");
        assert_eq!(authentication.timestamp, timestamp);
        assert_eq!(authentication.signature.as_str(), signature);
        assert!(!authentication.demo_trading);

        let debug = format!("{client:?} {request:?}");
        for secret in ["key", "actual-secret-material", "pass", signature.as_str()] {
            assert!(!debug.contains(secret), "debug output exposed {secret:?}");
        }
    }

    #[test]
    fn generates_exact_demo_header_and_websocket_login() {
        let client = Client::new(
            RecordingTransport::default(),
            Credentials::new("key", "secret", "pass").unwrap(),
            true,
        );
        let request = client
            .build_request("time", Method::Post, "/path", "{}")
            .unwrap();
        let authentication = request.authentication.as_ref().unwrap();
        assert!(authentication.demo_trading);
        assert_eq!(
            demo_header(authentication.demo_trading),
            Some(("x-simulated-trading", "1"))
        );
        assert_eq!(JSON_CONTENT_TYPE, "application/json");

        let login = client.websocket_login_at("1538054050").unwrap();
        assert_eq!(
            login.as_str(),
            r#"{"op":"login","args":[{"apiKey":"key","passphrase":"pass","timestamp":"1538054050","sign":"Gj2hQIVKFcXbiwCak8SmVOu5mxPCizWDdmUAhbx8Z+s="}]}"#
        );

        let debug = format!("{login:?} {request:?}");
        for secret in [
            "key",
            "secret",
            "pass",
            "Gj2hQIVKFcXbiwCak8SmVOu5mxPCizWDdmUAhbx8Z+s=",
        ] {
            assert!(!debug.contains(secret), "debug output exposed {secret:?}");
        }
    }

    #[tokio::test]
    async fn mock_transport_records_exact_method_path_and_body() {
        let transport = RecordingTransport::default();
        let requests = Arc::clone(&transport.requests);
        let client = Client::new(transport, credentials(), true);

        let get_response = client
            .get("/api/v5/account/balance?ccy=USDT")
            .await
            .unwrap();
        let post_response = client
            .post(
                "/api/v5/trade/order",
                r#"{"instId":"BTC-USDT","side":"buy"}"#,
            )
            .await
            .unwrap();

        assert_eq!(get_response.status(), 200);
        assert!(post_response.is_success());
        assert_eq!(
            *requests.lock().unwrap(),
            vec![
                RecordedRequest {
                    method: Method::Get,
                    path: "/api/v5/account/balance?ccy=USDT".to_string(),
                    body: String::new(),
                },
                RecordedRequest {
                    method: Method::Post,
                    path: "/api/v5/trade/order".to_string(),
                    body: r#"{"instId":"BTC-USDT","side":"buy"}"#.to_string(),
                },
            ]
        );
    }

    #[tokio::test]
    async fn adapter_supplied_timestamp_is_used_for_get_and_post() {
        #[derive(Default)]
        struct TimestampTransport {
            timestamps: Mutex<Vec<String>>,
        }

        #[async_trait]
        impl Transport for TimestampTransport {
            async fn execute(&self, request: Request) -> Result<Response, Error> {
                self.timestamps.lock().unwrap().push(
                    request
                        .authentication
                        .expect("timestamped requests are authenticated")
                        .timestamp,
                );
                Ok(Response::new(200, Vec::new()))
            }
        }

        let client = Client::new(TimestampTransport::default(), credentials(), false);
        client
            .get_at("2020-12-08T09:08:57.715Z", "/api/v5/account/config")
            .await
            .unwrap();
        client
            .post_at(
                "2020-12-08T09:08:58.125Z",
                "/api/v5/trade/cancel-order",
                r#"{"instId":"BTC-USDT","ordId":"1"}"#,
            )
            .await
            .unwrap();

        assert_eq!(
            *client.transport.timestamps.lock().unwrap(),
            [
                "2020-12-08T09:08:57.715Z".to_string(),
                "2020-12-08T09:08:58.125Z".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn public_get_has_no_authentication_or_demo_headers() {
        #[derive(Default)]
        struct PublicTransport {
            saw_unsigned_request: Mutex<bool>,
        }

        #[async_trait]
        impl Transport for PublicTransport {
            async fn execute(&self, request: Request) -> Result<Response, Error> {
                assert_eq!(request.method(), Method::Get);
                assert_eq!(request.path(), "/api/v5/public/time");
                assert_eq!(request.body(), "");
                assert!(request.authentication.is_none());
                *self.saw_unsigned_request.lock().unwrap() = true;
                Ok(Response::new(200, Vec::new()))
            }
        }

        let transport = PublicTransport::default();
        let client = Client::new(transport, credentials(), true);
        let response = client.public_get("/api/v5/public/time").await.unwrap();

        assert!(response.is_success());
        assert!(*client.transport.saw_unsigned_request.lock().unwrap());
    }

    #[tokio::test]
    async fn rejects_paths_that_could_change_the_authenticated_origin() {
        let client = Client::new(RecordingTransport::default(), credentials(), false);
        for path in [
            "https://attacker.example/path",
            "//attacker.example/path",
            "/path#fragment",
        ] {
            assert!(matches!(client.get(path).await, Err(Error::InvalidPath)));
        }
    }

    #[test]
    fn legacy_credential_values_are_deferred_to_the_transport() {
        for credentials in [
            Credentials::new("", "secret", "pass").unwrap(),
            Credentials::new("key", "", "pass").unwrap(),
            Credentials::new("key", "secret", "pass\nword").unwrap(),
        ] {
            let debug = format!("{credentials:?}");
            assert!(debug.contains("[REDACTED]"));
            assert!(!debug.contains("pass\nword"));
        }
    }
}
