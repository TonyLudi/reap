//! Private fixed-purpose transport and its loopback evidence seam.
//!
//! `PRODUCTION_POLICY` records the eventual closed transport shape for review,
//! but no production constructor consumes it. In particular, this module does
//! not create a live production client and cannot be reached from another
//! crate. Its only concrete sink is compiled for tests or the explicit
//! `loopback-evidence` feature and accepts selection values, never an origin,
//! method, path, query, body, or caller-selected headers. The later private
//! attempt seam adds only its fixed `/time` and same-holder closed-only stages.

use std::time::Duration;
#[cfg(any(test, feature = "loopback-evidence"))]
use std::time::Instant;

use reap_polymarket_auth::MAX_L1_CREDENTIAL_DERIVATION_RESPONSE_BYTES;

const DERIVE_API_KEY_PATH: &str = "/auth/derive-api-key";
#[cfg(any(test, feature = "loopback-evidence"))]
const SERVER_TIME_PATH: &str = "/time";
#[cfg(any(test, feature = "loopback-evidence"))]
const CLOSED_ONLY_PATH: &str = "/auth/ban-status/closed-only";
#[cfg(any(test, feature = "loopback-evidence"))]
const MAX_LOOPBACK_SOURCE_TIME_AGE: Duration = Duration::from_secs(5);

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum MethodPolicy {
    Get,
}

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum ProxyPolicy {
    Disabled,
}

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum RedirectPolicy {
    None,
}

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum RetryPolicy {
    Never,
}

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum HttpVersionPolicy {
    Http1Only,
}

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum ConnectionPolicy {
    Close,
}

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum SelectionPolicy {
    Required,
}

#[allow(dead_code)]
struct ClosedProductionTransportPolicy {
    scheme: &'static str,
    dns_name: &'static str,
    port: u16,
    method: MethodPolicy,
    path: &'static str,
    connect_timeout: Duration,
    request_timeout: Duration,
    https_only: bool,
    proxy: ProxyPolicy,
    redirects: RedirectPolicy,
    retry: RetryPolicy,
    http_version: HttpVersionPolicy,
    maximum_idle_connections_per_host: usize,
    connection: ConnectionPolicy,
    fixed_peer_resolve: SelectionPolicy,
    interface_binding: SelectionPolicy,
    local_address_binding: SelectionPolicy,
    maximum_response_body_bytes: usize,
}

// Closed review target only. There is intentionally no production
// constructor or API that can turn this policy into a request.
#[allow(dead_code)]
const PRODUCTION_POLICY: ClosedProductionTransportPolicy = ClosedProductionTransportPolicy {
    scheme: "https",
    dns_name: "clob.polymarket.com",
    port: 443,
    method: MethodPolicy::Get,
    path: DERIVE_API_KEY_PATH,
    connect_timeout: Duration::from_secs(3),
    request_timeout: Duration::from_secs(5),
    https_only: true,
    proxy: ProxyPolicy::Disabled,
    redirects: RedirectPolicy::None,
    retry: RetryPolicy::Never,
    http_version: HttpVersionPolicy::Http1Only,
    maximum_idle_connections_per_host: 0,
    connection: ConnectionPolicy::Close,
    fixed_peer_resolve: SelectionPolicy::Required,
    interface_binding: SelectionPolicy::Required,
    local_address_binding: SelectionPolicy::Required,
    maximum_response_body_bytes: MAX_L1_CREDENTIAL_DERIVATION_RESPONSE_BYTES,
};

#[cfg(any(test, feature = "loopback-evidence"))]
#[allow(dead_code)]
mod loopback {
    #[cfg(test)]
    use std::time::Duration;
    use std::{fmt, net::SocketAddr};

    use reap_polymarket_auth::{
        FixedClosedOnlyRequestSink, L1CredentialDerivationRequestSink,
        L1CredentialDerivationResponseInput,
    };
    use reap_polymarket_egress_binding::{PmFixedTlsPeerSelection, PmLocalEgressSelection};
    use reqwest::{
        Client, StatusCode, Url,
        header::{
            ACCEPT, ACCEPT_ENCODING, CONNECTION, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE,
            HeaderMap, HeaderName, HeaderValue,
        },
        redirect::Policy,
    };
    use tokio::runtime::{Builder as RuntimeBuilder, Runtime};
    use zeroize::Zeroizing;

    use super::{
        CLOSED_ONLY_PATH, DERIVE_API_KEY_PATH, Instant, MAX_LOOPBACK_SOURCE_TIME_AGE,
        PRODUCTION_POLICY, SERVER_TIME_PATH,
    };

    const LOOPBACK_EVIDENCE_SCHEME: &str = "http";
    const LOOPBACK_EVIDENCE_DNS_NAME: &str = "credential-proof.test";
    const POLY_ADDRESS: HeaderName = HeaderName::from_static("poly_address");
    const POLY_SIGNATURE: HeaderName = HeaderName::from_static("poly_signature");
    const POLY_TIMESTAMP: HeaderName = HeaderName::from_static("poly_timestamp");
    const POLY_NONCE: HeaderName = HeaderName::from_static("poly_nonce");
    const POLY_API_KEY: HeaderName = HeaderName::from_static("poly_api_key");
    const POLY_PASSPHRASE: HeaderName = HeaderName::from_static("poly_passphrase");

    pub(super) struct LoopbackCredentialProofSink {
        client: Client,
        runtime: Runtime,
        url: Url,
        expected_peer: SocketAddr,
        spent: bool,
    }

    pub(crate) struct LoopbackCredentialProofAttemptTransport {
        client: Client,
        runtime: Runtime,
        base_url: Url,
        expected_peer: SocketAddr,
        stage: AttemptTransportStage,
        active_server_time: Option<ActiveServerTimeObservation>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum AttemptTransportStage {
        FirstTime,
        Derive,
        SecondTime,
        ClosedOnly,
        Spent,
    }

    struct ActiveServerTimeObservation {
        unix_seconds: u64,
        monotonic_receive: Instant,
    }

    pub(crate) struct LoopbackServerTimeObservation {
        unix_seconds: u64,
    }

    impl LoopbackServerTimeObservation {
        pub(crate) const fn unix_seconds(&self) -> u64 {
            self.unix_seconds
        }
    }

    impl fmt::Debug for LoopbackServerTimeObservation {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("LoopbackServerTimeObservation(<SOURCE_OWNED; NO_AUTHORITY>)")
        }
    }

    /// Private zero-size evidence that the strict loopback response body was
    /// exactly `{"closed_only":false}`. It grants no remote or mutation authority.
    pub(crate) struct ClosedOnlyFalseLoopbackEvidence;

    impl fmt::Debug for ClosedOnlyFalseLoopbackEvidence {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("ClosedOnlyFalseLoopbackEvidence(<DENIED; LOOPBACK_ONLY>)")
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum CredentialProofTransportError {
        InvalidLoopbackSelection,
        TransportBuild,
        SinkAlreadySpent,
        InvalidAuthenticatedHeaders,
        RequestTimeout,
        RequestFailed,
        ResponsePeerMismatch,
        UnexpectedStatus,
        InvalidApplicationHeaders,
        ResponseBodyTooLarge,
        ResponseBodyFault,
        ResponseInputRejected,
        UnexpectedAttemptStage,
        InvalidServerTimeBody,
        ClosedOnlyTrueOrMalformed,
        MissingOrMismatchedServerTime,
        SourceTimeExpired,
    }

    impl fmt::Display for CredentialProofTransportError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(match self {
                Self::InvalidLoopbackSelection => "invalid loopback transport selection",
                Self::TransportBuild => "credential-proof transport build failed",
                Self::SinkAlreadySpent => "credential-proof sink already spent",
                Self::InvalidAuthenticatedHeaders => "invalid authenticated header value",
                Self::RequestTimeout => "credential-proof request timed out",
                Self::RequestFailed => "credential-proof request failed",
                Self::ResponsePeerMismatch => "credential-proof response peer mismatch",
                Self::UnexpectedStatus => "credential-proof response status rejected",
                Self::InvalidApplicationHeaders => {
                    "credential-proof response application headers rejected"
                }
                Self::ResponseBodyTooLarge => "credential-proof response body too large",
                Self::ResponseBodyFault => "credential-proof response body failed",
                Self::ResponseInputRejected => "credential-proof response input rejected",
                Self::UnexpectedAttemptStage => "credential-proof attempt stage rejected",
                Self::InvalidServerTimeBody => "credential-proof server-time body rejected",
                Self::ClosedOnlyTrueOrMalformed => "credential-proof closed-only response rejected",
                Self::MissingOrMismatchedServerTime => {
                    "credential-proof source-owned server time rejected"
                }
                Self::SourceTimeExpired => "credential-proof source-owned server time expired",
            })
        }
    }

    impl std::error::Error for CredentialProofTransportError {}

    impl LoopbackCredentialProofSink {
        #[cfg(any(test, feature = "loopback-evidence"))]
        pub(super) fn loopback_evidence(
            fixed_peer: PmFixedTlsPeerSelection,
            selected_local_egress: PmLocalEgressSelection,
        ) -> Result<Self, CredentialProofTransportError> {
            let (client, runtime, url, expected_peer) =
                build_loopback_transport(fixed_peer, selected_local_egress)?;

            Ok(Self {
                client,
                runtime,
                url,
                expected_peer,
                spent: false,
            })
        }
    }

    impl LoopbackCredentialProofAttemptTransport {
        pub(crate) fn loopback_evidence(
            fixed_peer: PmFixedTlsPeerSelection,
            selected_local_egress: PmLocalEgressSelection,
        ) -> Result<Self, CredentialProofTransportError> {
            let (client, runtime, base_url, expected_peer) =
                build_loopback_transport(fixed_peer, selected_local_egress)?;
            Ok(Self {
                client,
                runtime,
                base_url,
                expected_peer,
                stage: AttemptTransportStage::FirstTime,
                active_server_time: None,
            })
        }

        pub(crate) fn first_server_time(
            &mut self,
        ) -> Result<LoopbackServerTimeObservation, CredentialProofTransportError> {
            self.advance(
                AttemptTransportStage::FirstTime,
                AttemptTransportStage::Derive,
            )?;
            self.fetch_server_time()
        }

        pub(crate) fn second_server_time(
            &mut self,
        ) -> Result<LoopbackServerTimeObservation, CredentialProofTransportError> {
            self.advance(
                AttemptTransportStage::SecondTime,
                AttemptTransportStage::ClosedOnly,
            )?;
            self.fetch_server_time()
        }

        fn fetch_server_time(
            &mut self,
        ) -> Result<LoopbackServerTimeObservation, CredentialProofTransportError> {
            if self.active_server_time.is_some() {
                return Err(CredentialProofTransportError::UnexpectedAttemptStage);
            }
            let url = exact_url(&self.base_url, SERVER_TIME_PATH)?;
            let headers = public_json_headers();
            let body = self.runtime.block_on(receive_bounded_attempt_response(
                &self.client,
                url,
                self.expected_peer,
                headers,
                10,
                None,
            ))?;
            if body.len() != 10 || !body.iter().all(u8::is_ascii_digit) {
                return Err(CredentialProofTransportError::InvalidServerTimeBody);
            }
            let unix_seconds = std::str::from_utf8(body.as_slice())
                .ok()
                .and_then(|value| value.parse().ok())
                .ok_or(CredentialProofTransportError::InvalidServerTimeBody)?;
            self.active_server_time = Some(ActiveServerTimeObservation {
                unix_seconds,
                monotonic_receive: Instant::now(),
            });
            Ok(LoopbackServerTimeObservation { unix_seconds })
        }

        fn advance(
            &mut self,
            expected: AttemptTransportStage,
            next: AttemptTransportStage,
        ) -> Result<(), CredentialProofTransportError> {
            let observed = std::mem::replace(&mut self.stage, AttemptTransportStage::Spent);
            if observed != expected {
                return Err(CredentialProofTransportError::UnexpectedAttemptStage);
            }
            self.stage = next;
            Ok(())
        }

        fn take_matching_server_time(
            &mut self,
            poly_timestamp: &str,
        ) -> Result<ActiveServerTimeObservation, CredentialProofTransportError> {
            let observation = self
                .active_server_time
                .take()
                .ok_or(CredentialProofTransportError::MissingOrMismatchedServerTime)?;
            if observation.unix_seconds.to_string() != poly_timestamp {
                return Err(CredentialProofTransportError::MissingOrMismatchedServerTime);
            }
            require_fresh_source_time(&observation)?;
            Ok(observation)
        }

        #[cfg(test)]
        fn expire_active_server_time_for_test(&mut self) {
            if let Some(observation) = self.active_server_time.as_mut() {
                observation.monotonic_receive = Instant::now()
                    .checked_sub(MAX_LOOPBACK_SOURCE_TIME_AGE + Duration::from_nanos(1))
                    .expect("synthetic monotonic instant");
            }
        }
    }

    impl L1CredentialDerivationRequestSink for LoopbackCredentialProofSink {
        type Output = L1CredentialDerivationResponseInput;
        type Error = CredentialProofTransportError;

        fn send_exact_get_auth_derive_api_key(
            &mut self,
            poly_address: &str,
            poly_signature: &str,
            poly_timestamp: &str,
            poly_nonce: &str,
        ) -> Result<Self::Output, Self::Error> {
            if std::mem::replace(&mut self.spent, true) {
                return Err(CredentialProofTransportError::SinkAlreadySpent);
            }
            let headers =
                authenticated_headers(poly_address, poly_signature, poly_timestamp, poly_nonce)?;
            self.runtime.block_on(receive_one_response(
                &self.client,
                self.url.clone(),
                self.expected_peer,
                headers,
                None,
            ))
        }
    }

    impl L1CredentialDerivationRequestSink for LoopbackCredentialProofAttemptTransport {
        type Output = L1CredentialDerivationResponseInput;
        type Error = CredentialProofTransportError;

        fn send_exact_get_auth_derive_api_key(
            &mut self,
            poly_address: &str,
            poly_signature: &str,
            poly_timestamp: &str,
            poly_nonce: &str,
        ) -> Result<Self::Output, Self::Error> {
            self.advance(
                AttemptTransportStage::Derive,
                AttemptTransportStage::SecondTime,
            )?;
            let headers =
                authenticated_headers(poly_address, poly_signature, poly_timestamp, poly_nonce)?;
            let url = exact_url(&self.base_url, DERIVE_API_KEY_PATH)?;
            let source_time = self.take_matching_server_time(poly_timestamp)?;
            self.runtime.block_on(receive_one_response(
                &self.client,
                url,
                self.expected_peer,
                headers,
                Some(&source_time),
            ))
        }
    }

    impl FixedClosedOnlyRequestSink for LoopbackCredentialProofAttemptTransport {
        type Output = ClosedOnlyFalseLoopbackEvidence;
        type Error = CredentialProofTransportError;

        fn send_exact_get_auth_ban_status_closed_only(
            &mut self,
            poly_address: &str,
            poly_signature: &str,
            poly_timestamp: &str,
            poly_api_key: &str,
            poly_passphrase: &str,
        ) -> Result<Self::Output, Self::Error> {
            self.advance(
                AttemptTransportStage::ClosedOnly,
                AttemptTransportStage::Spent,
            )?;
            let mut headers = HeaderMap::with_capacity(7);
            headers.insert(POLY_ADDRESS, sensitive_header(poly_address)?);
            headers.insert(POLY_SIGNATURE, sensitive_header(poly_signature)?);
            headers.insert(POLY_TIMESTAMP, sensitive_header(poly_timestamp)?);
            headers.insert(POLY_API_KEY, sensitive_header(poly_api_key)?);
            headers.insert(POLY_PASSPHRASE, sensitive_header(poly_passphrase)?);
            headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
            headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
            let url = exact_url(&self.base_url, CLOSED_ONLY_PATH)?;
            let source_time = self.take_matching_server_time(poly_timestamp)?;
            let body = self.runtime.block_on(receive_bounded_attempt_response(
                &self.client,
                url,
                self.expected_peer,
                headers,
                21,
                Some(&source_time),
            ))?;
            if body.as_slice() != br#"{"closed_only":false}"# {
                return Err(CredentialProofTransportError::ClosedOnlyTrueOrMalformed);
            }
            Ok(ClosedOnlyFalseLoopbackEvidence)
        }
    }

    fn build_loopback_transport(
        fixed_peer: PmFixedTlsPeerSelection,
        selected_local_egress: PmLocalEgressSelection,
    ) -> Result<(Client, Runtime, Url, SocketAddr), CredentialProofTransportError> {
        fixed_peer
            .require_loopback_evidence()
            .map_err(|_| CredentialProofTransportError::InvalidLoopbackSelection)?;
        selected_local_egress
            .require_loopback_evidence()
            .map_err(|_| CredentialProofTransportError::InvalidLoopbackSelection)?;
        fixed_peer
            .require_same_address_family(&selected_local_egress)
            .map_err(|_| CredentialProofTransportError::InvalidLoopbackSelection)?;
        if fixed_peer.dns_name() != LOOPBACK_EVIDENCE_DNS_NAME {
            return Err(CredentialProofTransportError::InvalidLoopbackSelection);
        }

        let mut url = Url::parse(&format!(
            "{LOOPBACK_EVIDENCE_SCHEME}://{LOOPBACK_EVIDENCE_DNS_NAME}"
        ))
        .map_err(|_| CredentialProofTransportError::TransportBuild)?;
        url.set_port(Some(fixed_peer.peer_addr().port()))
            .map_err(|_| CredentialProofTransportError::TransportBuild)?;
        url.set_path(DERIVE_API_KEY_PATH);
        url.set_query(None);
        url.set_fragment(None);

        let builder = Client::builder()
            .connect_timeout(PRODUCTION_POLICY.connect_timeout)
            .timeout(PRODUCTION_POLICY.request_timeout)
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .no_gzip()
            .no_brotli()
            .no_zstd()
            .no_deflate()
            .http1_only()
            .pool_max_idle_per_host(PRODUCTION_POLICY.maximum_idle_connections_per_host);
        #[cfg(target_os = "linux")]
        let builder = builder
            .interface(selected_local_egress.interface_name())
            .local_address(selected_local_egress.local_source_ip());
        #[cfg(not(target_os = "linux"))]
        {
            let _ = builder;
            let _ = selected_local_egress;
            return Err(CredentialProofTransportError::TransportBuild);
        }
        let expected_peer = fixed_peer.peer_addr();
        let builder = builder.resolve(fixed_peer.dns_name(), expected_peer);
        let runtime = RuntimeBuilder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|_| CredentialProofTransportError::TransportBuild)?;
        let client = {
            let _runtime_context = runtime.enter();
            builder
                .build()
                .map_err(|_| CredentialProofTransportError::TransportBuild)?
        };
        Ok((client, runtime, url, expected_peer))
    }

    fn exact_url(base: &Url, path: &str) -> Result<Url, CredentialProofTransportError> {
        let mut url = base.clone();
        url.set_path(path);
        url.set_query(None);
        url.set_fragment(None);
        Ok(url)
    }

    fn public_json_headers() -> HeaderMap {
        let mut headers = HeaderMap::with_capacity(2);
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        headers
    }

    fn authenticated_headers(
        poly_address: &str,
        poly_signature: &str,
        poly_timestamp: &str,
        poly_nonce: &str,
    ) -> Result<HeaderMap, CredentialProofTransportError> {
        let mut headers = HeaderMap::with_capacity(6);
        headers.insert(POLY_ADDRESS, sensitive_header(poly_address)?);
        headers.insert(POLY_SIGNATURE, sensitive_header(poly_signature)?);
        headers.insert(POLY_TIMESTAMP, sensitive_header(poly_timestamp)?);
        headers.insert(POLY_NONCE, sensitive_header(poly_nonce)?);
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        Ok(headers)
    }

    fn sensitive_header(value: &str) -> Result<HeaderValue, CredentialProofTransportError> {
        let mut value = HeaderValue::from_str(value)
            .map_err(|_| CredentialProofTransportError::InvalidAuthenticatedHeaders)?;
        value.set_sensitive(true);
        Ok(value)
    }

    async fn receive_one_response(
        client: &Client,
        url: Url,
        expected_peer: SocketAddr,
        headers: HeaderMap,
        source_time: Option<&ActiveServerTimeObservation>,
    ) -> Result<L1CredentialDerivationResponseInput, CredentialProofTransportError> {
        let request = client.get(url).headers(headers).header(CONNECTION, "close");
        if let Some(source_time) = source_time {
            require_fresh_source_time(source_time)?;
        }
        let mut response = request.send().await.map_err(map_request_error)?;

        validate_response_peer(expected_peer, response.remote_addr())?;
        if response.status() != StatusCode::OK {
            return Err(CredentialProofTransportError::UnexpectedStatus);
        }
        validate_application_headers(response.headers())?;
        let declared_length = validate_declared_length(response.headers())?;
        if response
            .content_length()
            .is_some_and(|length| length > PRODUCTION_POLICY.maximum_response_body_bytes as u64)
        {
            return Err(CredentialProofTransportError::ResponseBodyTooLarge);
        }

        let capacity = declared_length
            .unwrap_or(0)
            .min(PRODUCTION_POLICY.maximum_response_body_bytes);
        let mut body = Zeroizing::new(Vec::with_capacity(capacity));
        while let Some(chunk) = response.chunk().await.map_err(map_body_error)? {
            let next_length = body
                .len()
                .checked_add(chunk.len())
                .ok_or(CredentialProofTransportError::ResponseBodyTooLarge)?;
            if next_length > PRODUCTION_POLICY.maximum_response_body_bytes {
                return Err(CredentialProofTransportError::ResponseBodyTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        if let Some(source_time) = source_time {
            require_fresh_source_time(source_time)?;
        }

        let owned_body = std::mem::take(&mut *body);
        L1CredentialDerivationResponseInput::new(owned_body)
            .map_err(|_| CredentialProofTransportError::ResponseInputRejected)
    }

    async fn receive_bounded_attempt_response(
        client: &Client,
        url: Url,
        expected_peer: SocketAddr,
        headers: HeaderMap,
        maximum_body_bytes: usize,
        source_time: Option<&ActiveServerTimeObservation>,
    ) -> Result<Zeroizing<Vec<u8>>, CredentialProofTransportError> {
        let request = client.get(url).headers(headers).header(CONNECTION, "close");
        if let Some(source_time) = source_time {
            require_fresh_source_time(source_time)?;
        }
        let mut response = request.send().await.map_err(map_request_error)?;
        validate_response_peer(expected_peer, response.remote_addr())?;
        if response.status() != StatusCode::OK {
            return Err(CredentialProofTransportError::UnexpectedStatus);
        }
        validate_application_headers(response.headers())?;
        let declared_length =
            validate_declared_length_with_bound(response.headers(), maximum_body_bytes)?;
        if response
            .content_length()
            .is_some_and(|length| length > maximum_body_bytes as u64)
        {
            return Err(CredentialProofTransportError::ResponseBodyTooLarge);
        }
        let capacity = declared_length.unwrap_or(0).min(maximum_body_bytes);
        let mut body = Zeroizing::new(Vec::with_capacity(capacity));
        while let Some(chunk) = response.chunk().await.map_err(map_body_error)? {
            let next_length = body
                .len()
                .checked_add(chunk.len())
                .ok_or(CredentialProofTransportError::ResponseBodyTooLarge)?;
            if next_length > maximum_body_bytes {
                return Err(CredentialProofTransportError::ResponseBodyTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        if let Some(source_time) = source_time {
            require_fresh_source_time(source_time)?;
        }
        Ok(body)
    }

    fn require_fresh_source_time(
        observation: &ActiveServerTimeObservation,
    ) -> Result<(), CredentialProofTransportError> {
        validate_source_time_age(observation, Instant::now())
    }

    fn validate_source_time_age(
        observation: &ActiveServerTimeObservation,
        now: Instant,
    ) -> Result<(), CredentialProofTransportError> {
        let age = now
            .checked_duration_since(observation.monotonic_receive)
            .ok_or(CredentialProofTransportError::SourceTimeExpired)?;
        if age > MAX_LOOPBACK_SOURCE_TIME_AGE {
            return Err(CredentialProofTransportError::SourceTimeExpired);
        }
        Ok(())
    }

    fn validate_response_peer(
        expected_peer: SocketAddr,
        observed_peer: Option<SocketAddr>,
    ) -> Result<(), CredentialProofTransportError> {
        if observed_peer != Some(expected_peer) {
            return Err(CredentialProofTransportError::ResponsePeerMismatch);
        }
        Ok(())
    }

    fn validate_application_headers(
        headers: &HeaderMap,
    ) -> Result<(), CredentialProofTransportError> {
        let content_type = exactly_one_header(headers, CONTENT_TYPE)?
            .ok_or(CredentialProofTransportError::InvalidApplicationHeaders)?;
        let content_type = content_type
            .to_str()
            .map_err(|_| CredentialProofTransportError::InvalidApplicationHeaders)?;
        if content_type != content_type.trim() {
            return Err(CredentialProofTransportError::InvalidApplicationHeaders);
        }
        let mut components = content_type.split(';');
        let essence = components
            .next()
            .ok_or(CredentialProofTransportError::InvalidApplicationHeaders)?;
        if !essence.trim().eq_ignore_ascii_case("application/json") {
            return Err(CredentialProofTransportError::InvalidApplicationHeaders);
        }
        if let Some(parameter) = components.next() {
            let (name, value) = parameter
                .trim()
                .split_once('=')
                .ok_or(CredentialProofTransportError::InvalidApplicationHeaders)?;
            if !name.trim().eq_ignore_ascii_case("charset")
                || !value.trim().eq_ignore_ascii_case("utf-8")
            {
                return Err(CredentialProofTransportError::InvalidApplicationHeaders);
            }
        }
        if components.next().is_some() {
            return Err(CredentialProofTransportError::InvalidApplicationHeaders);
        }

        if let Some(content_encoding) = exactly_one_header(headers, CONTENT_ENCODING)? {
            let content_encoding = content_encoding
                .to_str()
                .map_err(|_| CredentialProofTransportError::InvalidApplicationHeaders)?;
            if content_encoding != content_encoding.trim()
                || !content_encoding.eq_ignore_ascii_case("identity")
            {
                return Err(CredentialProofTransportError::InvalidApplicationHeaders);
            }
        }
        Ok(())
    }

    fn validate_declared_length(
        headers: &HeaderMap,
    ) -> Result<Option<usize>, CredentialProofTransportError> {
        let Some(value) = exactly_one_header(headers, CONTENT_LENGTH)? else {
            return Ok(None);
        };
        let value = value
            .to_str()
            .map_err(|_| CredentialProofTransportError::InvalidApplicationHeaders)?;
        if value.is_empty()
            || value != value.trim()
            || !value.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(CredentialProofTransportError::InvalidApplicationHeaders);
        }
        let value = value
            .parse::<u64>()
            .map_err(|_| CredentialProofTransportError::ResponseBodyTooLarge)?;
        if value > PRODUCTION_POLICY.maximum_response_body_bytes as u64 {
            return Err(CredentialProofTransportError::ResponseBodyTooLarge);
        }
        usize::try_from(value)
            .map(Some)
            .map_err(|_| CredentialProofTransportError::ResponseBodyTooLarge)
    }

    fn validate_declared_length_with_bound(
        headers: &HeaderMap,
        maximum_body_bytes: usize,
    ) -> Result<Option<usize>, CredentialProofTransportError> {
        let Some(value) = exactly_one_header(headers, CONTENT_LENGTH)? else {
            return Ok(None);
        };
        let value = value
            .to_str()
            .map_err(|_| CredentialProofTransportError::InvalidApplicationHeaders)?;
        if value.is_empty()
            || value != value.trim()
            || !value.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(CredentialProofTransportError::InvalidApplicationHeaders);
        }
        let value = value
            .parse::<u64>()
            .map_err(|_| CredentialProofTransportError::ResponseBodyTooLarge)?;
        if value > maximum_body_bytes as u64 {
            return Err(CredentialProofTransportError::ResponseBodyTooLarge);
        }
        usize::try_from(value)
            .map(Some)
            .map_err(|_| CredentialProofTransportError::ResponseBodyTooLarge)
    }

    fn exactly_one_header(
        headers: &HeaderMap,
        name: HeaderName,
    ) -> Result<Option<&HeaderValue>, CredentialProofTransportError> {
        let mut values = headers.get_all(name).iter();
        let first = values.next();
        if values.next().is_some() {
            return Err(CredentialProofTransportError::InvalidApplicationHeaders);
        }
        Ok(first)
    }

    fn map_request_error(error: reqwest::Error) -> CredentialProofTransportError {
        if error.is_timeout() {
            CredentialProofTransportError::RequestTimeout
        } else {
            CredentialProofTransportError::RequestFailed
        }
    }

    fn map_body_error(error: reqwest::Error) -> CredentialProofTransportError {
        if error.is_timeout() {
            CredentialProofTransportError::RequestTimeout
        } else {
            CredentialProofTransportError::ResponseBodyFault
        }
    }

    #[cfg(all(test, target_os = "linux"))]
    mod tests {
        include!("transport/tests.rs");
    }
}

#[cfg(any(test, feature = "loopback-evidence"))]
pub(crate) use loopback::{
    ClosedOnlyFalseLoopbackEvidence, CredentialProofTransportError,
    LoopbackCredentialProofAttemptTransport, LoopbackServerTimeObservation,
};
