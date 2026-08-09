use reap_polymarket_auth::L2Timestamp;
use reap_polymarket_wire::{
    MAX_PUBLIC_REST_BODY_BYTES, PmBookMarketBinding, PmBookParserConfig, parse_server_time,
};
use sha2::{Digest as _, Sha256};

use crate::{
    PM_CLOB_PRODUCTION_ORIGIN, PmLiveAdapterError, PmMutationServerTimeProductClock,
    PmPendingMutationServerTime, PmProductClockError, PmPublicHttpConfig, PmPublicHttpProductClock,
    PmReadServerTime, PmReadServerTimeProductClock, PmRestBookDeliveryError, PmRestResponseClock,
    config::OriginMode,
    http_transport::{PmHttpTransport, PmPublicRoute},
};

const MAX_SERVER_TIME_BODY_BYTES: usize = 64;
const READ_SERVER_TIME_OBSERVATION_COMMITMENT_DOMAIN: &[u8] =
    b"reap.pm.live-adapter.read-server-time-observation.v1\0";
const MUTATION_SERVER_TIME_OBSERVATION_COMMITMENT_DOMAIN: &[u8] =
    b"reap.pm.live-adapter.mutation-server-time-observation.v1\0";

/// SHA-256 commitment to one fixed `/time` response used for an authenticated
/// read. Construction is private to the source role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmReadServerTimeObservationCommitment([u8; 32]);

impl PmReadServerTimeObservationCommitment {
    const fn from_source_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// SHA-256 commitment to one fixed `/time` response reserved for mutation
/// admission. Its distinct type/domain prevents read-time substitution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmMutationServerTimeObservationCommitment([u8; 32]);

impl PmMutationServerTimeObservationCommitment {
    const fn from_source_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Sealed source observation for an authenticated-read server time.
///
/// The parsed value and receive edge are inspectable, but the underlying
/// proof remains move-only. Consumers must call [`Self::into_read_server_time`]
/// and remain subject to the existing freshness check at authentication.
pub struct PmReadServerTimeObservation {
    parsed_l2_timestamp: L2Timestamp,
    receive_clock: PmRestResponseClock,
    commitment: PmReadServerTimeObservationCommitment,
    proof: PmReadServerTime,
}

impl PmReadServerTimeObservation {
    fn from_source(
        parsed_l2_timestamp: L2Timestamp,
        receive_clock: PmRestResponseClock,
        commitment: PmReadServerTimeObservationCommitment,
        proof: PmReadServerTime,
    ) -> Self {
        Self {
            parsed_l2_timestamp,
            receive_clock,
            commitment,
            proof,
        }
    }

    #[must_use]
    pub const fn parsed_l2_timestamp(&self) -> L2Timestamp {
        self.parsed_l2_timestamp
    }

    #[must_use]
    pub const fn receive_clock(&self) -> PmRestResponseClock {
        self.receive_clock
    }

    #[must_use]
    pub const fn commitment(&self) -> PmReadServerTimeObservationCommitment {
        self.commitment
    }

    #[must_use]
    pub fn into_read_server_time(self) -> PmReadServerTime {
        self.proof
    }
}

impl std::fmt::Debug for PmReadServerTimeObservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmReadServerTimeObservation")
            .field("parsed_l2_timestamp", &self.parsed_l2_timestamp)
            .field("receive_clock", &self.receive_clock)
            .field("commitment", &self.commitment)
            .field("proof", &"<opaque>")
            .finish()
    }
}

/// Sealed source observation for one pending mutation server time.
///
/// Consuming this carrier yields only the existing pending proof, which must
/// still pass the same-domain freshness validator before mutation admission.
pub struct PmMutationServerTimeObservation {
    parsed_l2_timestamp: L2Timestamp,
    receive_clock: PmRestResponseClock,
    commitment: PmMutationServerTimeObservationCommitment,
    proof: PmPendingMutationServerTime,
}

impl PmMutationServerTimeObservation {
    fn from_source(
        parsed_l2_timestamp: L2Timestamp,
        receive_clock: PmRestResponseClock,
        commitment: PmMutationServerTimeObservationCommitment,
        proof: PmPendingMutationServerTime,
    ) -> Self {
        Self {
            parsed_l2_timestamp,
            receive_clock,
            commitment,
            proof,
        }
    }

    #[must_use]
    pub const fn parsed_l2_timestamp(&self) -> L2Timestamp {
        self.parsed_l2_timestamp
    }

    #[must_use]
    pub const fn receive_clock(&self) -> PmRestResponseClock {
        self.receive_clock
    }

    #[must_use]
    pub const fn commitment(&self) -> PmMutationServerTimeObservationCommitment {
        self.commitment
    }

    #[must_use]
    pub fn into_pending_mutation_server_time(self) -> PmPendingMutationServerTime {
        self.proof
    }
}

impl std::fmt::Debug for PmMutationServerTimeObservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmMutationServerTimeObservation")
            .field("parsed_l2_timestamp", &self.parsed_l2_timestamp)
            .field("receive_clock", &self.receive_clock)
            .field("commitment", &self.commitment)
            .field("proof", &"<opaque>")
            .finish()
    }
}

struct FetchedServerTime {
    raw_response: Vec<u8>,
    parsed_l2_timestamp: L2Timestamp,
    receive_clock: PmRestResponseClock,
}

/// Exact `/time` capability for authenticated read requests.
///
/// This move-only role has no book, mutation-time, or raw-client API.
pub struct PmReadServerTimeHttpRole {
    transport: PmHttpTransport,
    clock: PmReadServerTimeProductClock,
    mode: OriginMode,
}

impl PmReadServerTimeHttpRole {
    /// Build a standalone read-only `/time` capability.
    ///
    /// This constructor creates only a read-time clock domain. Callers do not
    /// need to construct the broader public HTTP role or receive a mutation
    /// time/validator capability.
    pub fn new(config: PmPublicHttpConfig) -> Result<Self, PmLiveAdapterError> {
        Self::with_product_clock(config, PmReadServerTimeProductClock::standalone_system())
    }

    pub(crate) fn with_product_clock(
        config: PmPublicHttpConfig,
        clock: PmReadServerTimeProductClock,
    ) -> Result<Self, PmLiveAdapterError> {
        let mode = config.mode();
        Ok(Self {
            transport: PmHttpTransport::new(&config)?,
            clock,
            mode,
        })
    }

    pub async fn fresh_read_server_time(&self) -> Result<PmReadServerTime, PmLiveAdapterError> {
        Ok(self
            .fresh_read_server_time_observation()
            .await?
            .into_read_server_time())
    }

    pub async fn fresh_read_server_time_observation(
        &self,
    ) -> Result<PmReadServerTimeObservation, PmLiveAdapterError> {
        let fetched = fetch_server_time(&self.transport, || self.clock.observe_rest_edge()).await?;
        let commitment = PmReadServerTimeObservationCommitment::from_source_bytes(
            server_time_observation_commitment(
                READ_SERVER_TIME_OBSERVATION_COMMITMENT_DOMAIN,
                self.mode,
                &fetched,
            ),
        );
        let proof = self
            .clock
            .read_time(fetched.parsed_l2_timestamp, fetched.receive_clock);
        Ok(PmReadServerTimeObservation::from_source(
            fetched.parsed_l2_timestamp,
            fetched.receive_clock,
            commitment,
            proof,
        ))
    }
}

/// Exact `/time` capability for one mutation path.
///
/// The public connectivity owner creates independent move-only instances for
/// place and cancel. This role has no read-time, book, or raw-client API.
pub struct PmMutationServerTimeHttpRole {
    transport: PmHttpTransport,
    clock: PmMutationServerTimeProductClock,
    mode: OriginMode,
}

impl PmMutationServerTimeHttpRole {
    pub(crate) fn with_product_clock(
        config: PmPublicHttpConfig,
        clock: PmMutationServerTimeProductClock,
    ) -> Result<Self, PmLiveAdapterError> {
        let mode = config.mode();
        Ok(Self {
            transport: PmHttpTransport::new(&config)?,
            clock,
            mode,
        })
    }

    pub async fn fresh_mutation_server_time(
        &self,
    ) -> Result<PmPendingMutationServerTime, PmLiveAdapterError> {
        Ok(self
            .fresh_mutation_server_time_observation()
            .await?
            .into_pending_mutation_server_time())
    }

    pub async fn fresh_mutation_server_time_observation(
        &self,
    ) -> Result<PmMutationServerTimeObservation, PmLiveAdapterError> {
        let fetched = fetch_server_time(&self.transport, || self.clock.observe_rest_edge()).await?;
        let commitment = PmMutationServerTimeObservationCommitment::from_source_bytes(
            server_time_observation_commitment(
                MUTATION_SERVER_TIME_OBSERVATION_COMMITMENT_DOMAIN,
                self.mode,
                &fetched,
            ),
        );
        let proof = self
            .clock
            .pending_mutation_time(fetched.parsed_l2_timestamp, fetched.receive_clock);
        Ok(PmMutationServerTimeObservation::from_source(
            fetched.parsed_l2_timestamp,
            fetched.receive_clock,
            commitment,
            proof,
        ))
    }
}

async fn fetch_server_time<F>(
    transport: &PmHttpTransport,
    observe_rest_edge: F,
) -> Result<FetchedServerTime, PmLiveAdapterError>
where
    F: FnOnce() -> Result<PmRestResponseClock, PmProductClockError>,
{
    let body = transport
        .get(PmPublicRoute::ServerTime, MAX_SERVER_TIME_BODY_BYTES)
        .await?;
    // Sample after the final bounded response byte and before parsing or
    // downstream queue service can add delay.
    let received = observe_rest_edge().map_err(|_| PmLiveAdapterError::ProductClock)?;
    let seconds = parse_server_time(&body).map_err(PmLiveAdapterError::Wire)?;
    let timestamp = L2Timestamp::from_unix_seconds(seconds)?;
    Ok(FetchedServerTime {
        raw_response: body,
        parsed_l2_timestamp: timestamp,
        receive_clock: received,
    })
}

fn server_time_observation_commitment(
    domain: &'static [u8],
    mode: OriginMode,
    fetched: &FetchedServerTime,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    encode_server_time_bytes(&mut digest, domain);
    encode_server_time_bytes(&mut digest, origin_mode_name(mode));
    encode_server_time_bytes(&mut digest, PM_CLOB_PRODUCTION_ORIGIN.as_bytes());
    encode_server_time_bytes(&mut digest, b"GET");
    encode_server_time_bytes(&mut digest, b"/time");
    encode_server_time_bytes(&mut digest, &fetched.raw_response);
    digest.update(fetched.parsed_l2_timestamp.unix_seconds().to_be_bytes());
    digest.update(fetched.receive_clock.local_wall_receive_ns().to_be_bytes());
    digest.update(fetched.receive_clock.monotonic_receive_ns().to_be_bytes());
    digest.finalize().into()
}

fn encode_server_time_bytes(digest: &mut Sha256, value: &[u8]) {
    digest.update(
        u64::try_from(value.len())
            .expect("bounded server-time commitment field length fits u64")
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmRestBookPurpose {
    Seed,
    Resync,
}

#[async_trait::async_trait]
pub trait PmRestBookSnapshotSink: Send {
    type Output;
    type Error;

    async fn deliver_native_rest_book(
        &mut self,
        purpose: PmRestBookPurpose,
        received: PmRestResponseClock,
        raw: &[u8],
    ) -> Result<Self::Output, Self::Error>;
}

pub struct PmPublicHttpRole {
    transport: PmHttpTransport,
    parser_config: PmBookParserConfig,
    clock: PmPublicHttpProductClock,
    mode: OriginMode,
}

impl PmPublicHttpRole {
    pub fn new(
        config: PmPublicHttpConfig,
        parser_config: PmBookParserConfig,
    ) -> Result<Self, PmLiveAdapterError> {
        if parser_config.market_binding() != PmBookMarketBinding::ConditionId {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "live public books require condition-bound market identity",
            ));
        }
        Self::with_product_clock(
            config,
            parser_config,
            PmPublicHttpProductClock::standalone_system(),
        )
    }

    pub(crate) fn with_product_clock(
        config: PmPublicHttpConfig,
        parser_config: PmBookParserConfig,
        clock: PmPublicHttpProductClock,
    ) -> Result<Self, PmLiveAdapterError> {
        if parser_config.market_binding() != PmBookMarketBinding::ConditionId {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "live public books require condition-bound market identity",
            ));
        }
        let mode = config.mode();
        let transport = PmHttpTransport::new(&config)?;
        Ok(Self {
            transport,
            parser_config,
            clock,
            mode,
        })
    }

    #[must_use]
    pub const fn parser_config(&self) -> PmBookParserConfig {
        self.parser_config
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }

    /// Fetch one exact CLOB server time for one authenticated read request.
    /// The proof remains move-only and freshness checked when consumed.
    pub async fn fresh_read_server_time(&self) -> Result<PmReadServerTime, PmLiveAdapterError> {
        Ok(self
            .fresh_read_server_time_observation()
            .await?
            .into_read_server_time())
    }

    pub async fn fresh_read_server_time_observation(
        &self,
    ) -> Result<PmReadServerTimeObservation, PmLiveAdapterError> {
        let fetched = self.fresh_server_time_observation_basis().await?;
        let commitment = PmReadServerTimeObservationCommitment::from_source_bytes(
            server_time_observation_commitment(
                READ_SERVER_TIME_OBSERVATION_COMMITMENT_DOMAIN,
                self.mode,
                &fetched,
            ),
        );
        let proof = self
            .clock
            .read_time(fetched.parsed_l2_timestamp, fetched.receive_clock);
        Ok(PmReadServerTimeObservation::from_source(
            fetched.parsed_l2_timestamp,
            fetched.receive_clock,
            commitment,
            proof,
        ))
    }

    /// Fetch one pending CLOB server time for mutation admission.
    ///
    /// The authenticated root must pass this value through its same-domain
    /// [`crate::PmMutationServerTimeValidator`] immediately before removing a
    /// Goal-F dispatch. Only the resulting authorized proof may enter the
    /// credential authority.
    pub async fn fresh_mutation_server_time(
        &self,
    ) -> Result<PmPendingMutationServerTime, PmLiveAdapterError> {
        Ok(self
            .fresh_mutation_server_time_observation()
            .await?
            .into_pending_mutation_server_time())
    }

    pub async fn fresh_mutation_server_time_observation(
        &self,
    ) -> Result<PmMutationServerTimeObservation, PmLiveAdapterError> {
        let fetched = self.fresh_server_time_observation_basis().await?;
        let commitment = PmMutationServerTimeObservationCommitment::from_source_bytes(
            server_time_observation_commitment(
                MUTATION_SERVER_TIME_OBSERVATION_COMMITMENT_DOMAIN,
                self.mode,
                &fetched,
            ),
        );
        let proof = self
            .clock
            .pending_mutation_time(fetched.parsed_l2_timestamp, fetched.receive_clock);
        Ok(PmMutationServerTimeObservation::from_source(
            fetched.parsed_l2_timestamp,
            fetched.receive_clock,
            commitment,
            proof,
        ))
    }

    async fn fresh_server_time_observation_basis(
        &self,
    ) -> Result<FetchedServerTime, PmLiveAdapterError> {
        fetch_server_time(&self.transport, || self.clock.observe_rest_edge()).await
    }

    pub async fn seed_book<S>(
        &self,
        sink: &mut S,
    ) -> Result<S::Output, PmRestBookDeliveryError<S::Error>>
    where
        S: PmRestBookSnapshotSink,
    {
        self.deliver_book(PmRestBookPurpose::Seed, sink).await
    }

    pub async fn resync_book<S>(
        &self,
        sink: &mut S,
    ) -> Result<S::Output, PmRestBookDeliveryError<S::Error>>
    where
        S: PmRestBookSnapshotSink,
    {
        self.deliver_book(PmRestBookPurpose::Resync, sink).await
    }

    async fn deliver_book<S>(
        &self,
        purpose: PmRestBookPurpose,
        sink: &mut S,
    ) -> Result<S::Output, PmRestBookDeliveryError<S::Error>>
    where
        S: PmRestBookSnapshotSink,
    {
        let raw = self
            .transport
            .get(
                PmPublicRoute::Book(self.parser_config.scope().token()),
                MAX_PUBLIC_REST_BODY_BYTES,
            )
            .await
            .map_err(PmRestBookDeliveryError::Http)?;
        // Timestamp the real response edge, never request start or sink
        // service time. This keeps a delayed REST snapshot ordered after any
        // intervening streaming frame from the shared clock domain.
        let received = self
            .clock
            .observe_rest_edge()
            .map_err(|_| PmRestBookDeliveryError::Http(PmLiveAdapterError::ProductClock))?;
        sink.deliver_native_rest_book(purpose, received, &raw)
            .await
            .map_err(PmRestBookDeliveryError::Sink)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use reap_pm_core::{PmConditionId, PmMarketId, PmQuantity, PmTick, PmTokenId, U256};
    use reap_polymarket_wire::{
        PmWireError, PmWireScope, compute_snapshot_hash, parse_rest_book_snapshot,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::{mpsc, oneshot},
        task::JoinHandle,
        time::sleep,
    };

    use super::*;
    use crate::PmPublicWsClockSource;

    const CONDITION: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const MARKET: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    struct MockResponse {
        status: u16,
        body: Vec<u8>,
        delay: Duration,
        location: Option<&'static str>,
        include_content_length: bool,
    }

    impl MockResponse {
        fn ok(body: impl Into<Vec<u8>>) -> Self {
            Self {
                status: 200,
                body: body.into(),
                delay: Duration::ZERO,
                location: None,
                include_content_length: true,
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
                let _ = requests_tx.send(request.lines().next().unwrap().to_string());
                sleep(response.delay).await;
                let reason = match response.status {
                    200 => "OK",
                    302 => "Found",
                    404 => "Not Found",
                    503 => "Service Unavailable",
                    _ => "Mock",
                };
                let mut headers = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nConnection: close\r\n",
                    response.status, reason
                );
                if response.include_content_length {
                    headers.push_str(&format!("Content-Length: {}\r\n", response.body.len()));
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

    fn parser_config() -> PmBookParserConfig {
        PmBookParserConfig::new_condition_bound(
            PmWireScope::new(
                PmConditionId::parse(CONDITION).unwrap(),
                PmMarketId::parse(MARKET).unwrap(),
                PmTokenId::new(U256::from_u64(123)).unwrap(),
            ),
            PmTick::parse_decimal("0.01").unwrap(),
            PmQuantity::parse_decimal("5").unwrap(),
            false,
        )
    }

    fn rest_book() -> String {
        let placeholder = format!(
            r#"{{"market":"{CONDITION}","asset_id":"123","timestamp":"123456789","hash":"","bids":[{{"price":"0.30","size":"100"}},{{"price":"0.40","size":"50"}}],"asks":[{{"price":"0.60","size":"75"}},{{"price":"0.70","size":"100"}}],"min_order_size":"5","tick_size":"0.01","neg_risk":false,"last_trade_price":"0.50"}}"#
        );
        let hash = compute_snapshot_hash(placeholder.as_bytes()).unwrap();
        placeholder.replace(r#""hash":"""#, &format!(r#""hash":"{hash}""#))
    }

    fn local_role(origin: &str, request_timeout: Duration) -> PmPublicHttpRole {
        let config =
            PmPublicHttpConfig::local_evidence(origin, Duration::from_millis(100), request_timeout)
                .unwrap();
        PmPublicHttpRole::new(config, parser_config()).unwrap()
    }

    #[test]
    fn live_role_rejects_legacy_question_id_book_binding() {
        let config = PmPublicHttpConfig::local_evidence(
            "http://127.0.0.1:18080",
            Duration::from_millis(100),
            Duration::from_millis(200),
        )
        .unwrap();
        let condition_bound = parser_config();
        let legacy = PmBookParserConfig::new(
            condition_bound.scope(),
            condition_bound.tick(),
            condition_bound.minimum_order_size(),
            condition_bound.negative_risk(),
        );
        assert!(matches!(
            PmPublicHttpRole::new(config, legacy),
            Err(PmLiveAdapterError::InvalidConfiguration(_))
        ));
    }

    #[derive(Default)]
    struct ValidatingSink {
        purposes: Vec<PmRestBookPurpose>,
        bodies: Vec<Vec<u8>>,
    }

    struct DurabilityBarrierSink {
        started: Option<oneshot::Sender<PmRestBookPurpose>>,
        release: Option<oneshot::Receiver<()>>,
    }

    struct EdgeSink;

    #[async_trait::async_trait]
    impl PmRestBookSnapshotSink for EdgeSink {
        type Output = PmRestResponseClock;
        type Error = PmWireError;

        async fn deliver_native_rest_book(
            &mut self,
            _purpose: PmRestBookPurpose,
            received: PmRestResponseClock,
            raw: &[u8],
        ) -> Result<Self::Output, Self::Error> {
            parse_rest_book_snapshot(raw, parser_config())?;
            Ok(received)
        }
    }

    #[async_trait::async_trait]
    impl PmRestBookSnapshotSink for DurabilityBarrierSink {
        type Output = PmRestBookPurpose;
        type Error = PmWireError;

        async fn deliver_native_rest_book(
            &mut self,
            purpose: PmRestBookPurpose,
            _received: PmRestResponseClock,
            raw: &[u8],
        ) -> Result<Self::Output, Self::Error> {
            parse_rest_book_snapshot(raw, parser_config())?;
            self.started
                .take()
                .expect("one delivery")
                .send(purpose)
                .expect("test observes sink entry");
            self.release
                .take()
                .expect("one durability barrier")
                .await
                .expect("test releases durability barrier");
            Ok(purpose)
        }
    }

    #[async_trait::async_trait]
    impl PmRestBookSnapshotSink for ValidatingSink {
        type Output = usize;
        type Error = PmWireError;

        async fn deliver_native_rest_book(
            &mut self,
            purpose: PmRestBookPurpose,
            _received: PmRestResponseClock,
            raw: &[u8],
        ) -> Result<Self::Output, Self::Error> {
            parse_rest_book_snapshot(raw, parser_config())?;
            self.purposes.push(purpose);
            self.bodies.push(raw.to_vec());
            Ok(self.bodies.len())
        }
    }

    #[tokio::test]
    async fn exact_time_and_token_book_routes_deliver_valid_native_bytes() {
        let book = rest_book();
        let (origin, mut requests, server) = mock_server(vec![
            MockResponse::ok(b"1234567890".to_vec()),
            MockResponse::ok(book.as_bytes().to_vec()),
            MockResponse::ok(book.as_bytes().to_vec()),
        ])
        .await;
        let role = local_role(&origin, Duration::from_secs(1));
        assert_eq!(
            role.fresh_read_server_time()
                .await
                .unwrap()
                .into_l2_timestamp()
                .unwrap()
                .unix_seconds(),
            1_234_567_890
        );
        let mut sink = ValidatingSink::default();
        assert_eq!(role.seed_book(&mut sink).await.unwrap(), 1);
        assert_eq!(role.resync_book(&mut sink).await.unwrap(), 2);
        assert_eq!(
            sink.purposes,
            [PmRestBookPurpose::Seed, PmRestBookPurpose::Resync]
        );
        assert_eq!(requests.recv().await.unwrap(), "GET /time HTTP/1.1");
        assert_eq!(
            requests.recv().await.unwrap(),
            "GET /book?token_id=123 HTTP/1.1"
        );
        assert_eq!(
            requests.recv().await.unwrap(),
            "GET /book?token_id=123 HTTP/1.1"
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn restricted_read_place_and_cancel_roles_each_use_exact_time_route() {
        let (origin, mut requests, server) = mock_server(vec![
            MockResponse::ok(b"1234567890".to_vec()),
            MockResponse::ok(b"1234567891".to_vec()),
            MockResponse::ok(b"1234567892".to_vec()),
        ])
        .await;
        let config = PmPublicHttpConfig::local_evidence(
            &origin,
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap();
        let clock = crate::PmProductClockOwner::test_support_scripted(&[
            (1_000, 10),
            (1_001, 11),
            (1_002, 12),
            (1_003, 13),
            (1_004, 14),
            (1_005, 15),
        ])
        .unwrap();
        let (_, _, _, read_clock, _, place_clock, cancel_clock, _, _, mut validator) =
            clock.split().into_views();
        let read =
            PmReadServerTimeHttpRole::with_product_clock(config.clone(), read_clock).unwrap();
        let place =
            PmMutationServerTimeHttpRole::with_product_clock(config.clone(), place_clock).unwrap();
        let cancel =
            PmMutationServerTimeHttpRole::with_product_clock(config, cancel_clock).unwrap();

        assert_eq!(
            read.fresh_read_server_time()
                .await
                .unwrap()
                .into_l2_timestamp()
                .unwrap()
                .unix_seconds(),
            1_234_567_890
        );
        assert_eq!(
            validator
                .authorize(place.fresh_mutation_server_time().await.unwrap())
                .unwrap()
                .into_l2_timestamp()
                .unix_seconds(),
            1_234_567_891
        );
        assert_eq!(
            validator
                .authorize(cancel.fresh_mutation_server_time().await.unwrap())
                .unwrap()
                .into_l2_timestamp()
                .unix_seconds(),
            1_234_567_892
        );
        for _ in 0..3 {
            assert_eq!(requests.recv().await.unwrap(), "GET /time HTTP/1.1");
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn loopback_server_time_observations_bind_every_source_input_and_preserve_proofs() {
        let (origin, mut requests, server) = mock_server(vec![
            MockResponse::ok(b"1234567890".to_vec()),
            MockResponse::ok(b"1234567891".to_vec()),
        ])
        .await;
        let config = PmPublicHttpConfig::local_evidence(
            &origin,
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap();
        let clock = crate::PmProductClockOwner::test_support_scripted(&[
            (1_000, 10),
            (1_001, 11),
            (1_002, 12),
            (1_003, 13),
        ])
        .unwrap();
        let (_, _, _, read_clock, _, mutation_clock, _, _, _, mut validator) =
            clock.split().into_views();
        let read =
            PmReadServerTimeHttpRole::with_product_clock(config.clone(), read_clock).unwrap();
        let mutation =
            PmMutationServerTimeHttpRole::with_product_clock(config, mutation_clock).unwrap();

        let read_observation = read.fresh_read_server_time_observation().await.unwrap();
        assert_eq!(
            read_observation.parsed_l2_timestamp().unix_seconds(),
            1_234_567_890
        );
        assert_eq!(read_observation.receive_clock().monotonic_receive_ns(), 10);
        let read_basis = FetchedServerTime {
            raw_response: b"1234567890".to_vec(),
            parsed_l2_timestamp: read_observation.parsed_l2_timestamp(),
            receive_clock: read_observation.receive_clock(),
        };
        let base = server_time_observation_commitment(
            READ_SERVER_TIME_OBSERVATION_COMMITMENT_DOMAIN,
            OriginMode::LocalEvidence,
            &read_basis,
        );
        assert_eq!(base, read_observation.commitment().bytes());
        assert_ne!(
            base,
            server_time_observation_commitment(
                READ_SERVER_TIME_OBSERVATION_COMMITMENT_DOMAIN,
                OriginMode::Production,
                &read_basis,
            )
        );
        let raw_mutation = FetchedServerTime {
            raw_response: b"{\"timestamp\":1234567890}".to_vec(),
            parsed_l2_timestamp: read_basis.parsed_l2_timestamp,
            receive_clock: read_basis.receive_clock,
        };
        assert_ne!(
            base,
            server_time_observation_commitment(
                READ_SERVER_TIME_OBSERVATION_COMMITMENT_DOMAIN,
                OriginMode::LocalEvidence,
                &raw_mutation,
            )
        );
        let parsed_mutation = FetchedServerTime {
            raw_response: read_basis.raw_response.clone(),
            parsed_l2_timestamp: L2Timestamp::from_unix_seconds(1_234_567_899).unwrap(),
            receive_clock: read_basis.receive_clock,
        };
        assert_ne!(
            base,
            server_time_observation_commitment(
                READ_SERVER_TIME_OBSERVATION_COMMITMENT_DOMAIN,
                OriginMode::LocalEvidence,
                &parsed_mutation,
            )
        );
        assert_ne!(
            base,
            server_time_observation_commitment(
                MUTATION_SERVER_TIME_OBSERVATION_COMMITMENT_DOMAIN,
                OriginMode::LocalEvidence,
                &read_basis,
            )
        );
        assert_eq!(
            read_observation
                .into_read_server_time()
                .into_l2_timestamp()
                .unwrap()
                .unix_seconds(),
            1_234_567_890
        );

        let mutation_observation = mutation
            .fresh_mutation_server_time_observation()
            .await
            .unwrap();
        assert_eq!(
            mutation_observation.parsed_l2_timestamp().unix_seconds(),
            1_234_567_891
        );
        assert_eq!(
            mutation_observation.receive_clock().monotonic_receive_ns(),
            12
        );
        let clock_mutation = FetchedServerTime {
            raw_response: read_basis.raw_response,
            parsed_l2_timestamp: read_basis.parsed_l2_timestamp,
            receive_clock: mutation_observation.receive_clock(),
        };
        assert_ne!(
            base,
            server_time_observation_commitment(
                READ_SERVER_TIME_OBSERVATION_COMMITMENT_DOMAIN,
                OriginMode::LocalEvidence,
                &clock_mutation,
            )
        );
        assert_eq!(
            validator
                .authorize(mutation_observation.into_pending_mutation_server_time())
                .unwrap()
                .into_l2_timestamp()
                .unix_seconds(),
            1_234_567_891
        );
        for _ in 0..2 {
            assert_eq!(requests.recv().await.unwrap(), "GET /time HTTP/1.1");
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn rest_book_role_awaits_the_async_durability_sink() {
        let book = rest_book();
        let (origin, _requests, server) =
            mock_server(vec![MockResponse::ok(book.as_bytes().to_vec())]).await;
        let role = local_role(&origin, Duration::from_secs(1));
        let (started, entered) = oneshot::channel();
        let (release, barrier) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut sink = DurabilityBarrierSink {
                started: Some(started),
                release: Some(barrier),
            };
            role.seed_book(&mut sink).await
        });

        assert_eq!(entered.await.unwrap(), PmRestBookPurpose::Seed);
        assert!(
            !task.is_finished(),
            "transport must not release parsed book evidence before sink durability"
        );
        release.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap(), PmRestBookPurpose::Seed);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_time_and_book_fail_at_the_typed_boundary() {
        let (origin, _, server) = mock_server(vec![
            MockResponse::ok(b"1.5".to_vec()),
            MockResponse::ok(b"{".to_vec()),
        ])
        .await;
        let role = local_role(&origin, Duration::from_secs(1));
        assert!(matches!(
            role.fresh_read_server_time().await,
            Err(PmLiveAdapterError::Wire(PmWireError::MalformedJson))
        ));
        let mut sink = ValidatingSink::default();
        assert_eq!(
            role.seed_book(&mut sink).await,
            Err(PmRestBookDeliveryError::Sink(PmWireError::MalformedJson))
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn non_success_and_redirect_are_terminal_and_redirect_is_not_followed() {
        let (origin, _, server) = mock_server(vec![
            MockResponse {
                status: 503,
                body: b"{}".to_vec(),
                delay: Duration::ZERO,
                location: None,
                include_content_length: true,
            },
            MockResponse {
                status: 302,
                body: Vec::new(),
                delay: Duration::ZERO,
                location: Some("http://127.0.0.1:9/escape"),
                include_content_length: true,
            },
        ])
        .await;
        let role = local_role(&origin, Duration::from_secs(1));
        assert!(matches!(
            role.fresh_read_server_time().await,
            Err(PmLiveAdapterError::UnexpectedStatus { status: 503 })
        ));
        assert!(matches!(
            role.fresh_read_server_time().await,
            Err(PmLiveAdapterError::Redirect { status: 302 })
        ));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn advertised_and_streamed_oversized_bodies_are_rejected_before_sink_delivery() {
        let oversized = vec![b'x'; MAX_PUBLIC_REST_BODY_BYTES + 1];
        let (origin, _, server) = mock_server(vec![
            MockResponse {
                status: 200,
                body: oversized.clone(),
                delay: Duration::ZERO,
                location: None,
                include_content_length: true,
            },
            MockResponse {
                status: 200,
                body: oversized,
                delay: Duration::ZERO,
                location: None,
                include_content_length: false,
            },
        ])
        .await;
        let role = local_role(&origin, Duration::from_secs(2));
        let mut sink = ValidatingSink::default();
        for purpose in [PmRestBookPurpose::Seed, PmRestBookPurpose::Resync] {
            let result = match purpose {
                PmRestBookPurpose::Seed => role.seed_book(&mut sink).await,
                PmRestBookPurpose::Resync => role.resync_book(&mut sink).await,
            };
            assert_eq!(
                result,
                Err(PmRestBookDeliveryError::Http(
                    PmLiveAdapterError::ResponseBodyTooLarge {
                        limit: MAX_PUBLIC_REST_BODY_BYTES,
                    }
                ))
            );
        }
        assert!(sink.bodies.is_empty());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn request_timeout_is_typed_and_bounded() {
        let (origin, _, server) = mock_server(vec![MockResponse {
            status: 200,
            body: b"1234567890".to_vec(),
            delay: Duration::from_millis(100),
            location: None,
            include_content_length: true,
        }])
        .await;
        let role = local_role(&origin, Duration::from_millis(20));
        assert!(matches!(
            role.fresh_read_server_time().await,
            Err(PmLiveAdapterError::RequestTimeout)
        ));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn delayed_rest_body_is_stamped_after_an_intervening_websocket_edge() {
        let (origin, mut requests, server) = mock_server(vec![MockResponse {
            status: 200,
            body: rest_book().into_bytes(),
            delay: Duration::from_millis(100),
            location: None,
            include_content_length: true,
        }])
        .await;
        let clock = crate::PmProductClockOwner::test_support_scripted(&[
            (1_700_000_000_000_000_100, 100),
            (1_700_000_000_000_000_200, 200),
        ])
        .unwrap();
        let (mut public_ws_clock, _, http_clock, _, _, _, _, _, _, _) = clock.split().into_views();
        let config = PmPublicHttpConfig::local_evidence(
            &origin,
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap();
        let role =
            PmPublicHttpRole::with_product_clock(config, parser_config(), http_clock).unwrap();
        let task = tokio::spawn(async move {
            let mut sink = EdgeSink;
            role.seed_book(&mut sink).await.unwrap()
        });

        assert_eq!(
            requests.recv().await.unwrap(),
            "GET /book?token_id=123 HTTP/1.1"
        );
        let intervening = public_ws_clock.observe_public_ws_edge().unwrap();
        let rest = task.await.unwrap();
        assert!(rest.monotonic_receive_ns() > intervening.monotonic_receive_ns());
        assert_eq!(rest.monotonic_receive_ns(), 200);
        server.await.unwrap();
    }
}
