use std::{fmt, net::SocketAddr};

use reap_pm_core::{EvmAddress, PmTokenId};
use reap_polymarket_auth::{AuthenticatedL2Headers, EoaAddress, FixedOrderId, L2HeaderSink};
use reap_polymarket_egress_binding::{PmFixedTlsPeerSelection, PmLocalEgressSelection};
use reap_polymarket_wire::{
    MAX_PM_CLOSED_ONLY_BODY_BYTES, MAX_PM_LIVE_BODY_BYTES, PmClosedOnlyStatus, PmWireScope,
    parse_pm_closed_only,
};
use reqwest::{
    Client, RequestBuilder, StatusCode, Url,
    header::{ACCEPT, HeaderName, HeaderValue},
    redirect::Policy,
};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

use crate::{
    PM_CLOB_PRODUCTION_ORIGIN, PmAccountHttpRole, PmLiveAdapterError, PmPrivateHttpConfig,
    PmPrivateReadEdgeClock, PmPrivateReadProductClock, PmReadOnlySignatureType,
    PmReconciliationHttpRole, config::OriginMode, private_credentials::PmHttpCredentialRole,
    read_authority::PmHttpReadAuthorityProvider,
};

const FIRST_PAGE_CURSOR: &str = "MA==";
const POLY_ADDRESS: HeaderName = HeaderName::from_static("poly_address");
const POLY_SIGNATURE: HeaderName = HeaderName::from_static("poly_signature");
const POLY_TIMESTAMP: HeaderName = HeaderName::from_static("poly_timestamp");
const POLY_API_KEY: HeaderName = HeaderName::from_static("poly_api_key");
const POLY_PASSPHRASE: HeaderName = HeaderName::from_static("poly_passphrase");
const CLOSED_ONLY_OBSERVATION_COMMITMENT_DOMAIN: &[u8] =
    b"reap.pm.live-adapter.closed-only-observation.v1\0";

/// SHA-256 commitment to one fixed signer-authenticated closed-only read.
/// Construction is private to the source role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmClosedOnlyObservationCommitment([u8; 32]);

impl PmClosedOnlyObservationCommitment {
    const fn from_source_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Sealed result of the fixed authenticated closed-only source.
///
/// For signature type 1, the commitment names the L2 signer and profile only.
/// The response does not remotely attest the separately configured proxy
/// funder.
#[derive(Debug)]
pub struct PmClosedOnlyObservation {
    status: PmClosedOnlyStatus,
    receive_clock: PmPrivateReadEdgeClock,
    commitment: PmClosedOnlyObservationCommitment,
}

impl PmClosedOnlyObservation {
    fn from_source(
        status: PmClosedOnlyStatus,
        receive_clock: PmPrivateReadEdgeClock,
        commitment: PmClosedOnlyObservationCommitment,
    ) -> Self {
        Self {
            status,
            receive_clock,
            commitment,
        }
    }

    #[must_use]
    pub const fn status(&self) -> PmClosedOnlyStatus {
        self.status
    }

    #[must_use]
    pub const fn receive_clock(&self) -> PmPrivateReadEdgeClock {
        self.receive_clock
    }

    #[must_use]
    pub const fn commitment(&self) -> PmClosedOnlyObservationCommitment {
        self.commitment
    }

    #[must_use]
    pub const fn into_status(self) -> PmClosedOnlyStatus {
        self.status
    }
}

struct FetchedClosedOnly {
    raw_response: Zeroizing<Vec<u8>>,
    parsed: PmClosedOnlyStatus,
}

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
    ClosedOnly,
}

impl PmPrivateRoute<'_> {
    const fn accepts_not_found(&self) -> bool {
        matches!(self, Self::ExactOrder(_))
    }

    const fn maximum_body_bytes(&self) -> usize {
        match self {
            Self::ClosedOnly => MAX_PM_CLOSED_ONLY_BODY_BYTES,
            Self::OpenOrders { .. }
            | Self::Trades { .. }
            | Self::ExactOrder(_)
            | Self::CollateralBalanceAllowance(_)
            | Self::ConditionalBalanceAllowance { .. } => MAX_PM_LIVE_BODY_BYTES,
        }
    }
}

pub(crate) enum PmPrivateHttpObservation {
    Found(Zeroizing<Vec<u8>>),
    NotFound,
}

pub(crate) struct PmPrivateHttpTransport {
    client: Client,
    origin: Url,
    mode: OriginMode,
    configured_address: EoaAddress,
    expected_peer: Option<SocketAddr>,
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
            config.selected_local_egress(),
            config.fixed_tls_peer(),
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
            config.selected_local_egress(),
            config.fixed_tls_peer(),
        )
    }

    fn build(
        origin: Url,
        connect_timeout: std::time::Duration,
        request_timeout: std::time::Duration,
        mode: OriginMode,
        configured_address: EoaAddress,
        selected_local_egress: Option<&PmLocalEgressSelection>,
        fixed_tls_peer: Option<&PmFixedTlsPeerSelection>,
    ) -> Result<Self, PmLiveAdapterError> {
        if fixed_tls_peer.is_some() && selected_local_egress.is_none() {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "fixed TLS peer requires an inseparable selected local egress",
            ));
        }
        let mut builder = Client::builder()
            .connect_timeout(connect_timeout)
            .timeout(request_timeout)
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy();
        if mode == OriginMode::Production {
            builder = builder.https_only(true);
        }
        if let Some(selected_local_egress) = selected_local_egress {
            #[cfg(target_os = "linux")]
            {
                builder = builder
                    .interface(selected_local_egress.interface_name())
                    .local_address(selected_local_egress.local_source_ip());
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = selected_local_egress;
                return Err(PmLiveAdapterError::InvalidConfiguration(
                    "selected local egress requires Linux",
                ));
            }
        }
        if let Some(fixed_tls_peer) = fixed_tls_peer {
            builder = builder.resolve(fixed_tls_peer.dns_name(), fixed_tls_peer.peer_addr());
        }
        let client = builder
            .build()
            .map_err(|_| PmLiveAdapterError::TransportBuild)?;
        Ok(Self {
            client,
            origin,
            mode,
            configured_address,
            expected_peer: fixed_tls_peer.map(PmFixedTlsPeerSelection::peer_addr),
        })
    }

    pub(crate) const fn mode(&self) -> OriginMode {
        self.mode
    }

    pub(crate) const fn configured_signer(&self) -> EoaAddress {
        self.configured_address
    }

    pub(crate) async fn get(
        &self,
        route: PmPrivateRoute<'_>,
        authenticated: AuthenticatedL2Headers,
    ) -> Result<PmPrivateHttpObservation, PmLiveAdapterError> {
        let accepts_not_found = route.accepts_not_found();
        let maximum_body_bytes = route.maximum_body_bytes();
        let url = self.route_url(route);
        let request = self.client.get(url).header(ACCEPT, "application/json");
        let mut sink = FixedReadHeaderSink::new(request, self.configured_address);
        authenticated.apply_to(&mut sink)?;
        let mut response = sink.finish()?.send().await.map_err(map_request_error)?;
        validate_response_peer(self.expected_peer, response.remote_addr())?;
        let status = response.status();
        if status.is_redirection() {
            return Err(PmLiveAdapterError::Redirect {
                status: status.as_u16(),
            });
        }
        let accepted_not_found = status == StatusCode::NOT_FOUND && accepts_not_found;
        if status != StatusCode::OK && !accepted_not_found {
            return Err(PmLiveAdapterError::UnexpectedStatus {
                status: status.as_u16(),
            });
        }
        if response
            .content_length()
            .is_some_and(|length| length > maximum_body_bytes as u64)
        {
            return Err(PmLiveAdapterError::ResponseBodyTooLarge {
                limit: maximum_body_bytes,
            });
        }

        let capacity = response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or(0)
            .min(maximum_body_bytes);
        let mut body = Zeroizing::new(Vec::with_capacity(capacity));
        while let Some(chunk) = response.chunk().await.map_err(map_body_error)? {
            let next_length = body.len().checked_add(chunk.len()).ok_or(
                PmLiveAdapterError::ResponseBodyTooLarge {
                    limit: maximum_body_bytes,
                },
            )?;
            if next_length > maximum_body_bytes {
                return Err(PmLiveAdapterError::ResponseBodyTooLarge {
                    limit: maximum_body_bytes,
                });
            }
            body.extend_from_slice(&chunk);
        }
        if accepted_not_found {
            Ok(PmPrivateHttpObservation::NotFound)
        } else {
            Ok(PmPrivateHttpObservation::Found(body))
        }
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
            PmPrivateRoute::ClosedOnly => url.set_path("/auth/ban-status/closed-only"),
        }
        url
    }
}

fn validate_response_peer(
    expected_peer: Option<SocketAddr>,
    observed_peer: Option<SocketAddr>,
) -> Result<(), PmLiveAdapterError> {
    if expected_peer.is_some() && expected_peer != observed_peer {
        return Err(PmLiveAdapterError::RequestFailed);
    }
    Ok(())
}

/// Move-only custody for one exact authenticated account transport.
///
/// The owner lends mutually exclusive, short-lived read capabilities. It has
/// no generic request API and cannot authorize a mutation.
pub struct PmAuthenticatedHttpOwner {
    authority: Box<dyn PmHttpReadAuthorityProvider>,
    transport: PmPrivateHttpTransport,
    exact_order_scope: PmWireScope,
    l2_signer_address: EoaAddress,
    expected_order_maker: EvmAddress,
    balance_signature_type: PmReadOnlySignatureType,
}

impl PmAuthenticatedHttpOwner {
    pub(crate) fn from_authority_with_account_profile(
        transport: PmPrivateHttpTransport,
        exact_order_scope: PmWireScope,
        l2_signer_address: EoaAddress,
        expected_order_maker: EvmAddress,
        balance_signature_type: PmReadOnlySignatureType,
        authority: PmHttpCredentialRole,
    ) -> Self {
        Self::from_external_authority_with_account_profile(
            transport,
            exact_order_scope,
            l2_signer_address,
            expected_order_maker,
            balance_signature_type,
            Box::new(authority),
        )
    }

    pub(crate) fn from_external_authority_with_account_profile(
        transport: PmPrivateHttpTransport,
        exact_order_scope: PmWireScope,
        l2_signer_address: EoaAddress,
        expected_order_maker: EvmAddress,
        balance_signature_type: PmReadOnlySignatureType,
        authority: Box<dyn PmHttpReadAuthorityProvider>,
    ) -> Self {
        Self {
            authority,
            transport,
            exact_order_scope,
            l2_signer_address,
            expected_order_maker,
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
        let exact_order_scope = config.exact_order_scope();
        let l2_signer_address = credentials.address();
        let transport = PmPrivateHttpTransport::new(&config, l2_signer_address)?;
        let (authority, supervisor) =
            crate::private_credentials::test_http_credential_role(credentials)?;
        Ok((
            Self::from_authority_with_account_profile(
                transport,
                exact_order_scope,
                l2_signer_address,
                l2_signer_address.as_core(),
                PmReadOnlySignatureType::Eoa,
                authority,
            ),
            supervisor,
        ))
    }

    #[cfg(test)]
    pub(crate) fn new_proxy_read_only(
        config: PmPrivateHttpConfig,
        expected_order_maker: EvmAddress,
        credentials: reap_polymarket_auth::L2Credentials,
    ) -> Result<(Self, crate::PmCredentialAuthoritySupervisor), PmLiveAdapterError> {
        let exact_order_scope = config.exact_order_scope();
        let l2_signer_address = credentials.address();
        if l2_signer_address.as_core() == expected_order_maker {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "proxy read profile requires distinct signer and funder",
            ));
        }
        let transport = PmPrivateHttpTransport::new(&config, l2_signer_address)?;
        let (authority, supervisor) =
            crate::private_credentials::test_http_credential_role(credentials)?;
        Ok((
            Self::from_authority_with_account_profile(
                transport,
                exact_order_scope,
                l2_signer_address,
                expected_order_maker,
                PmReadOnlySignatureType::Proxy,
                authority,
            ),
            supervisor,
        ))
    }

    pub fn reconciliation(&mut self) -> PmReconciliationHttpRole<'_> {
        debug_assert_eq!(self.transport.configured_address, self.l2_signer_address);
        PmReconciliationHttpRole::new(
            self.authority.as_mut(),
            &self.transport,
            self.exact_order_scope,
            self.expected_order_maker,
            self.balance_signature_type,
        )
    }

    pub fn account(&mut self) -> PmAccountHttpRole<'_> {
        PmAccountHttpRole::new(
            self.authority.as_mut(),
            &self.transport,
            self.exact_order_scope.token(),
            self.balance_signature_type,
        )
    }

    /// Borrow the sole fixed authenticated account-safety read capability.
    pub fn preflight(&mut self) -> PmPrivatePreflightHttpRole<'_> {
        PmPrivatePreflightHttpRole {
            authority: self.authority.as_mut(),
            transport: &self.transport,
            signature_type: self.balance_signature_type,
        }
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }
}

/// Borrowed authenticated capability for only
/// `GET /auth/ban-status/closed-only`.
pub struct PmPrivatePreflightHttpRole<'a> {
    authority: &'a mut dyn PmHttpReadAuthorityProvider,
    transport: &'a PmPrivateHttpTransport,
    signature_type: PmReadOnlySignatureType,
}

impl PmPrivatePreflightHttpRole<'_> {
    pub async fn closed_only(
        &mut self,
        server_time: crate::PmReadServerTime,
    ) -> Result<PmClosedOnlyStatus, PmLiveAdapterError> {
        Ok(self.closed_only_source(server_time).await?.parsed)
    }

    pub async fn closed_only_observation(
        &mut self,
        server_time: crate::PmReadServerTime,
        clock: &mut PmPrivateReadProductClock,
    ) -> Result<PmClosedOnlyObservation, PmLiveAdapterError> {
        let fetched = self.closed_only_source(server_time).await?;
        // The source samples after authentication, the complete bounded body,
        // and strict parsing have all succeeded.
        let receive_clock = clock
            .observe_authenticated_read_complete()
            .map_err(|_| PmLiveAdapterError::ProductClock)?;
        let commitment = closed_only_observation_commitment(
            self.transport.mode(),
            self.signature_type,
            self.transport.configured_signer().bytes(),
            &fetched.raw_response,
            fetched.parsed,
            receive_clock,
        );
        Ok(PmClosedOnlyObservation::from_source(
            fetched.parsed,
            receive_clock,
            commitment,
        ))
    }

    async fn closed_only_source(
        &mut self,
        server_time: crate::PmReadServerTime,
    ) -> Result<FetchedClosedOnly, PmLiveAdapterError> {
        let headers = self
            .authority
            .authenticate_closed_only(
                server_time
                    .into_l2_timestamp()
                    .map_err(|_| PmLiveAdapterError::ProductClock)?,
            )
            .await?;
        let body = match self
            .transport
            .get(PmPrivateRoute::ClosedOnly, headers)
            .await?
        {
            PmPrivateHttpObservation::Found(body) => body,
            PmPrivateHttpObservation::NotFound => {
                return Err(PmLiveAdapterError::UnexpectedStatus { status: 404 });
            }
        };
        let parsed = parse_pm_closed_only(&body)?;
        Ok(FetchedClosedOnly {
            raw_response: body,
            parsed,
        })
    }
}

fn closed_only_observation_commitment(
    mode: OriginMode,
    signature_type: PmReadOnlySignatureType,
    authenticated_signer: [u8; 20],
    raw_response: &[u8],
    parsed: PmClosedOnlyStatus,
    receive_clock: PmPrivateReadEdgeClock,
) -> PmClosedOnlyObservationCommitment {
    let mut digest = Sha256::new();
    encode_closed_only_bytes(&mut digest, CLOSED_ONLY_OBSERVATION_COMMITMENT_DOMAIN);
    encode_closed_only_bytes(&mut digest, origin_mode_name(mode));
    encode_closed_only_bytes(&mut digest, PM_CLOB_PRODUCTION_ORIGIN.as_bytes());
    encode_closed_only_bytes(&mut digest, b"GET");
    encode_closed_only_bytes(&mut digest, b"/auth/ban-status/closed-only");
    digest.update([signature_type.value()]);
    digest.update(authenticated_signer);
    encode_closed_only_bytes(&mut digest, raw_response);
    digest.update([u8::from(parsed.closed_only())]);
    digest.update(receive_clock.local_wall_receive_ns().to_be_bytes());
    digest.update(receive_clock.monotonic_receive_ns().to_be_bytes());
    PmClosedOnlyObservationCommitment::from_source_bytes(digest.finalize().into())
}

fn encode_closed_only_bytes(digest: &mut Sha256, value: &[u8]) {
    digest.update(
        u64::try_from(value.len())
            .expect("bounded closed-only commitment field length fits u64")
            .to_be_bytes(),
    );
    digest.update(value);
}

const fn origin_mode_name(mode: OriginMode) -> &'static [u8] {
    match mode {
        OriginMode::Production => b"production",
        #[cfg(any(test, feature = "loopback-evidence", feature = "read-only-evidence"))]
        OriginMode::LocalEvidence => b"local-evidence",
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
    use std::{net::IpAddr, time::Duration};

    use reap_pm_core::{PmConditionId, PmMarketId, PmTokenId, U256};
    use reap_polymarket_auth::{L2CredentialInput, L2Credentials};
    use reap_polymarket_egress_binding::{PmFixedTlsPeerSelection, PmLocalEgressSelection};
    use reap_polymarket_wire::PmWireScope;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::mpsc,
        task::JoinHandle,
        time::{sleep, timeout},
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

    async fn selected_one_response_server(
        body: &'static [u8],
    ) -> (String, JoinHandle<(IpAddr, String)>) {
        let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut stream, peer) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let mut chunk = [0_u8; 1_024];
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
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.write_all(body).await.unwrap();
            (peer.ip(), request)
        });
        (format!("http://127.0.0.1:{port}"), task)
    }

    fn scope() -> PmWireScope {
        PmWireScope::new(
            PmConditionId::parse(CONDITION).unwrap(),
            PmMarketId::parse(QUESTION).unwrap(),
            PmTokenId::new(U256::from_u64(123)).unwrap(),
        )
    }

    fn credentials() -> L2Credentials {
        credentials_for(API_KEY)
    }

    fn credentials_for(api_key: &str) -> L2Credentials {
        L2Credentials::bind(
            ADDRESS,
            L2CredentialInput::new(api_key.to_owned(), SECRET.into(), PASSPHRASE.into()),
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

    fn selected_local_owner(
        origin: &str,
        request_timeout: Duration,
        interface_name: &str,
    ) -> (
        crate::PmAuthenticatedHttpOwner,
        crate::PmCredentialAuthoritySupervisor,
    ) {
        let selection =
            PmLocalEgressSelection::loopback_evidence(interface_name, "127.0.0.2".parse().unwrap())
                .unwrap();
        let config = PmPrivateHttpConfig::local_evidence_on_selected_local_egress(
            origin,
            Duration::from_millis(100),
            request_timeout,
            scope(),
            selection,
        )
        .unwrap();
        crate::PmAuthenticatedHttpOwner::new(config, credentials()).unwrap()
    }

    fn local_owner_for_api_key(
        origin: &str,
        request_timeout: Duration,
        api_key: &str,
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
        crate::PmAuthenticatedHttpOwner::new(config, credentials_for(api_key)).unwrap()
    }

    fn local_proxy_owner(
        origin: &str,
        request_timeout: Duration,
        expected_order_maker: reap_pm_core::EvmAddress,
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
        crate::PmAuthenticatedHttpOwner::new_proxy_read_only(
            config,
            expected_order_maker,
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

    fn private_read_clock(readings: &[(u64, u64)]) -> PmPrivateReadProductClock {
        let owner = crate::PmProductClockOwner::test_support_scripted(readings).unwrap();
        let (_, _, _, _, private_read, _, _, _, _, _, _) = owner.split().into_views();
        private_read
    }

    async fn reconciliation_commitments_for_api_key(
        origin: &str,
        api_key: &str,
    ) -> ([u8; 32], [u8; 32], [u8; 32]) {
        let (mut owner, supervisor) =
            local_owner_for_api_key(origin, Duration::from_secs(1), api_key);
        let mut clock = private_read_clock(&[(1_000, 10), (1_001, 11), (1_002, 12)]);
        let commitments = {
            let mut role = owner.reconciliation();
            let open_cut = role
                .begin_open_orders_observation(timestamp(), &mut clock)
                .await
                .unwrap();
            let crate::PmOpenOrdersCutProgress::Complete(open_cut) = open_cut else {
                panic!("terminal open-order response must complete")
            };
            let open = role.seal_complete_open_orders(open_cut).unwrap();

            let trade_cut = role
                .begin_trades_observation(timestamp(), &mut clock)
                .await
                .unwrap();
            let crate::PmTradesCutProgress::Complete(trade_cut) = trade_cut else {
                panic!("terminal trade response must complete")
            };
            let trades = role.seal_complete_trades(trade_cut).unwrap();

            let exact = role
                .exact_local_order_detail_observation(
                    timestamp(),
                    FixedOrderId::parse(ORDER_ID).unwrap(),
                    &mut clock,
                )
                .await
                .unwrap();
            (
                open.commitment().bytes(),
                trades.commitment().bytes(),
                exact.commitment().bytes(),
            )
        };
        supervisor.shutdown().await.unwrap();
        commitments
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
        let (mut owner, supervisor) = local_proxy_owner(
            &origin,
            Duration::from_secs(1),
            reap_pm_core::EvmAddress::parse(FOREIGN_MAKER).unwrap(),
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
    async fn closed_only_is_one_fixed_signer_authenticated_get_for_proxy_accounts() {
        let (origin, mut requests, task) =
            mock_server(vec![MockResponse::ok(br#"{"closed_only":false}"#)]).await;
        let (mut owner, supervisor) = local_proxy_owner(
            &origin,
            Duration::from_secs(1),
            reap_pm_core::EvmAddress::parse(FOREIGN_MAKER).unwrap(),
        );

        let status = owner.preflight().closed_only(timestamp()).await.unwrap();
        assert!(!status.closed_only());

        let request = requests.recv().await.unwrap();
        assert_eq!(
            request.lines().next(),
            Some("GET /auth/ban-status/closed-only HTTP/1.1")
        );
        let headers = request.to_ascii_lowercase();
        assert!(headers.contains(&format!("poly_address: {}", ADDRESS.to_ascii_lowercase())));
        assert!(!headers.contains(&format!(
            "poly_address: {}",
            FOREIGN_MAKER.to_ascii_lowercase()
        )));
        assert!(headers.contains("poly_signature: n1obdnq7auhb1m63pmycfo6tkgegkwjgrzkv86-ztfe="));
        assert!(headers.contains(&format!("poly_timestamp: {AUTH_SECONDS}")));
        assert!(headers.contains(&format!("poly_api_key: {API_KEY}")));
        assert!(headers.contains(&format!("poly_passphrase: {PASSPHRASE}")));

        task.await.unwrap();
        assert!(requests.try_recv().is_err());
        supervisor.shutdown().await.unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn selected_private_client_uses_the_nondefault_loopback_source_ip() {
        let (origin, server) = selected_one_response_server(br#"{"closed_only":false}"#).await;
        let (mut owner, supervisor) = selected_local_owner(&origin, Duration::from_secs(1), "lo");
        let status = owner.preflight().closed_only(timestamp()).await.unwrap();
        assert!(!status.closed_only());
        let (peer_ip, request) = server.await.unwrap();
        assert_eq!(peer_ip, "127.0.0.2".parse::<IpAddr>().unwrap());
        assert!(request.starts_with("GET /auth/ban-status/closed-only HTTP/1.1\r\n"));
        supervisor.shutdown().await.unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn fixed_peer_private_get_uses_exact_host_source_and_peer_with_idle_decoy() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let exact_peer = listener.local_addr().unwrap();
        let decoy = TcpListener::bind(("127.0.0.3", exact_peer.port()))
            .await
            .unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, source) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let mut chunk = [0_u8; 1_024];
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
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 21\r\nConnection: close\r\n\r\n{\"closed_only\":false}",
                )
                .await
                .unwrap();
            (source, request)
        });
        let selected_source = "127.0.0.2".parse::<IpAddr>().unwrap();
        let local_egress =
            PmLocalEgressSelection::loopback_evidence("lo", selected_source).unwrap();
        let fixed_peer =
            PmFixedTlsPeerSelection::loopback_evidence("clob.polymarket.test", exact_peer).unwrap();
        let config =
            PmPrivateHttpConfig::loopback_evidence_on_fixed_tls_peer_and_selected_local_egress(
                Duration::from_secs(1),
                Duration::from_secs(1),
                scope(),
                fixed_peer,
                local_egress,
            )
            .unwrap();
        let (mut owner, supervisor) =
            crate::PmAuthenticatedHttpOwner::new(config, credentials()).unwrap();
        let status = owner.preflight().closed_only(timestamp()).await.unwrap();
        assert!(!status.closed_only());
        let (source, request) = server.await.unwrap();
        assert_eq!(source.ip(), selected_source);
        assert!(request.starts_with("GET /auth/ban-status/closed-only HTTP/1.1\r\n"));
        assert!(request.to_ascii_lowercase().contains(&format!(
            "host: clob.polymarket.test:{}\r\n",
            exact_peer.port()
        )));
        assert!(
            timeout(Duration::from_millis(100), decoy.accept())
                .await
                .is_err()
        );
        supervisor.shutdown().await.unwrap();
    }

    #[test]
    fn fixed_peer_private_response_rejects_missing_or_different_remote() {
        let expected = "127.0.0.1:443".parse().unwrap();
        let different = "127.0.0.3:443".parse().unwrap();
        assert_eq!(
            validate_response_peer(Some(expected), None),
            Err(PmLiveAdapterError::RequestFailed)
        );
        assert_eq!(
            validate_response_peer(Some(expected), Some(different)),
            Err(PmLiveAdapterError::RequestFailed)
        );
        assert!(validate_response_peer(Some(expected), Some(expected)).is_ok());
        assert!(validate_response_peer(None, None).is_ok());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn nonexistent_private_selected_interface_fails_before_a_request_arrives() {
        let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let origin = format!("http://127.0.0.1:{port}");
        let (mut owner, supervisor) =
            selected_local_owner(&origin, Duration::from_millis(100), "missing0");
        assert!(owner.preflight().closed_only(timestamp()).await.is_err());
        assert!(
            timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
        supervisor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn loopback_type_one_preflight_observations_bind_source_inputs_after_parse() {
        let closed_raw = br#"{"closed_only":false}"#;
        let balance_raw = balance();
        let alternate_balance_raw = balance_raw.replace("1000", "1001");
        let (origin, mut requests, task) = mock_server(vec![
            MockResponse::ok(closed_raw.to_vec()),
            MockResponse::ok(balance_raw.clone()),
            MockResponse::ok(balance_raw.clone()),
            MockResponse::ok(alternate_balance_raw),
        ])
        .await;
        let (mut owner, supervisor) = local_proxy_owner(
            &origin,
            Duration::from_secs(1),
            reap_pm_core::EvmAddress::parse(FOREIGN_MAKER).unwrap(),
        );
        let mut clock = private_read_clock(&[
            (1_000, 10),
            (1_001, 11),
            (1_002, 12),
            (1_003, 13),
            (1_004, 14),
        ]);
        let signer = reap_pm_core::EvmAddress::parse(ADDRESS).unwrap().bytes();
        let foreign_signer = reap_pm_core::EvmAddress::parse(FOREIGN_MAKER)
            .unwrap()
            .bytes();

        let closed = owner
            .preflight()
            .closed_only_observation(timestamp(), &mut clock)
            .await
            .unwrap();
        assert!(!closed.status().closed_only());
        assert_eq!(closed.receive_clock().monotonic_receive_ns(), 10);
        let closed_base = closed_only_observation_commitment(
            OriginMode::LocalEvidence,
            PmReadOnlySignatureType::Proxy,
            signer,
            closed_raw,
            closed.status(),
            closed.receive_clock(),
        );
        assert_eq!(closed_base, closed.commitment());
        assert_ne!(
            closed_base,
            closed_only_observation_commitment(
                OriginMode::Production,
                PmReadOnlySignatureType::Proxy,
                signer,
                closed_raw,
                closed.status(),
                closed.receive_clock(),
            )
        );
        assert_ne!(
            closed_base,
            closed_only_observation_commitment(
                OriginMode::LocalEvidence,
                PmReadOnlySignatureType::Eoa,
                signer,
                closed_raw,
                closed.status(),
                closed.receive_clock(),
            )
        );
        assert_ne!(
            closed_base,
            closed_only_observation_commitment(
                OriginMode::LocalEvidence,
                PmReadOnlySignatureType::Proxy,
                foreign_signer,
                closed_raw,
                closed.status(),
                closed.receive_clock(),
            )
        );
        let closed_raw_mutation = br#"{ "closed_only": false }"#;
        assert_ne!(
            closed_base,
            closed_only_observation_commitment(
                OriginMode::LocalEvidence,
                PmReadOnlySignatureType::Proxy,
                signer,
                closed_raw_mutation,
                closed.status(),
                closed.receive_clock(),
            )
        );
        let closed_status_mutation = parse_pm_closed_only(br#"{"closed_only":true}"#).unwrap();
        assert_ne!(
            closed_base,
            closed_only_observation_commitment(
                OriginMode::LocalEvidence,
                PmReadOnlySignatureType::Proxy,
                signer,
                closed_raw,
                closed_status_mutation,
                closed.receive_clock(),
            )
        );

        let (collateral, conditional, alternate_value) = {
            let mut account = owner.account();
            let collateral = account
                .collateral_balance_allowance_observation(timestamp(), &mut clock)
                .await
                .unwrap();
            let conditional = account
                .conditional_balance_allowance_observation(timestamp(), &mut clock)
                .await
                .unwrap();
            let alternate_value = account
                .collateral_balance_allowance_observation(timestamp(), &mut clock)
                .await
                .unwrap();
            (collateral, conditional, alternate_value)
        };
        assert_eq!(
            collateral.balance_allowance().asset(),
            crate::PmAccountAsset::Collateral
        );
        assert_eq!(collateral.receive_clock().monotonic_receive_ns(), 11);
        assert_eq!(
            conditional.balance_allowance().asset(),
            crate::PmAccountAsset::Conditional(scope().token())
        );
        let balance_base = crate::account::balance_allowance_observation_commitment(
            OriginMode::LocalEvidence,
            PmReadOnlySignatureType::Proxy,
            signer,
            balance_raw.as_bytes(),
            collateral.balance_allowance(),
            collateral.receive_clock(),
        );
        assert_eq!(balance_base, collateral.commitment());
        assert_ne!(
            balance_base,
            crate::account::balance_allowance_observation_commitment(
                OriginMode::Production,
                PmReadOnlySignatureType::Proxy,
                signer,
                balance_raw.as_bytes(),
                collateral.balance_allowance(),
                collateral.receive_clock(),
            )
        );
        assert_ne!(
            balance_base,
            crate::account::balance_allowance_observation_commitment(
                OriginMode::LocalEvidence,
                PmReadOnlySignatureType::Eoa,
                signer,
                balance_raw.as_bytes(),
                collateral.balance_allowance(),
                collateral.receive_clock(),
            )
        );
        assert_ne!(
            balance_base,
            crate::account::balance_allowance_observation_commitment(
                OriginMode::LocalEvidence,
                PmReadOnlySignatureType::Proxy,
                foreign_signer,
                balance_raw.as_bytes(),
                collateral.balance_allowance(),
                collateral.receive_clock(),
            )
        );
        let balance_raw_mutation = format!("{balance_raw} ");
        assert_ne!(
            balance_base,
            crate::account::balance_allowance_observation_commitment(
                OriginMode::LocalEvidence,
                PmReadOnlySignatureType::Proxy,
                signer,
                balance_raw_mutation.as_bytes(),
                collateral.balance_allowance(),
                collateral.receive_clock(),
            )
        );
        assert_ne!(
            balance_base,
            crate::account::balance_allowance_observation_commitment(
                OriginMode::LocalEvidence,
                PmReadOnlySignatureType::Proxy,
                signer,
                balance_raw.as_bytes(),
                conditional.balance_allowance(),
                collateral.receive_clock(),
            )
        );
        assert_ne!(
            balance_base,
            crate::account::balance_allowance_observation_commitment(
                OriginMode::LocalEvidence,
                PmReadOnlySignatureType::Proxy,
                signer,
                balance_raw.as_bytes(),
                alternate_value.balance_allowance(),
                collateral.receive_clock(),
            )
        );
        let later_receive = clock.observe_authenticated_read_complete().unwrap();
        assert_ne!(
            balance_base,
            crate::account::balance_allowance_observation_commitment(
                OriginMode::LocalEvidence,
                PmReadOnlySignatureType::Proxy,
                signer,
                balance_raw.as_bytes(),
                collateral.balance_allowance(),
                later_receive,
            )
        );

        assert!(
            !closed.into_status().closed_only(),
            "consuming the sealed carrier preserves the legacy typed status"
        );
        assert_eq!(
            collateral.into_balance_allowance().value().balance(),
            U256::from_u64(1_000)
        );

        let mut request_lines = Vec::new();
        for _ in 0..4 {
            request_lines.push(
                requests
                    .recv()
                    .await
                    .unwrap()
                    .lines()
                    .next()
                    .unwrap()
                    .to_owned(),
            );
        }
        assert_eq!(
            request_lines,
            [
                "GET /auth/ban-status/closed-only HTTP/1.1",
                "GET /balance-allowance?asset_type=COLLATERAL&signature_type=1 HTTP/1.1",
                "GET /balance-allowance?asset_type=CONDITIONAL&token_id=123&signature_type=1 HTTP/1.1",
                "GET /balance-allowance?asset_type=COLLATERAL&signature_type=1 HTTP/1.1",
            ]
        );
        task.await.unwrap();
        supervisor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn closed_only_redirect_and_both_body_oversize_modes_fail_without_retry() {
        let advertised = MockResponse {
            status: 200,
            body: Vec::new(),
            delay: Duration::ZERO,
            location: None,
            content_length: Some(MAX_PM_CLOSED_ONLY_BODY_BYTES + 1),
        };
        let streamed = MockResponse {
            status: 200,
            body: vec![b'x'; MAX_PM_CLOSED_ONLY_BODY_BYTES + 1],
            delay: Duration::ZERO,
            location: None,
            content_length: None,
        };
        let mut redirect = MockResponse::status(302);
        redirect.location = Some("/auth/ban-status/closed-only");
        let (origin, mut requests, task) = mock_server(vec![advertised, streamed, redirect]).await;
        let (mut owner, supervisor) = local_owner(&origin, Duration::from_secs(1));

        for _ in 0..2 {
            assert!(matches!(
                owner.preflight().closed_only(timestamp()).await,
                Err(PmLiveAdapterError::ResponseBodyTooLarge {
                    limit: MAX_PM_CLOSED_ONLY_BODY_BYTES
                })
            ));
        }
        assert!(matches!(
            owner.preflight().closed_only(timestamp()).await,
            Err(PmLiveAdapterError::Redirect { status: 302 })
        ));

        for _ in 0..3 {
            assert_eq!(
                requests.recv().await.unwrap().lines().next(),
                Some("GET /auth/ban-status/closed-only HTTP/1.1")
            );
        }
        task.await.unwrap();
        assert!(requests.try_recv().is_err());
        supervisor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn proxy_exact_order_matches_funder_while_l2_headers_keep_signer() {
        let proxy = reap_pm_core::EvmAddress::parse(FOREIGN_MAKER).unwrap();
        let correct = order(ORDER_ID, CONDITION, "123", FOREIGN_MAKER, API_KEY);
        let foreign = order(ORDER_ID, CONDITION, "123", ADDRESS, API_KEY);
        let (origin, mut requests, task) =
            mock_server(vec![MockResponse::ok(correct), MockResponse::ok(foreign)]).await;
        let (mut owner, supervisor) = local_proxy_owner(&origin, Duration::from_secs(1), proxy);
        let mut reconciliation = owner.reconciliation();
        let id = FixedOrderId::parse(ORDER_ID).unwrap();

        assert!(matches!(
            reconciliation
                .exact_local_order_detail(timestamp(), id)
                .await
                .unwrap(),
            PmExactOrderObservation::Present(_)
        ));
        assert_eq!(
            reconciliation
                .exact_local_order_detail(timestamp(), id)
                .await,
            Err(PmLiveAdapterError::ExactOrderMakerMismatch)
        );

        for _ in 0..2 {
            let request = requests.recv().await.unwrap().to_ascii_lowercase();
            assert!(request.contains(&format!("poly_address: {}", ADDRESS.to_ascii_lowercase())));
            assert!(!request.contains(&format!(
                "poly_address: {}",
                FOREIGN_MAKER.to_ascii_lowercase()
            )));
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
    async fn reconciliation_observations_are_terminal_clocked_and_cannot_be_resealed() {
        let open_order = order(ORDER_ID, CONDITION, "123", ADDRESS, API_KEY);
        let live_trade = trade(CONDITION, "123", ADDRESS, API_KEY);
        let exact_order = order(ORDER_ID, CONDITION, "123", ADDRESS, API_KEY);
        let missing_body = br#"{"error":"authenticated missing order"}"#.to_vec();
        let missing_length = missing_body.len();
        let responses = vec![
            MockResponse::ok(page(&open_order, "LTE=")),
            MockResponse::ok(page(&live_trade, "LTE=")),
            MockResponse::ok(page(&live_trade, "LTE=")),
            MockResponse::ok(exact_order),
            MockResponse {
                status: 404,
                body: missing_body,
                delay: Duration::ZERO,
                location: None,
                content_length: Some(missing_length),
            },
        ];
        let (origin, mut requests, task) = mock_server(responses).await;
        let (mut owner, supervisor) = local_owner(&origin, Duration::from_secs(1));
        let mut clock = private_read_clock(&[
            (1_000, 10),
            (1_001, 11),
            (1_002, 12),
            (1_003, 13),
            (1_004, 14),
        ]);
        let mut role = owner.reconciliation();

        #[cfg(feature = "test-support")]
        {
            let open_page =
                reap_polymarket_wire::parse_live_open_order_page(empty_page("LTE=").as_bytes())
                    .unwrap();
            let forged_open =
                crate::PmCompleteOpenOrdersCut::test_support_from_pages(Box::new([open_page]))
                    .unwrap();
            assert!(matches!(
                role.seal_complete_open_orders(forged_open),
                Err(PmLiveAdapterError::InvalidConfiguration(_))
            ));

            let trade_page =
                reap_polymarket_wire::parse_live_trade_page(empty_page("LTE=").as_bytes()).unwrap();
            let forged_trades =
                crate::PmCompleteTradesCut::test_support_from_pages(Box::new([trade_page]))
                    .unwrap();
            assert!(matches!(
                role.seal_complete_trades(forged_trades),
                Err(PmLiveAdapterError::InvalidConfiguration(_))
            ));
        }

        let open_cut = role
            .begin_open_orders_observation(timestamp(), &mut clock)
            .await
            .unwrap();
        let crate::PmOpenOrdersCutProgress::Complete(open_cut) = open_cut else {
            panic!("terminal open-order response must complete")
        };
        let post_fetch_clock = clock.observe_authenticated_read_complete().unwrap();
        assert_eq!(post_fetch_clock.monotonic_receive_ns(), 11);
        let open_observation = role.seal_complete_open_orders(open_cut).unwrap();
        assert_eq!(open_observation.receive_clock().monotonic_receive_ns(), 10);
        assert_ne!(open_observation.receive_clock(), post_fetch_clock);
        let consumed_cut = open_observation.into_cut();
        assert!(matches!(
            role.seal_complete_open_orders(consumed_cut),
            Err(PmLiveAdapterError::InvalidConfiguration(_))
        ));

        let legacy_trade_cut = role.begin_trades(timestamp()).await.unwrap();
        let crate::PmTradesCutProgress::Complete(legacy_trade_cut) = legacy_trade_cut else {
            panic!("terminal trade response must complete")
        };
        assert!(matches!(
            role.seal_complete_trades(legacy_trade_cut),
            Err(PmLiveAdapterError::InvalidConfiguration(_))
        ));

        let trade_cut = role
            .begin_trades_observation(timestamp(), &mut clock)
            .await
            .unwrap();
        let crate::PmTradesCutProgress::Complete(trade_cut) = trade_cut else {
            panic!("terminal trade response must complete")
        };
        let trade_observation = role.seal_complete_trades(trade_cut).unwrap();
        assert_eq!(trade_observation.receive_clock().monotonic_receive_ns(), 12);
        assert_eq!(trade_observation.cut().row_count(), 1);

        let id = FixedOrderId::parse(ORDER_ID).unwrap();
        let present = role
            .exact_local_order_detail_observation(timestamp(), id, &mut clock)
            .await
            .unwrap();
        assert!(matches!(
            present.classification(),
            PmExactOrderObservation::Present(_)
        ));
        assert_eq!(present.receive_clock().monotonic_receive_ns(), 13);
        let absent = role
            .exact_local_order_detail_observation(timestamp(), id, &mut clock)
            .await
            .unwrap();
        assert_eq!(absent.classification(), &PmExactOrderObservation::Absent);
        assert_eq!(absent.receive_clock().monotonic_receive_ns(), 14);
        assert_ne!(present.commitment(), absent.commitment());

        let mut request_lines = Vec::new();
        for _ in 0..5 {
            request_lines.push(
                requests
                    .recv()
                    .await
                    .unwrap()
                    .lines()
                    .next()
                    .unwrap()
                    .to_owned(),
            );
        }
        assert_eq!(
            request_lines,
            [
                "GET /data/orders?next_cursor=MA%3D%3D HTTP/1.1",
                "GET /data/trades?next_cursor=MA%3D%3D HTTP/1.1",
                "GET /data/trades?next_cursor=MA%3D%3D HTTP/1.1",
                &format!("GET /data/order/{ORDER_ID} HTTP/1.1"),
                &format!("GET /data/order/{ORDER_ID} HTTP/1.1"),
            ]
        );
        task.await.unwrap();
        supervisor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn reconciliation_commitments_exclude_api_key_but_owner_binding_stays_strict() {
        let responses_for = |owner: &str| {
            vec![
                MockResponse::ok(page(
                    &order(ORDER_ID, CONDITION, "123", ADDRESS, owner),
                    "LTE=",
                )),
                MockResponse::ok(page(&trade(CONDITION, "123", ADDRESS, owner), "LTE=")),
                MockResponse::ok(order(ORDER_ID, CONDITION, "123", ADDRESS, owner)),
            ]
        };
        let (first_origin, _first_requests, first_task) = mock_server(responses_for(API_KEY)).await;
        let (second_origin, _second_requests, second_task) =
            mock_server(responses_for(FOREIGN_API_KEY)).await;

        let first = reconciliation_commitments_for_api_key(&first_origin, API_KEY).await;
        let second = reconciliation_commitments_for_api_key(&second_origin, FOREIGN_API_KEY).await;
        assert_eq!(
            first, second,
            "durable commitments must not derive from the credential owner/API key"
        );
        first_task.await.unwrap();
        second_task.await.unwrap();

        let mismatched = order(ORDER_ID, CONDITION, "123", ADDRESS, FOREIGN_API_KEY);
        let (mismatch_origin, _mismatch_requests, mismatch_task) =
            mock_server(vec![MockResponse::ok(page(&mismatched, "LTE="))]).await;
        let (mut owner, supervisor) = local_owner(&mismatch_origin, Duration::from_secs(1));
        let mut clock = private_read_clock(&[(2_000, 20)]);
        assert!(matches!(
            owner
                .reconciliation()
                .begin_open_orders_observation(timestamp(), &mut clock)
                .await,
            Err(PmLiveAdapterError::CredentialOwnerMismatch)
        ));
        mismatch_task.await.unwrap();
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
