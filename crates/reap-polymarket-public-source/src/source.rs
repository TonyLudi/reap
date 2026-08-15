use std::{collections::BTreeSet, time::Duration};

use reap_polymarket_egress_binding::{PmFixedTlsPeerSelection, PmLocalEgressSelection};
use reqwest::{
    Client, StatusCode, Url,
    header::{ACCEPT, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_TYPE, HeaderMap, HeaderName},
    redirect::Policy,
};
use sha2::{Digest, Sha256};

use crate::{
    PM_DATA_API_PRODUCTION_ORIGIN, PmConfiguredTokenPosition, PmDataApiFixedPeerSourceError,
    PmDataApiPositionConfig, PmDataApiPositionEvidence, PmDataApiPositionObservationCommitment,
    PmDataApiPositionScope, PmDataApiReceiveClockObservation, PmExactPositionDecimal,
    PmMonitoredPositionObservation, PmPublicPositionError,
    config::OriginMode,
    position::{MAX_POSITION_PAGE_ROWS, ParsedPositionRow, parse_position_page},
};

pub const MAX_POSITION_PAGE_BODY_BYTES: usize = 1_048_576;
const POSITION_PAGE_LIMIT: u16 = 500;
const MAX_POSITION_OFFSET: u16 = 10_000;
const POSITION_OBSERVATION_COMMITMENT_DOMAIN: &[u8] =
    b"reap.polymarket.public-source.position-observation.v1\0";
const POSITION_OBSERVATION_METHOD: &[u8] = b"GET";
const POSITION_OBSERVATION_ROUTE: &[u8] = b"/positions";

#[derive(Clone, Copy)]
enum ClockSource {
    System,
    #[cfg(test)]
    Fixed(PmDataApiReceiveClockObservation),
}

impl ClockSource {
    fn observe(self) -> Result<PmDataApiReceiveClockObservation, PmPublicPositionError> {
        match self {
            Self::System => PmDataApiReceiveClockObservation::capture(),
            #[cfg(test)]
            Self::Fixed(observation) => Ok(observation),
        }
    }
}

struct PmDataApiPositionTransport {
    client: Client,
    origin: Url,
    scope: PmDataApiPositionScope,
    mode: OriginMode,
    expected_peer: Option<std::net::SocketAddr>,
}

/// Errors specific to obtaining a production-origin proof around an otherwise
/// unchanged bounded Data API position observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PmProductionDataApiPositionError {
    #[error("production Data API position evidence requires the fixed production origin")]
    OriginRequired,
    #[error(transparent)]
    Source(#[from] PmPublicPositionError),
}

/// Move-only proof that one bounded Data API position observation came from
/// this source's fixed production-origin mode.
///
/// This remains a monitored public projection, not atomic inventory
/// completeness or authority to sell, place, cancel, sign, or dispatch.
pub struct PmProductionDataApiPositionObservation {
    observation: PmMonitoredPositionObservation,
}

impl PmProductionDataApiPositionObservation {
    fn from_source(
        _production_origin: ProductionDataApiPositionOrigin,
        observation: PmMonitoredPositionObservation,
    ) -> Self {
        Self { observation }
    }

    #[must_use]
    pub const fn scope(&self) -> PmDataApiPositionScope {
        self.observation.scope()
    }

    #[must_use]
    pub const fn pages_observed(&self) -> u8 {
        self.observation.pages_observed()
    }

    #[must_use]
    pub const fn rows_observed(&self) -> u16 {
        self.observation.rows_observed()
    }

    #[must_use]
    pub const fn configured_token(&self) -> &PmConfiguredTokenPosition {
        self.observation.configured_token()
    }

    #[must_use]
    pub const fn completed_clock(&self) -> PmDataApiReceiveClockObservation {
        self.observation.completed_clock()
    }

    #[must_use]
    pub const fn commitment(&self) -> PmDataApiPositionObservationCommitment {
        self.observation.commitment()
    }
}

impl std::fmt::Debug for PmProductionDataApiPositionObservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(
            "PmProductionDataApiPositionObservation(<production-origin; monitored-only; sealed>)",
        )
    }
}

struct ProductionDataApiPositionOrigin;

impl ProductionDataApiPositionOrigin {
    fn verify(mode: OriginMode) -> Result<Self, PmProductionDataApiPositionError> {
        match mode {
            OriginMode::Production => Ok(Self),
            #[cfg(test)]
            OriginMode::NumericLoopback => Err(PmProductionDataApiPositionError::OriginRequired),
        }
    }
}

impl PmDataApiPositionTransport {
    fn new(config: &PmDataApiPositionConfig) -> Result<Self, PmPublicPositionError> {
        let mut builder = Client::builder()
            .connect_timeout(config.connect_timeout())
            .timeout(config.request_timeout())
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy();
        if let Some(local_egress) = config.local_egress() {
            #[cfg(target_os = "linux")]
            {
                builder = builder
                    .interface(local_egress.interface_name())
                    .local_address(local_egress.local_source_ip());
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = local_egress;
                return Err(PmPublicPositionError::SelectedLocalEgressUnsupported);
            }
        }
        if let Some(fixed_peer) = config.fixed_peer() {
            if config.origin().host_str() != Some(fixed_peer.dns_name()) {
                return Err(PmPublicPositionError::TransportBuild);
            }
            builder = builder.resolve(fixed_peer.dns_name(), fixed_peer.peer_addr());
        }
        if config.mode() == OriginMode::Production {
            builder = builder.https_only(true);
        }
        let client = builder
            .build()
            .map_err(|_| PmPublicPositionError::TransportBuild)?;
        Ok(Self {
            client,
            origin: config.origin().clone(),
            scope: config.scope(),
            mode: config.mode(),
            expected_peer: config.fixed_peer().map(PmFixedTlsPeerSelection::peer_addr),
        })
    }

    async fn fetch_page(&self, offset: u16) -> Result<Vec<u8>, PmPublicPositionError> {
        let url = self.position_url(offset);
        let mut response = self
            .client
            .get(url)
            .header(ACCEPT, "application/json")
            .header(ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(map_request_error)?;
        if self
            .expected_peer
            .is_some_and(|expected_peer| response.remote_addr() != Some(expected_peer))
        {
            return Err(PmPublicPositionError::RequestFailed);
        }
        let status = response.status();
        if status.is_redirection() {
            return Err(PmPublicPositionError::Redirect(status.as_u16()));
        }
        if status != StatusCode::OK {
            return Err(PmPublicPositionError::UnexpectedStatus(status.as_u16()));
        }
        validate_application_headers(response.headers())?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_POSITION_PAGE_BODY_BYTES as u64)
        {
            return Err(PmPublicPositionError::ResponseBodyTooLarge);
        }

        let capacity = response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or(0)
            .min(MAX_POSITION_PAGE_BODY_BYTES);
        let mut body = Vec::with_capacity(capacity);
        while let Some(chunk) = response.chunk().await.map_err(map_body_error)? {
            let next_length = body
                .len()
                .checked_add(chunk.len())
                .ok_or(PmPublicPositionError::ResponseBodyTooLarge)?;
            if next_length > MAX_POSITION_PAGE_BODY_BYTES {
                return Err(PmPublicPositionError::ResponseBodyTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    fn position_url(&self, offset: u16) -> Url {
        let mut url = self.origin.clone();
        url.set_path("/positions");
        url.set_query(None);
        url.query_pairs_mut()
            .append_pair("user", &self.scope.proxy_funder().to_string())
            .append_pair("market", &self.scope.condition().to_string())
            .append_pair("sizeThreshold", "0")
            .append_pair("limit", "500")
            .append_pair("offset", &offset.to_string())
            .append_pair("sortBy", "TOKENS")
            .append_pair("sortDirection", "DESC");
        url
    }
}

/// Credential-free capability for one fixed Data API position page walk.
///
/// It exposes no raw client, arbitrary request, route, origin, query, signing,
/// authentication, or mutation method.
pub struct PmDataApiCurrentPositionSource {
    transport: PmDataApiPositionTransport,
    clock: ClockSource,
}

impl PmDataApiCurrentPositionSource {
    #[must_use]
    pub const fn configured_scope(&self) -> PmDataApiPositionScope {
        self.transport.scope
    }

    #[must_use]
    pub const fn is_production(&self) -> bool {
        matches!(self.transport.mode, OriginMode::Production)
    }

    pub fn production(
        scope: PmDataApiPositionScope,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, PmPublicPositionError> {
        Self::new(
            PmDataApiPositionConfig::production(scope, connect_timeout, request_timeout)?,
            ClockSource::System,
        )
    }

    /// Construct the same fixed credential-free production source with a
    /// selected local interface/source IP. This adds no origin, route, query,
    /// method, authentication, or mutation input.
    pub fn production_on_selected_local_egress(
        scope: PmDataApiPositionScope,
        connect_timeout: Duration,
        request_timeout: Duration,
        local_egress: &PmLocalEgressSelection,
    ) -> Result<Self, PmPublicPositionError> {
        Self::new(
            PmDataApiPositionConfig::production_on_selected_local_egress(
                scope,
                connect_timeout,
                request_timeout,
                local_egress,
            )?,
            ClockSource::System,
        )
    }

    /// Construct the fixed credential-free production source on one exact
    /// reviewed TLS peer and one selected local interface/source IP.
    pub fn production_on_fixed_tls_peer_and_selected_local_egress(
        scope: PmDataApiPositionScope,
        connect_timeout: Duration,
        request_timeout: Duration,
        fixed_peer: &PmFixedTlsPeerSelection,
        local_egress: &PmLocalEgressSelection,
    ) -> Result<Self, PmDataApiFixedPeerSourceError> {
        Ok(Self::new(
            PmDataApiPositionConfig::production_on_fixed_tls_peer_and_selected_local_egress(
                scope,
                connect_timeout,
                request_timeout,
                fixed_peer,
                local_egress,
            )?,
            ClockSource::System,
        )?)
    }

    fn new(
        config: PmDataApiPositionConfig,
        clock: ClockSource,
    ) -> Result<Self, PmPublicPositionError> {
        Ok(Self {
            transport: PmDataApiPositionTransport::new(&config)?,
            clock,
        })
    }

    #[cfg(test)]
    fn numeric_loopback_evidence(
        origin: &str,
        scope: PmDataApiPositionScope,
        connect_timeout: Duration,
        request_timeout: Duration,
        clock: PmDataApiReceiveClockObservation,
    ) -> Result<Self, PmPublicPositionError> {
        Self::new(
            PmDataApiPositionConfig::numeric_loopback_evidence(
                origin,
                scope,
                connect_timeout,
                request_timeout,
            )?,
            ClockSource::Fixed(clock),
        )
    }

    #[cfg(test)]
    fn numeric_loopback_evidence_on_selected_local_egress(
        origin: &str,
        scope: PmDataApiPositionScope,
        connect_timeout: Duration,
        request_timeout: Duration,
        clock: PmDataApiReceiveClockObservation,
        local_egress: &PmLocalEgressSelection,
    ) -> Result<Self, PmPublicPositionError> {
        Self::new(
            PmDataApiPositionConfig::numeric_loopback_evidence_on_selected_local_egress(
                origin,
                scope,
                connect_timeout,
                request_timeout,
                local_egress,
            )?,
            ClockSource::Fixed(clock),
        )
    }

    #[cfg(test)]
    fn loopback_evidence_on_fixed_tls_peer_and_selected_local_egress(
        scope: PmDataApiPositionScope,
        connect_timeout: Duration,
        request_timeout: Duration,
        clock: PmDataApiReceiveClockObservation,
        fixed_peer: &PmFixedTlsPeerSelection,
        local_egress: &PmLocalEgressSelection,
    ) -> Result<Self, PmDataApiFixedPeerSourceError> {
        Ok(Self::new(
            PmDataApiPositionConfig::loopback_evidence_on_fixed_tls_peer_and_selected_local_egress(
                scope,
                connect_timeout,
                request_timeout,
                fixed_peer,
                local_egress,
            )?,
            ClockSource::Fixed(clock),
        )?)
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }

    /// Verify the private production mode before I/O, then fetch and seal one
    /// bounded configured-token observation as move-only production-origin
    /// evidence.
    pub async fn production_observe_configured_token(
        &self,
    ) -> Result<PmProductionDataApiPositionObservation, PmProductionDataApiPositionError> {
        let production_origin = ProductionDataApiPositionOrigin::verify(self.transport.mode)?;
        let observation = self.observe_configured_token().await?;
        Ok(PmProductionDataApiPositionObservation::from_source(
            production_origin,
            observation,
        ))
    }

    pub async fn observe_configured_token(
        &self,
    ) -> Result<PmMonitoredPositionObservation, PmPublicPositionError> {
        let scope = self.transport.scope;
        let mut seen_assets = BTreeSet::new();
        let mut configured_token = None;
        let mut offset = 0_u16;
        let mut commitment = PositionObservationCommitmentBuilder::new(scope, self.transport.mode);

        loop {
            let body = self.transport.fetch_page(offset).await?;
            let received_clock = self.clock.observe()?;
            let rows = parse_position_page(&body, scope)?;
            let page_rows = rows.len();
            commitment.observe_page(offset, &body, &rows, received_clock);

            for row in rows {
                if !seen_assets.insert(row.asset) {
                    return Err(PmPublicPositionError::DuplicateAsset);
                }
                if row.asset == scope.configured_token() {
                    configured_token = Some(row.evidence);
                }
            }

            if page_rows < MAX_POSITION_PAGE_ROWS {
                break;
            }
            if offset == MAX_POSITION_OFFSET {
                return Err(PmPublicPositionError::FullPageAtOffsetCap);
            }
            offset = offset
                .checked_add(POSITION_PAGE_LIMIT)
                .expect("fixed position offset bound");
        }

        let configured_token = configured_token
            .map_or(PmConfiguredTokenPosition::Absent, |position| {
                PmConfiguredTokenPosition::Present(Box::new(position))
            });
        let (pages_observed, rows_observed, completed_clock, commitment) =
            commitment.finish(&configured_token);
        Ok(PmMonitoredPositionObservation::new(
            scope,
            pages_observed,
            rows_observed,
            configured_token,
            completed_clock,
            commitment,
        ))
    }
}

struct PositionObservationCommitmentBuilder {
    hasher: Sha256,
    pages_observed: u8,
    rows_observed: u16,
    completed_clock: Option<PmDataApiReceiveClockObservation>,
}

impl PositionObservationCommitmentBuilder {
    fn new(scope: PmDataApiPositionScope, mode: OriginMode) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(POSITION_OBSERVATION_COMMITMENT_DOMAIN);
        update_commitment_bytes(&mut hasher, POSITION_OBSERVATION_METHOD);
        update_commitment_bytes(&mut hasher, POSITION_OBSERVATION_ROUTE);
        update_commitment_bytes(&mut hasher, PM_DATA_API_PRODUCTION_ORIGIN.as_bytes());
        hasher.update([match mode {
            OriginMode::Production => 0,
            #[cfg(test)]
            OriginMode::NumericLoopback => 1,
        }]);
        hasher.update(scope.proxy_funder().bytes());
        hasher.update(scope.condition().bytes());
        hasher.update(scope.configured_token().units().to_be_bytes());
        Self {
            hasher,
            pages_observed: 0,
            rows_observed: 0,
            completed_clock: None,
        }
    }

    fn observe_page(
        &mut self,
        offset: u16,
        body: &[u8],
        rows: &[ParsedPositionRow],
        received_clock: PmDataApiReceiveClockObservation,
    ) {
        let page_sequence = self.pages_observed;
        self.pages_observed = self
            .pages_observed
            .checked_add(1)
            .expect("fixed 21-page position bound");
        self.rows_observed = self
            .rows_observed
            .checked_add(u16::try_from(rows.len()).expect("500-row page bound"))
            .expect("fixed position aggregate bound");
        self.completed_clock = Some(received_clock);

        self.hasher.update([page_sequence]);
        self.hasher.update(offset.to_be_bytes());
        self.hasher
            .update(received_clock.unix_milliseconds().to_be_bytes());
        update_commitment_bytes(&mut self.hasher, body);
        self.hasher.update(
            u16::try_from(rows.len())
                .expect("500-row page bound")
                .to_be_bytes(),
        );
        for (row_index, row) in rows.iter().enumerate() {
            self.hasher.update(
                u16::try_from(row_index)
                    .expect("500-row page bound")
                    .to_be_bytes(),
            );
            update_position_evidence(&mut self.hasher, &row.evidence);
        }
    }

    fn finish(
        mut self,
        configured_token: &PmConfiguredTokenPosition,
    ) -> (
        u8,
        u16,
        PmDataApiReceiveClockObservation,
        PmDataApiPositionObservationCommitment,
    ) {
        self.hasher.update(self.pages_observed.to_be_bytes());
        self.hasher.update(self.rows_observed.to_be_bytes());
        match configured_token {
            PmConfiguredTokenPosition::Absent => self.hasher.update([0]),
            PmConfiguredTokenPosition::Present(position) => {
                self.hasher.update([1]);
                update_position_evidence(&mut self.hasher, position);
            }
        }
        let completed_clock = self
            .completed_clock
            .expect("one response is required to complete a page walk");
        let commitment = PmDataApiPositionObservationCommitment::from_source_bytes(
            self.hasher.finalize().into(),
        );
        (
            self.pages_observed,
            self.rows_observed,
            completed_clock,
            commitment,
        )
    }
}

fn update_position_evidence(hasher: &mut Sha256, evidence: &PmDataApiPositionEvidence) {
    hasher.update(evidence.asset().units().to_be_bytes());
    hasher.update(evidence.opposite_asset().units().to_be_bytes());
    for decimal in [
        evidence.size(),
        evidence.average_price(),
        evidence.initial_value(),
        evidence.current_value(),
        evidence.cash_pnl(),
        evidence.percent_pnl(),
        evidence.total_bought(),
        evidence.realized_pnl(),
        evidence.percent_realized_pnl(),
        evidence.current_price(),
    ] {
        update_exact_decimal(hasher, decimal);
    }
    hasher.update(evidence.outcome_index().to_be_bytes());
    update_commitment_bytes(hasher, evidence.outcome().as_bytes());
    update_commitment_bytes(hasher, evidence.opposite_outcome().as_bytes());
    hasher.update([
        u8::from(evidence.redeemable()),
        u8::from(evidence.mergeable()),
        u8::from(evidence.negative_risk()),
    ]);
}

fn update_exact_decimal(hasher: &mut Sha256, decimal: &PmExactPositionDecimal) {
    update_commitment_bytes(hasher, decimal.lexeme().as_bytes());
    hasher.update([u8::from(decimal.is_negative())]);
    hasher.update(decimal.coefficient().to_be_bytes());
    hasher.update(decimal.decimal_exponent().to_be_bytes());
}

fn update_commitment_bytes(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(
        u64::try_from(value.len())
            .expect("bounded source observation length")
            .to_be_bytes(),
    );
    hasher.update(value);
}

fn validate_application_headers(headers: &HeaderMap) -> Result<(), PmPublicPositionError> {
    let content_type = exactly_one_header(headers, CONTENT_TYPE)?
        .ok_or(PmPublicPositionError::InvalidApplicationHeaders)?;
    let content_type = content_type
        .to_str()
        .map_err(|_| PmPublicPositionError::InvalidApplicationHeaders)?;
    if content_type != content_type.trim() {
        return Err(PmPublicPositionError::InvalidApplicationHeaders);
    }
    let mut components = content_type.split(';');
    let essence = components
        .next()
        .ok_or(PmPublicPositionError::InvalidApplicationHeaders)?
        .trim();
    if !essence.eq_ignore_ascii_case("application/json") {
        return Err(PmPublicPositionError::InvalidApplicationHeaders);
    }
    if let Some(parameter) = components.next() {
        let (name, value) = parameter
            .trim()
            .split_once('=')
            .ok_or(PmPublicPositionError::InvalidApplicationHeaders)?;
        if !name.trim().eq_ignore_ascii_case("charset")
            || !value.trim().eq_ignore_ascii_case("utf-8")
        {
            return Err(PmPublicPositionError::InvalidApplicationHeaders);
        }
    }
    if components.next().is_some() {
        return Err(PmPublicPositionError::InvalidApplicationHeaders);
    }

    if let Some(content_encoding) = exactly_one_header(headers, CONTENT_ENCODING)? {
        let content_encoding = content_encoding
            .to_str()
            .map_err(|_| PmPublicPositionError::InvalidApplicationHeaders)?;
        if content_encoding != content_encoding.trim()
            || !content_encoding.eq_ignore_ascii_case("identity")
        {
            return Err(PmPublicPositionError::InvalidApplicationHeaders);
        }
    }
    Ok(())
}

fn exactly_one_header(
    headers: &HeaderMap,
    name: HeaderName,
) -> Result<Option<&reqwest::header::HeaderValue>, PmPublicPositionError> {
    let mut values = headers.get_all(name).iter();
    let first = values.next();
    if values.next().is_some() {
        return Err(PmPublicPositionError::InvalidApplicationHeaders);
    }
    Ok(first)
}

fn map_request_error(error: reqwest::Error) -> PmPublicPositionError {
    if error.is_timeout() {
        PmPublicPositionError::RequestTimeout
    } else {
        PmPublicPositionError::RequestFailed
    }
}

fn map_body_error(error: reqwest::Error) -> PmPublicPositionError {
    if error.is_timeout() {
        PmPublicPositionError::RequestTimeout
    } else {
        PmPublicPositionError::ResponseBodyRead
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, net::IpAddr, ops::Deref};

    use reap_pm_core::{EvmAddress, PmConditionId, PmTokenId, U256};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::mpsc,
        task::JoinHandle,
    };

    use super::*;
    use crate::position::tests::{CONDITION, FUNDER, row, scope};

    struct MockResponse {
        status: u16,
        content_type: &'static str,
        content_encoding: Option<&'static str>,
        declared_length: Option<usize>,
        body: String,
        location: Option<&'static str>,
    }

    struct RecordedRequest {
        raw: String,
        peer_ip: IpAddr,
    }

    impl Deref for RecordedRequest {
        type Target = str;

        fn deref(&self) -> &Self::Target {
            &self.raw
        }
    }

    impl MockResponse {
        fn ok(body: String) -> Self {
            Self {
                status: 200,
                content_type: "application/json; charset=utf-8",
                content_encoding: None,
                declared_length: None,
                body,
                location: None,
            }
        }
    }

    async fn mock_server(
        responses: Vec<MockResponse>,
    ) -> (
        String,
        mpsc::UnboundedReceiver<RecordedRequest>,
        JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (requests_rx, task) = serve_mock(listener, responses);
        (
            format!("http://127.0.0.1:{}", address.port()),
            requests_rx,
            task,
        )
    }

    #[cfg(target_os = "linux")]
    async fn fixed_peer_mock_server(
        responses: Vec<MockResponse>,
    ) -> (
        std::net::SocketAddr,
        TcpListener,
        mpsc::UnboundedReceiver<RecordedRequest>,
        JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let peer_addr = listener.local_addr().unwrap();
        let decoy = TcpListener::bind(("127.0.0.3", peer_addr.port()))
            .await
            .unwrap();
        let (requests, task) = serve_mock(listener, responses);
        (peer_addr, decoy, requests, task)
    }

    fn serve_mock(
        listener: TcpListener,
        responses: Vec<MockResponse>,
    ) -> (mpsc::UnboundedReceiver<RecordedRequest>, JoinHandle<()>) {
        let (requests_tx, requests_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut responses = VecDeque::from(responses);
            while let Some(response) = responses.pop_front() {
                let (mut stream, peer) = listener.accept().await.unwrap();
                let mut raw = Vec::new();
                let mut chunk = [0_u8; 4_096];
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
                let _ = requests_tx.send(RecordedRequest {
                    raw: String::from_utf8(raw).unwrap(),
                    peer_ip: peer.ip(),
                });

                let reason = match response.status {
                    200 => "OK",
                    302 => "Found",
                    503 => "Service Unavailable",
                    _ => "Mock",
                };
                let mut headers = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nConnection: close\r\n",
                    response.status, reason, response.content_type
                );
                if let Some(encoding) = response.content_encoding {
                    headers.push_str(&format!("Content-Encoding: {encoding}\r\n"));
                }
                if let Some(location) = response.location {
                    headers.push_str(&format!("Location: {location}\r\n"));
                }
                headers.push_str(&format!(
                    "Content-Length: {}\r\n\r\n",
                    response.declared_length.unwrap_or(response.body.len())
                ));
                stream.write_all(headers.as_bytes()).await.unwrap();
                if response.declared_length.is_none() {
                    stream.write_all(response.body.as_bytes()).await.unwrap();
                }
            }
        });
        (requests_rx, task)
    }

    fn source(origin: &str, token: u64) -> PmDataApiCurrentPositionSource {
        PmDataApiCurrentPositionSource::numeric_loopback_evidence(
            origin,
            scope(token),
            Duration::from_secs(2),
            Duration::from_secs(5),
            PmDataApiReceiveClockObservation::for_loopback_evidence(1_700_000_000_123),
        )
        .unwrap()
    }

    #[cfg(target_os = "linux")]
    fn selected_source(origin: &str, token: u64) -> PmDataApiCurrentPositionSource {
        let selection =
            PmLocalEgressSelection::loopback_evidence("lo", "127.0.0.2".parse().unwrap()).unwrap();
        PmDataApiCurrentPositionSource::numeric_loopback_evidence_on_selected_local_egress(
            origin,
            scope(token),
            Duration::from_secs(2),
            Duration::from_secs(5),
            PmDataApiReceiveClockObservation::for_loopback_evidence(1_700_000_000_123),
            &selection,
        )
        .unwrap()
    }

    fn page(first: u64, count: usize, configured: Option<(u64, &str)>) -> String {
        let rows = (first..first + count as u64)
            .map(|asset| {
                configured
                    .filter(|(configured_asset, _)| *configured_asset == asset)
                    .map_or_else(|| row(asset, "1"), |(_, size)| row(asset, size))
            })
            .collect::<Vec<_>>()
            .join(",");
        format!("[{rows}]")
    }

    fn commitment_for_pages(
        scope: PmDataApiPositionScope,
        pages: &[(u16, String, u64)],
        mode: OriginMode,
    ) -> PmDataApiPositionObservationCommitment {
        let mut builder = PositionObservationCommitmentBuilder::new(scope, mode);
        let mut configured = None;
        for (offset, body, clock) in pages {
            let rows = parse_position_page(body.as_bytes(), scope).unwrap();
            builder.observe_page(
                *offset,
                body.as_bytes(),
                &rows,
                PmDataApiReceiveClockObservation::for_loopback_evidence(*clock),
            );
            for parsed in rows {
                if parsed.asset == scope.configured_token() {
                    configured = Some(parsed.evidence);
                }
            }
        }
        let configured = configured.map_or(PmConfiguredTokenPosition::Absent, |position| {
            PmConfiguredTokenPosition::Present(Box::new(position))
        });
        builder.finish(&configured).3
    }

    #[tokio::test]
    async fn exact_pagination_route_and_present_zero_are_preserved() {
        let responses = vec![
            MockResponse::ok(page(1, 500, None)),
            MockResponse::ok(page(501, 1, Some((501, "0")))),
        ];
        let (origin, mut requests, server) = mock_server(responses).await;
        let observation = source(&origin, 501)
            .observe_configured_token()
            .await
            .unwrap();
        assert_eq!(observation.pages_observed(), 2);
        assert_eq!(observation.rows_observed(), 501);
        assert_eq!(
            observation.completed_clock().unix_milliseconds(),
            1_700_000_000_123
        );
        assert_eq!(observation.commitment().bytes().len(), 32);
        assert!(!observation.production_order_entry_authorized());
        let present = observation.configured_token().as_present().unwrap();
        assert!(present.size().is_zero());
        assert_eq!(present.size().lexeme(), "0");

        for expected_offset in [0, 500] {
            let request = requests.recv().await.unwrap();
            let first_line = request.lines().next().unwrap();
            assert_eq!(
                first_line,
                format!(
                    "GET /positions?user={FUNDER}&market={CONDITION}&sizeThreshold=0&limit=500&offset={expected_offset}&sortBy=TOKENS&sortDirection=DESC HTTP/1.1"
                )
            );
            let lowercase = request.to_ascii_lowercase();
            assert!(lowercase.contains("accept: application/json\r\n"));
            assert!(lowercase.contains("accept-encoding: identity\r\n"));
            assert!(!lowercase.contains("poly_"));
        }
        server.await.unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn selected_loopback_interface_and_source_ip_preserve_the_fixed_position_get() {
        let (origin, mut requests, server) =
            mock_server(vec![MockResponse::ok(format!("[{}]", row(501, "0")))]).await;
        let observation = selected_source(&origin, 501)
            .observe_configured_token()
            .await
            .unwrap();
        assert_eq!(observation.rows_observed(), 1);
        let request = requests.recv().await.unwrap();
        assert!(request.starts_with("GET /positions?"));
        assert_eq!(request.peer_ip, "127.0.0.2".parse::<IpAddr>().unwrap());
        server.await.unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn fixed_peer_keeps_hostname_source_ip_exact_peer_and_avoids_decoy() {
        let (peer_addr, decoy, mut requests, server) =
            fixed_peer_mock_server(vec![MockResponse::ok(format!("[{}]", row(501, "0")))]).await;
        let fixed_peer =
            PmFixedTlsPeerSelection::loopback_evidence("data-api-source.test", peer_addr).unwrap();
        let local_egress =
            PmLocalEgressSelection::loopback_evidence("lo", "127.0.0.2".parse().unwrap()).unwrap();
        let source = PmDataApiCurrentPositionSource::loopback_evidence_on_fixed_tls_peer_and_selected_local_egress(
            scope(501),
            Duration::from_secs(2),
            Duration::from_secs(5),
            PmDataApiReceiveClockObservation::for_loopback_evidence(1_700_000_000_123),
            &fixed_peer,
            &local_egress,
        )
        .unwrap();
        assert_eq!(source.transport.expected_peer, Some(peer_addr));

        let observation = source.observe_configured_token().await.unwrap();
        assert_eq!(observation.rows_observed(), 1);
        let request = requests.recv().await.unwrap();
        assert_eq!(request.peer_ip, "127.0.0.2".parse::<IpAddr>().unwrap());
        let expected_host = format!("host: data-api-source.test:{}", peer_addr.port());
        assert!(
            request
                .lines()
                .any(|line| line.eq_ignore_ascii_case(&expected_host))
        );
        server.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), decoy.accept())
                .await
                .is_err()
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn fixed_peer_response_rejects_a_different_expected_remote_socket() {
        let (peer_addr, _decoy, _requests, server) =
            fixed_peer_mock_server(vec![MockResponse::ok(format!("[{}]", row(501, "0")))]).await;
        let fixed_peer =
            PmFixedTlsPeerSelection::loopback_evidence("data-api-source.test", peer_addr).unwrap();
        let local_egress =
            PmLocalEgressSelection::loopback_evidence("lo", "127.0.0.2".parse().unwrap()).unwrap();
        let mut source = PmDataApiCurrentPositionSource::loopback_evidence_on_fixed_tls_peer_and_selected_local_egress(
            scope(501),
            Duration::from_secs(2),
            Duration::from_secs(5),
            PmDataApiReceiveClockObservation::for_loopback_evidence(1_700_000_000_123),
            &fixed_peer,
            &local_egress,
        )
        .unwrap();
        source.transport.expected_peer = Some(std::net::SocketAddr::new(
            "127.0.0.3".parse().unwrap(),
            peer_addr.port(),
        ));

        assert!(matches!(
            source.observe_configured_token().await,
            Err(PmPublicPositionError::RequestFailed)
        ));
        server.await.unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn selected_production_constructor_changes_no_fixed_source_authority() {
        let selection =
            PmLocalEgressSelection::production("pm-tunnel0", "192.0.2.10".parse().unwrap())
                .unwrap();
        assert!(!selection.production_order_entry_authorized());
        PmDataApiCurrentPositionSource::production_on_selected_local_egress(
            scope(501),
            Duration::from_secs(1),
            Duration::from_secs(2),
            &selection,
        )
        .expect("client construction does not perform network I/O");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn selected_production_and_loopback_modes_cannot_cross_source_constructors() {
        let production =
            PmLocalEgressSelection::production("pm0", "192.0.2.10".parse().unwrap()).unwrap();
        assert!(matches!(
            PmDataApiPositionConfig::numeric_loopback_evidence_on_selected_local_egress(
                "http://127.0.0.1:9",
                scope(501),
                Duration::from_secs(1),
                Duration::from_secs(2),
                &production,
            ),
            Err(PmPublicPositionError::LocalEgressSelection(
                reap_polymarket_egress_binding::PmLocalEgressSelectionError::LoopbackEvidenceSelectionRequired
            ))
        ));

        let loopback =
            PmLocalEgressSelection::loopback_evidence("lo", "127.0.0.2".parse().unwrap()).unwrap();
        assert!(matches!(
            PmDataApiCurrentPositionSource::production_on_selected_local_egress(
                scope(501),
                Duration::from_secs(1),
                Duration::from_secs(2),
                &loopback,
            ),
            Err(PmPublicPositionError::LocalEgressSelection(
                reap_polymarket_egress_binding::PmLocalEgressSelectionError::ProductionSelectionRequired
            ))
        ));

        let production_peer =
            PmFixedTlsPeerSelection::production("data-api.polymarket.com", "8.8.8.8").unwrap();
        assert!(matches!(
            PmDataApiCurrentPositionSource::production_on_fixed_tls_peer_and_selected_local_egress(
                scope(501),
                Duration::from_secs(1),
                Duration::from_secs(2),
                &production_peer,
                &loopback,
            ),
            Err(PmDataApiFixedPeerSourceError::LocalEgressSelection(
                reap_polymarket_egress_binding::PmLocalEgressSelectionError::ProductionSelectionRequired
            ))
        ));

        let loopback_peer = PmFixedTlsPeerSelection::loopback_evidence(
            "data-api-source.test",
            "127.0.0.1:9".parse().unwrap(),
        )
        .unwrap();
        assert!(matches!(
            PmDataApiPositionConfig::loopback_evidence_on_fixed_tls_peer_and_selected_local_egress(
                scope(501),
                Duration::from_secs(1),
                Duration::from_secs(2),
                &loopback_peer,
                &production,
            ),
            Err(PmDataApiFixedPeerSourceError::LocalEgressSelection(
                reap_polymarket_egress_binding::PmLocalEgressSelectionError::LoopbackEvidenceSelectionRequired
            ))
        ));

        let wrong_host_peer =
            PmFixedTlsPeerSelection::production("polygon.drpc.org", "8.8.8.8").unwrap();
        assert!(matches!(
            PmDataApiCurrentPositionSource::production_on_fixed_tls_peer_and_selected_local_egress(
                scope(501),
                Duration::from_secs(1),
                Duration::from_secs(2),
                &wrong_host_peer,
                &production,
            ),
            Err(PmDataApiFixedPeerSourceError::DnsNameMismatch)
        ));

        let ipv6_local =
            PmLocalEgressSelection::production("pm0", "2001:4860:4860::8844".parse().unwrap())
                .unwrap();
        assert!(matches!(
            PmDataApiCurrentPositionSource::production_on_fixed_tls_peer_and_selected_local_egress(
                scope(501),
                Duration::from_secs(1),
                Duration::from_secs(2),
                &production_peer,
                &ipv6_local,
            ),
            Err(PmDataApiFixedPeerSourceError::FixedTlsPeerSelection(
                reap_polymarket_egress_binding::PmFixedTlsPeerSelectionError::AddressFamilyMismatch
            ))
        ));
    }

    #[tokio::test]
    async fn production_wrapper_preserves_the_exact_observation_and_redacts_debug() {
        let (origin, _, server) =
            mock_server(vec![MockResponse::ok(format!("[{}]", row(501, "0")))]).await;
        let observation = source(&origin, 501)
            .observe_configured_token()
            .await
            .unwrap();
        let expected_scope = observation.scope();
        let expected_pages = observation.pages_observed();
        let expected_rows = observation.rows_observed();
        let expected_clock = observation.completed_clock();
        let expected_commitment = observation.commitment();
        let production = PmProductionDataApiPositionObservation::from_source(
            ProductionDataApiPositionOrigin,
            observation,
        );

        assert_eq!(production.scope(), expected_scope);
        assert_eq!(production.pages_observed(), expected_pages);
        assert_eq!(production.rows_observed(), expected_rows);
        assert!(
            production
                .configured_token()
                .as_present()
                .unwrap()
                .size()
                .is_zero()
        );
        assert_eq!(production.completed_clock(), expected_clock);
        assert_eq!(production.commitment(), expected_commitment);
        assert_eq!(
            format!("{production:?}"),
            "PmProductionDataApiPositionObservation(<production-origin; monitored-only; sealed>)"
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn numeric_loopback_source_cannot_issue_production_wrapper_before_io() {
        assert!(matches!(
            source("http://127.0.0.1:9", 501)
                .production_observe_configured_token()
                .await,
            Err(PmProductionDataApiPositionError::OriginRequired)
        ));
    }

    #[test]
    fn production_origin_proof_accepts_only_production_mode() {
        assert!(ProductionDataApiPositionOrigin::verify(OriginMode::Production).is_ok());
        assert!(matches!(
            ProductionDataApiPositionOrigin::verify(OriginMode::NumericLoopback),
            Err(PmProductionDataApiPositionError::OriginRequired)
        ));
        assert_eq!(
            PmProductionDataApiPositionError::from(PmPublicPositionError::DuplicateAsset),
            PmProductionDataApiPositionError::Source(PmPublicPositionError::DuplicateAsset)
        );
    }

    #[tokio::test]
    async fn empty_walk_reports_absent_not_zero() {
        let (origin, _, server) = mock_server(vec![MockResponse::ok("[]".to_owned())]).await;
        let observation = source(&origin, 77)
            .observe_configured_token()
            .await
            .unwrap();
        assert!(observation.configured_token().is_absent());
        assert_eq!(observation.rows_observed(), 0);
        server.await.unwrap();
    }

    #[test]
    fn observation_commitment_binds_scope_page_order_counts_values_and_receive_clock() {
        let baseline_body = format!("[{}]", row(7, "1"));
        let baseline_pages = [(0, baseline_body.clone(), 1_700_000_000_123)];
        let exact = commitment_for_pages(scope(7), &baseline_pages, OriginMode::NumericLoopback);
        assert_eq!(exact.bytes().len(), 32);
        assert_eq!(exact.to_string().len(), 66);

        let row_mutations = [
            ("\"asset\":\"7\"", "\"asset\":\"8\""),
            (
                "\"oppositeAsset\":\"1000007\"",
                "\"oppositeAsset\":\"1000008\"",
            ),
            ("\"size\":1", "\"size\":2"),
            ("\"avgPrice\":0.42", "\"avgPrice\":0.43"),
            ("\"initialValue\":1.25", "\"initialValue\":1.26"),
            ("\"currentValue\":1.5", "\"currentValue\":1.6"),
            ("\"cashPnl\":-0.25", "\"cashPnl\":-0.24"),
            ("\"percentPnl\":-20", "\"percentPnl\":-19"),
            ("\"totalBought\":3e0", "\"totalBought\":4e0"),
            ("\"realizedPnl\":0", "\"realizedPnl\":1"),
            ("\"percentRealizedPnl\":0", "\"percentRealizedPnl\":1"),
            ("\"curPrice\":0.5", "\"curPrice\":0.6"),
            ("\"outcomeIndex\":0", "\"outcomeIndex\":1"),
            ("\"outcome\":\"Yes\"", "\"outcome\":\"Up\""),
            ("\"oppositeOutcome\":\"No\"", "\"oppositeOutcome\":\"Down\""),
            ("\"redeemable\":false", "\"redeemable\":true"),
            ("\"mergeable\":false", "\"mergeable\":true"),
            ("\"negativeRisk\":false", "\"negativeRisk\":true"),
            (
                "\"title\":\"Synthetic market\"",
                "\"title\":\"Other market\"",
            ),
            ("\"slug\":\"synthetic\"", "\"slug\":\"other\""),
            (
                "\"icon\":\"https://example.invalid/icon\"",
                "\"icon\":\"https://example.invalid/other\"",
            ),
            (
                "\"eventSlug\":\"synthetic-event\"",
                "\"eventSlug\":\"other-event\"",
            ),
            (
                "\"endDate\":\"2030-01-01T00:00:00Z\"",
                "\"endDate\":\"2030-01-02T00:00:00Z\"",
            ),
        ];
        for (from, to) in row_mutations {
            let changed_body = baseline_body.replace(from, to);
            assert_ne!(changed_body, baseline_body, "missing fixture field {from}");
            let changed = commitment_for_pages(
                scope(7),
                &[(0, changed_body, 1_700_000_000_123)],
                OriginMode::NumericLoopback,
            );
            assert_ne!(changed, exact, "unbound observation field {from}");
        }

        let changed_funder = "0x3333333333333333333333333333333333333333";
        let changed_condition =
            "0x4444444444444444444444444444444444444444444444444444444444444444";
        let changed_scope = PmDataApiPositionScope::new(
            EvmAddress::parse(changed_funder).unwrap(),
            PmConditionId::parse(changed_condition).unwrap(),
            PmTokenId::new(U256::from_u64(7)).unwrap(),
        );
        let changed_scope_body = baseline_body
            .replace(FUNDER, changed_funder)
            .replace(CONDITION, changed_condition);
        assert_ne!(
            commitment_for_pages(
                changed_scope,
                &[(0, changed_scope_body, 1_700_000_000_123)],
                OriginMode::NumericLoopback,
            ),
            exact
        );
        assert_ne!(
            commitment_for_pages(scope(8), &baseline_pages, OriginMode::NumericLoopback),
            exact
        );

        let two_rows = format!("[{},{}]", row(7, "1"), row(8, "1"));
        let reversed_rows = format!("[{},{}]", row(8, "1"), row(7, "1"));
        let two_row_commitment = commitment_for_pages(
            scope(7),
            &[(0, two_rows, 1_700_000_000_123)],
            OriginMode::NumericLoopback,
        );
        assert_ne!(two_row_commitment, exact, "row count is unbound");
        assert_ne!(
            commitment_for_pages(
                scope(7),
                &[(0, reversed_rows, 1_700_000_000_123)],
                OriginMode::NumericLoopback,
            ),
            two_row_commitment,
            "row order is unbound"
        );
        assert_ne!(
            commitment_for_pages(
                scope(7),
                &[
                    (0, format!("[{}]", row(7, "1")), 1_700_000_000_122),
                    (500, format!("[{}]", row(8, "1")), 1_700_000_000_123),
                ],
                OriginMode::NumericLoopback,
            ),
            two_row_commitment,
            "page count and boundaries are unbound"
        );
        assert_ne!(
            commitment_for_pages(
                scope(7),
                &[(0, baseline_body.clone(), 1_700_000_000_124)],
                OriginMode::NumericLoopback,
            ),
            exact,
            "receive clock is unbound"
        );
        assert_ne!(
            commitment_for_pages(scope(7), &baseline_pages, OriginMode::Production),
            exact,
            "source mode is unbound"
        );
        assert_ne!(
            commitment_for_pages(
                scope(7),
                &[(0, format!("[ {} ]", row(7, "1")), 1_700_000_000_123)],
                OriginMode::NumericLoopback,
            ),
            exact,
            "exact received body is unbound"
        );
    }

    #[tokio::test]
    async fn duplicate_asset_across_pages_fails_the_whole_observation() {
        let responses = vec![
            MockResponse::ok(page(1, 500, None)),
            MockResponse::ok(page(1, 1, None)),
        ];
        let (origin, _, server) = mock_server(responses).await;
        assert_eq!(
            source(&origin, 77)
                .observe_configured_token()
                .await
                .unwrap_err(),
            PmPublicPositionError::DuplicateAsset
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn full_page_at_offset_cap_fails_closed() {
        let responses = (0..=20)
            .map(|page_index| MockResponse::ok(page(1 + page_index * 500, 500, None)))
            .collect();
        let (origin, mut requests, server) = mock_server(responses).await;
        assert_eq!(
            source(&origin, 20_000)
                .observe_configured_token()
                .await
                .unwrap_err(),
            PmPublicPositionError::FullPageAtOffsetCap
        );
        let mut last = String::new();
        while let Ok(request) = requests.try_recv() {
            last = request.raw;
        }
        assert!(last.lines().next().unwrap().contains("offset=10000"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn redirect_oversize_and_non_json_headers_fail_without_retry() {
        let cases = [
            (
                MockResponse {
                    status: 302,
                    content_type: "application/json",
                    content_encoding: None,
                    declared_length: None,
                    body: String::new(),
                    location: Some("https://example.invalid/positions"),
                },
                PmPublicPositionError::Redirect(302),
            ),
            (
                MockResponse {
                    status: 200,
                    content_type: "application/json",
                    content_encoding: None,
                    declared_length: Some(MAX_POSITION_PAGE_BODY_BYTES + 1),
                    body: String::new(),
                    location: None,
                },
                PmPublicPositionError::ResponseBodyTooLarge,
            ),
            (
                MockResponse {
                    status: 200,
                    content_type: "text/plain",
                    content_encoding: None,
                    declared_length: None,
                    body: "[]".to_owned(),
                    location: None,
                },
                PmPublicPositionError::InvalidApplicationHeaders,
            ),
            (
                MockResponse {
                    status: 200,
                    content_type: "application/json",
                    content_encoding: Some("gzip"),
                    declared_length: None,
                    body: "[]".to_owned(),
                    location: None,
                },
                PmPublicPositionError::InvalidApplicationHeaders,
            ),
        ];

        for (response, expected) in cases {
            let (origin, mut requests, server) = mock_server(vec![response]).await;
            assert_eq!(
                source(&origin, 77)
                    .observe_configured_token()
                    .await
                    .unwrap_err(),
                expected
            );
            assert!(requests.recv().await.is_some());
            assert!(requests.try_recv().is_err(), "unexpected retry or redirect");
            server.await.unwrap();
        }
    }

    #[test]
    fn fixed_url_builder_has_no_route_or_query_input() {
        let config = PmDataApiPositionConfig::numeric_loopback_evidence(
            "http://127.0.0.1:1234",
            scope(77),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap();
        let transport = PmDataApiPositionTransport::new(&config).unwrap();
        assert_eq!(
            transport.position_url(10_000).as_str(),
            format!(
                "http://127.0.0.1:1234/positions?user={FUNDER}&market={CONDITION}&sizeThreshold=0&limit=500&offset=10000&sortBy=TOKENS&sortDirection=DESC"
            )
        );
    }
}
