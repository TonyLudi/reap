use std::fmt;

use reap_polymarket_wire::{
    MAX_PUBLIC_REST_BODY_BYTES, PmClobV2Metadata, PmClobV2RequestScope, PmLifecycleMetadata,
    PmLiveClobMarketLifecycle, PmLongMarketLifecycleDetails, PmWireScope,
    parse_live_clob_market_lifecycle_details, parse_live_clob_v2_metadata,
};
use sha2::{Digest as _, Sha256};

use crate::{
    PM_CLOB_PRODUCTION_ORIGIN, PmHttpReceiveClock, PmLiveAdapterError, PmPublicHttpConfig,
    PmPublicMetadataDeliveryError,
    config::OriginMode,
    http_transport::{PmHttpTransport, PmPublicRoute},
    observation_clock::PmHttpReceiveClockSource,
};

const LIVE_METADATA_OBSERVATION_COMMITMENT_DOMAIN: &[u8] =
    b"reap.pm.live-adapter.public-metadata-observation.v2\0";

/// The two native public metadata bodies bound to one configured scope.
/// Neither constituent body is delivered independently.
pub struct PmLiveMetadataPair<'a> {
    scope: PmWireScope,
    market_bytes: &'a [u8],
    clob_v2_bytes: &'a [u8],
}

impl PmLiveMetadataPair<'_> {
    #[must_use]
    pub const fn scope(&self) -> PmWireScope {
        self.scope
    }

    #[must_use]
    pub const fn market_bytes(&self) -> &[u8] {
        self.market_bytes
    }

    #[must_use]
    pub const fn clob_v2_bytes(&self) -> &[u8] {
        self.clob_v2_bytes
    }
}

impl fmt::Debug for PmLiveMetadataPair<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmLiveMetadataPair")
            .field("scope", &self.scope)
            .field("market_bytes", &self.market_bytes.len())
            .field("clob_v2_bytes", &self.clob_v2_bytes.len())
            .finish()
    }
}

pub trait PmLiveMetadataPairSink {
    type Output;
    type Error;

    fn deliver_native_metadata_pair(
        &mut self,
        pair: PmLiveMetadataPair<'_>,
    ) -> Result<Self::Output, Self::Error>;
}

/// Fully parsed result of the exact long-market plus abbreviated CLOB-details
/// pair. Construction succeeds only after both bounded responses validate
/// against the same configured condition/question/token scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmTypedLiveMarketDetails {
    long_market: PmLiveClobMarketLifecycle,
    clob: PmClobV2Metadata,
}

impl PmTypedLiveMarketDetails {
    #[must_use]
    pub const fn lifecycle(&self) -> &PmLifecycleMetadata {
        self.long_market.metadata()
    }

    #[must_use]
    pub const fn lifecycle_details(&self) -> &PmLongMarketLifecycleDetails {
        self.long_market.details()
    }

    #[must_use]
    pub const fn clob(&self) -> &PmClobV2Metadata {
        &self.clob
    }
}

/// Domain-separated SHA-256 commitment to one complete live metadata
/// observation. It binds the fixed production/evidence mode, exact scope,
/// ordered native response bodies, parsed lifecycle/CLOB facts, and the
/// source-captured receive edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmLiveMetadataObservationCommitment([u8; 32]);

impl PmLiveMetadataObservationCommitment {
    const fn from_source_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Sealed parsed observation returned by the fixed two-route metadata source.
/// Raw bodies participate in the commitment but cannot escape this carrier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmLiveMetadataObservation {
    details: PmTypedLiveMarketDetails,
    receive_clock: PmHttpReceiveClock,
    commitment: PmLiveMetadataObservationCommitment,
}

impl PmLiveMetadataObservation {
    fn from_source(
        details: PmTypedLiveMarketDetails,
        receive_clock: PmHttpReceiveClock,
        commitment: PmLiveMetadataObservationCommitment,
    ) -> Self {
        Self {
            details,
            receive_clock,
            commitment,
        }
    }

    #[must_use]
    pub const fn details(&self) -> &PmTypedLiveMarketDetails {
        &self.details
    }

    #[must_use]
    pub const fn lifecycle(&self) -> &PmLifecycleMetadata {
        self.details.lifecycle()
    }

    #[must_use]
    pub const fn lifecycle_details(&self) -> &PmLongMarketLifecycleDetails {
        self.details.lifecycle_details()
    }

    #[must_use]
    pub const fn clob(&self) -> &PmClobV2Metadata {
        self.details.clob()
    }

    #[must_use]
    pub const fn receive_clock(&self) -> PmHttpReceiveClock {
        self.receive_clock
    }

    #[must_use]
    pub const fn commitment(&self) -> PmLiveMetadataObservationCommitment {
        self.commitment
    }

    #[must_use]
    pub fn into_details(self) -> PmTypedLiveMarketDetails {
        self.details
    }
}

/// Exact two-route public metadata capability for one configured PM outcome.
#[derive(Clone)]
pub struct PmPublicMetadataHttpRole {
    transport: PmHttpTransport,
    scope: PmWireScope,
    mode: OriginMode,
    clock: PmHttpReceiveClockSource,
}

impl PmPublicMetadataHttpRole {
    pub fn new(config: PmPublicHttpConfig, scope: PmWireScope) -> Result<Self, PmLiveAdapterError> {
        let mode = config.mode();
        let transport = PmHttpTransport::new(&config)?;
        Ok(Self {
            transport,
            scope,
            mode,
            clock: PmHttpReceiveClockSource::system(),
        })
    }

    #[must_use]
    pub const fn configured_scope(&self) -> PmWireScope {
        self.scope
    }

    /// Fetch both fixed metadata routes and deliver them once as one pair.
    /// A failure on either read drops all gathered bytes without invoking the
    /// sink.
    pub async fn refresh<S>(
        &self,
        sink: &mut S,
    ) -> Result<S::Output, PmPublicMetadataDeliveryError<S::Error>>
    where
        S: PmLiveMetadataPairSink,
    {
        let (market_bytes, clob_v2_bytes) = self
            .fetch_pair()
            .await
            .map_err(PmPublicMetadataDeliveryError::Http)?;
        sink.deliver_native_metadata_pair(PmLiveMetadataPair {
            scope: self.scope,
            market_bytes: &market_bytes,
            clob_v2_bytes: &clob_v2_bytes,
        })
        .map_err(PmPublicMetadataDeliveryError::Sink)
    }

    /// Fetch and strictly parse the complete configured market preflight pair.
    pub async fn refresh_typed(&self) -> Result<PmTypedLiveMarketDetails, PmLiveAdapterError> {
        Ok(self.refresh_typed_observation().await?.into_details())
    }

    /// Fetch, source-time, strictly parse, and seal the complete configured
    /// market preflight pair. The receive edge is sampled after the second
    /// bounded body completes and before either body is parsed.
    pub async fn refresh_typed_observation(
        &self,
    ) -> Result<PmLiveMetadataObservation, PmLiveAdapterError> {
        let (market_bytes, clob_v2_bytes) = self.fetch_pair().await?;
        let receive_clock = self.clock.observe()?;
        let long_market = parse_live_clob_market_lifecycle_details(&market_bytes, self.scope)?;
        let clob = parse_live_clob_v2_metadata(
            &clob_v2_bytes,
            PmClobV2RequestScope::new(self.scope.condition(), self.scope.token()),
        )?;
        let details = PmTypedLiveMarketDetails { long_market, clob };
        let commitment = live_metadata_observation_commitment(
            self.mode,
            self.scope,
            &market_bytes,
            &clob_v2_bytes,
            &details,
            receive_clock,
        );
        Ok(PmLiveMetadataObservation::from_source(
            details,
            receive_clock,
            commitment,
        ))
    }

    async fn fetch_pair(&self) -> Result<(Vec<u8>, Vec<u8>), PmLiveAdapterError> {
        let market_bytes = self
            .transport
            .get(
                PmPublicRoute::MarketMetadata(self.scope.condition()),
                MAX_PUBLIC_REST_BODY_BYTES,
            )
            .await?;
        let clob_v2_bytes = self
            .transport
            .get(
                PmPublicRoute::ClobV2Metadata(self.scope.condition()),
                MAX_PUBLIC_REST_BODY_BYTES,
            )
            .await?;
        Ok((market_bytes, clob_v2_bytes))
    }
}

fn live_metadata_observation_commitment(
    mode: OriginMode,
    scope: PmWireScope,
    market_bytes: &[u8],
    clob_v2_bytes: &[u8],
    details: &PmTypedLiveMarketDetails,
    receive_clock: PmHttpReceiveClock,
) -> PmLiveMetadataObservationCommitment {
    let mut digest = Sha256::new();
    encode_metadata_bytes(&mut digest, LIVE_METADATA_OBSERVATION_COMMITMENT_DOMAIN);
    encode_metadata_bytes(&mut digest, origin_mode_name(mode));
    encode_metadata_bytes(&mut digest, PM_CLOB_PRODUCTION_ORIGIN.as_bytes());
    encode_metadata_bytes(&mut digest, b"GET");
    encode_metadata_bytes(&mut digest, b"/markets/{condition_id}");
    encode_metadata_bytes(&mut digest, b"GET");
    encode_metadata_bytes(&mut digest, b"/clob-markets/{condition_id}");

    digest.update(scope.condition().bytes());
    digest.update(scope.market().bytes());
    digest.update(scope.token().units().to_be_bytes());
    digest.update(receive_clock.local_wall_receive_ns().to_be_bytes());
    digest.update(receive_clock.monotonic_receive_ns().to_be_bytes());

    // Response order is protocol-significant: long market first, abbreviated
    // CLOB details second.
    encode_metadata_bytes(&mut digest, market_bytes);
    encode_metadata_bytes(&mut digest, clob_v2_bytes);

    let lifecycle = details.lifecycle();
    digest.update(lifecycle.condition().bytes());
    digest.update(lifecycle.market().bytes());
    let state = lifecycle.lifecycle();
    digest.update([
        u8::from(state.active()),
        u8::from(state.closed()),
        u8::from(state.archived()),
        u8::from(state.accepting_orders()),
        u8::from(state.order_book_enabled()),
    ]);
    let lifecycle_details = details.lifecycle_details();
    encode_optional_metadata_ascii(
        &mut digest,
        lifecycle_details
            .accepting_order_timestamp()
            .map(|value| value.as_str()),
    );
    encode_metadata_bytes(
        &mut digest,
        lifecycle_details.end_date_iso().as_str().as_bytes(),
    );
    encode_optional_metadata_ascii(
        &mut digest,
        lifecycle_details
            .game_start_time()
            .map(|value| value.as_str()),
    );
    digest.update(lifecycle_details.seconds_delay().to_be_bytes());

    let clob = details.clob();
    digest.update(clob.requested_condition().bytes());
    match clob.reported_condition() {
        Some(condition) => {
            digest.update([1]);
            digest.update(condition.bytes());
        }
        None => digest.update([0]),
    }
    digest.update(
        u32::try_from(clob.tokens().len())
            .expect("bounded CLOB token count fits u32")
            .to_be_bytes(),
    );
    for token in clob.tokens() {
        digest.update(token.token().units().to_be_bytes());
        encode_metadata_bytes(&mut digest, token.label().as_str().as_bytes());
    }
    let configured_outcome = clob.configured_outcome();
    digest.update(configured_outcome.token().units().to_be_bytes());
    encode_metadata_bytes(&mut digest, configured_outcome.label().as_str().as_bytes());
    digest.update(clob.tick().units().to_be_bytes());
    digest.update(clob.minimum_order_size().protocol_units().to_be_bytes());
    digest.update([u8::from(clob.negative_risk())]);
    digest.update(clob.maker_base_fee_bps().to_be_bytes());
    digest.update(clob.taker_base_fee_bps().to_be_bytes());
    encode_optional_metadata_ascii(
        &mut digest,
        clob.fee_details().rate().map(|value| value.as_str()),
    );
    encode_optional_metadata_ascii(
        &mut digest,
        clob.fee_details().exponent().map(|value| value.as_str()),
    );
    match clob.fee_details().taker_only() {
        Some(value) => digest.update([1, u8::from(value)]),
        None => digest.update([0]),
    }
    digest.update([u8::from(clob.take_only_delay_enabled())]);
    digest.update(clob.minimum_order_age_seconds().to_be_bytes());

    PmLiveMetadataObservationCommitment::from_source_bytes(digest.finalize().into())
}

fn encode_metadata_bytes(digest: &mut Sha256, value: &[u8]) {
    digest.update(
        u64::try_from(value.len())
            .expect("bounded metadata commitment field length fits u64")
            .to_be_bytes(),
    );
    digest.update(value);
}

fn encode_optional_metadata_ascii(digest: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            digest.update([1]);
            encode_metadata_bytes(digest, value.as_bytes());
        }
        None => digest.update([0]),
    }
}

const fn origin_mode_name(mode: OriginMode) -> &'static [u8] {
    match mode {
        OriginMode::Production => b"production",
        #[cfg(any(test, feature = "loopback-evidence", feature = "read-only-evidence"))]
        OriginMode::LocalEvidence => b"local-evidence",
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use reap_pm_core::{PmConditionId, PmMarketId, PmQuantity, PmTokenId, U256};
    use reap_polymarket_wire::{
        PmBookMarketBinding, PmBookParserConfig, PmClobV2RequestScope, PmWireError,
        parse_live_clob_market_lifecycle, parse_live_clob_market_lifecycle_details,
        parse_live_clob_v2_metadata,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::mpsc,
        task::JoinHandle,
    };

    use super::*;

    const CONDITION: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
    const MARKET: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";

    struct MockResponse {
        status: u16,
        body: Vec<u8>,
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
                location: None,
            }
        }

        fn status(status: u16) -> Self {
            Self {
                status,
                body: b"{}".to_vec(),
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
        let (request_tx, request_rx) = mpsc::unbounded_channel();
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
                let _ = request_tx.send(request.lines().next().unwrap().to_owned());
                let reason = match response.status {
                    200 => "OK",
                    302 => "Found",
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
        (format!("http://{address}"), request_rx, task)
    }

    fn scope() -> PmWireScope {
        PmWireScope::new(
            PmConditionId::parse(CONDITION).unwrap(),
            PmMarketId::parse(MARKET).unwrap(),
            PmTokenId::new(U256::from_u64(123)).unwrap(),
        )
    }

    fn long_market() -> String {
        format!(
            r#"{{"condition_id":"{CONDITION}","question_id":"{MARKET}","active":true,"closed":false,"archived":false,"accepting_orders":true,"enable_order_book":true,"accepting_order_timestamp":"2026-08-08T00:00:00Z","end_date_iso":"2027-01-01T00:00:00Z","game_start_time":null,"seconds_delay":0}}"#
        )
    }

    fn short_market() -> String {
        format!(
            r#"{{"c":"{CONDITION}","t":[{{"t":"123","o":"Yes"}},{{"t":"456","o":"No"}}],"mts":0.01,"mos":5,"nr":false,"fd":{{"r":0.02,"e":2,"to":true}},"mbf":0,"tbf":0,"itode":false,"oas":0}}"#
        )
    }

    fn local_role(origin: &str) -> PmPublicMetadataHttpRole {
        let config = PmPublicHttpConfig::local_evidence(
            origin,
            Duration::from_millis(100),
            Duration::from_secs(1),
        )
        .unwrap();
        PmPublicMetadataHttpRole::new(config, scope()).unwrap()
    }

    #[derive(Default)]
    struct ValidatingSink {
        calls: usize,
    }

    impl PmLiveMetadataPairSink for ValidatingSink {
        type Output = PmBookParserConfig;
        type Error = PmWireError;

        fn deliver_native_metadata_pair(
            &mut self,
            pair: PmLiveMetadataPair<'_>,
        ) -> Result<Self::Output, Self::Error> {
            self.calls += 1;
            let lifecycle = parse_live_clob_market_lifecycle(pair.market_bytes(), pair.scope())?;
            let clob = parse_live_clob_v2_metadata(
                pair.clob_v2_bytes(),
                PmClobV2RequestScope::new(pair.scope().condition(), pair.scope().token()),
            )?;
            assert_eq!(lifecycle.condition(), pair.scope().condition());
            assert_eq!(lifecycle.market(), pair.scope().market());
            Ok(PmBookParserConfig::new_condition_bound(
                pair.scope(),
                clob.tick(),
                clob.minimum_order_size(),
                clob.negative_risk(),
            ))
        }
    }

    #[tokio::test]
    async fn exact_routes_deliver_one_validated_condition_bound_pair() {
        let (origin, mut requests, task) = mock_server(vec![
            MockResponse::ok(long_market()),
            MockResponse::ok(short_market()),
        ])
        .await;
        let role = local_role(&origin);
        let mut sink = ValidatingSink::default();
        let parser = role.refresh(&mut sink).await.unwrap();
        assert_eq!(sink.calls, 1);
        assert_eq!(parser.market_binding(), PmBookMarketBinding::ConditionId);
        assert_eq!(parser.scope(), scope());
        assert_eq!(
            parser.minimum_order_size(),
            PmQuantity::parse_decimal("5").unwrap()
        );
        assert_eq!(
            requests.recv().await.unwrap(),
            format!("GET /markets/{CONDITION} HTTP/1.1")
        );
        assert_eq!(
            requests.recv().await.unwrap(),
            format!("GET /clob-markets/{CONDITION} HTTP/1.1")
        );
        task.await.unwrap();
    }

    #[tokio::test]
    async fn typed_observation_seals_the_ordered_pair_at_the_source_receive_edge() {
        let market = long_market();
        let clob = short_market();
        let (origin, mut requests, task) =
            mock_server(vec![MockResponse::ok(market), MockResponse::ok(clob)]).await;
        let role = local_role(&origin);
        let observation = role.refresh_typed_observation().await.unwrap();

        assert_eq!(observation.lifecycle().condition(), scope().condition());
        assert_eq!(observation.lifecycle().market(), scope().market());
        assert_eq!(
            observation
                .lifecycle_details()
                .accepting_order_timestamp()
                .unwrap()
                .as_str(),
            "2026-08-08T00:00:00Z"
        );
        assert_eq!(
            observation.lifecycle_details().end_date_iso().as_str(),
            "2027-01-01T00:00:00Z"
        );
        assert_eq!(observation.lifecycle_details().game_start_time(), None);
        assert_eq!(observation.lifecycle_details().seconds_delay(), 0);
        assert_eq!(
            observation.clob().configured_outcome().token(),
            scope().token()
        );
        assert!(observation.receive_clock().local_wall_receive_ns() > 0);
        assert!(observation.receive_clock().monotonic_receive_ns() > 0);
        assert_ne!(observation.commitment().bytes(), [0; 32]);
        assert_eq!(
            requests.recv().await.unwrap(),
            format!("GET /markets/{CONDITION} HTTP/1.1")
        );
        assert_eq!(
            requests.recv().await.unwrap(),
            format!("GET /clob-markets/{CONDITION} HTTP/1.1")
        );
        task.await.unwrap();
    }

    #[tokio::test]
    async fn loopback_metadata_commitment_changes_for_every_observed_input_class() {
        let market = long_market();
        let clob = short_market();
        let (origin, _requests, task) = mock_server(vec![
            MockResponse::ok(market.clone()),
            MockResponse::ok(clob.clone()),
        ])
        .await;
        let role = local_role(&origin);
        let observation = role.refresh_typed_observation().await.unwrap();
        let received = observation.receive_clock();
        let base = live_metadata_observation_commitment(
            OriginMode::LocalEvidence,
            scope(),
            market.as_bytes(),
            clob.as_bytes(),
            observation.details(),
            received,
        );
        assert_eq!(base, observation.commitment());

        assert_ne!(
            base,
            live_metadata_observation_commitment(
                OriginMode::Production,
                scope(),
                market.as_bytes(),
                clob.as_bytes(),
                observation.details(),
                received,
            )
        );

        let alternate_scope = PmWireScope::new(
            scope().condition(),
            scope().market(),
            PmTokenId::new(U256::from_u64(456)).unwrap(),
        );
        assert_ne!(
            base,
            live_metadata_observation_commitment(
                OriginMode::LocalEvidence,
                alternate_scope,
                market.as_bytes(),
                clob.as_bytes(),
                observation.details(),
                received,
            )
        );

        let raw_mutation = format!("{market} ");
        assert_ne!(
            base,
            live_metadata_observation_commitment(
                OriginMode::LocalEvidence,
                scope(),
                raw_mutation.as_bytes(),
                clob.as_bytes(),
                observation.details(),
                received,
            )
        );
        assert_ne!(
            base,
            live_metadata_observation_commitment(
                OriginMode::LocalEvidence,
                scope(),
                clob.as_bytes(),
                market.as_bytes(),
                observation.details(),
                received,
            )
        );

        let inactive_market = market.replace("\"active\":true", "\"active\":false");
        let inactive_details = PmTypedLiveMarketDetails {
            long_market: parse_live_clob_market_lifecycle_details(
                inactive_market.as_bytes(),
                scope(),
            )
            .unwrap(),
            clob: observation.clob().clone(),
        };
        assert_ne!(
            base,
            live_metadata_observation_commitment(
                OriginMode::LocalEvidence,
                scope(),
                market.as_bytes(),
                clob.as_bytes(),
                &inactive_details,
                received,
            )
        );

        for lifecycle_mutation in [
            market.replace("2026-08-08T00:00:00Z", "2026-08-08T00:00:01Z"),
            market.replace("2027-01-01T00:00:00Z", "2027-01-02T00:00:00Z"),
            market.replace(
                r#""game_start_time":null"#,
                r#""game_start_time":"2026-12-31T00:00:00Z""#,
            ),
            market.replace(r#""seconds_delay":0"#, r#""seconds_delay":1"#),
        ] {
            let lifecycle_details = PmTypedLiveMarketDetails {
                long_market: parse_live_clob_market_lifecycle_details(
                    lifecycle_mutation.as_bytes(),
                    scope(),
                )
                .unwrap(),
                clob: observation.clob().clone(),
            };
            assert_ne!(
                base,
                live_metadata_observation_commitment(
                    OriginMode::LocalEvidence,
                    scope(),
                    market.as_bytes(),
                    clob.as_bytes(),
                    &lifecycle_details,
                    received,
                )
            );
        }

        let later_receive = loop {
            let candidate = role.clock.observe().unwrap();
            if candidate != received {
                break candidate;
            }
            std::hint::spin_loop();
        };
        assert_ne!(
            base,
            live_metadata_observation_commitment(
                OriginMode::LocalEvidence,
                scope(),
                market.as_bytes(),
                clob.as_bytes(),
                observation.details(),
                later_receive,
            )
        );
        task.await.unwrap();
    }

    #[tokio::test]
    async fn either_http_failure_or_redirect_releases_no_pair() {
        let mut redirect = MockResponse::status(302);
        redirect.location = Some("/clob-markets/elsewhere");
        let (origin, mut requests, task) = mock_server(vec![
            MockResponse::ok(long_market()),
            MockResponse::status(503),
            MockResponse::ok(long_market()),
            redirect,
            MockResponse::status(503),
        ])
        .await;
        let role = local_role(&origin);
        let mut sink = ValidatingSink::default();
        assert!(matches!(
            role.refresh(&mut sink).await,
            Err(PmPublicMetadataDeliveryError::Http(
                PmLiveAdapterError::UnexpectedStatus { status: 503 }
            ))
        ));
        assert!(matches!(
            role.refresh(&mut sink).await,
            Err(PmPublicMetadataDeliveryError::Http(
                PmLiveAdapterError::Redirect { status: 302 }
            ))
        ));
        assert!(matches!(
            role.refresh(&mut sink).await,
            Err(PmPublicMetadataDeliveryError::Http(
                PmLiveAdapterError::UnexpectedStatus { status: 503 }
            ))
        ));
        assert_eq!(sink.calls, 0);
        let mut count = 0;
        while requests.try_recv().is_ok() {
            count += 1;
        }
        assert_eq!(count, 5);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn either_body_bound_failure_releases_no_pair() {
        let advertised = MockResponse {
            status: 200,
            body: Vec::new(),
            location: None,
            content_length: Some(MAX_PUBLIC_REST_BODY_BYTES + 1),
        };
        let streamed = MockResponse {
            status: 200,
            body: vec![b'x'; MAX_PUBLIC_REST_BODY_BYTES + 1],
            location: None,
            content_length: None,
        };
        let (origin, _requests, task) =
            mock_server(vec![advertised, MockResponse::ok(long_market()), streamed]).await;
        let role = local_role(&origin);
        let mut sink = ValidatingSink::default();
        for _ in 0..2 {
            assert!(matches!(
                role.refresh(&mut sink).await,
                Err(PmPublicMetadataDeliveryError::Http(
                    PmLiveAdapterError::ResponseBodyTooLarge {
                        limit: MAX_PUBLIC_REST_BODY_BYTES
                    }
                ))
            ));
        }
        assert_eq!(sink.calls, 0);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn sink_validation_failure_is_typed_after_atomic_delivery() {
        let wrong = long_market().replace(CONDITION, MARKET);
        let (origin, _requests, task) = mock_server(vec![
            MockResponse::ok(wrong),
            MockResponse::ok(short_market()),
        ])
        .await;
        let role = local_role(&origin);
        let mut sink = ValidatingSink::default();
        assert!(matches!(
            role.refresh(&mut sink).await,
            Err(PmPublicMetadataDeliveryError::Sink(
                PmWireError::ConditionMismatch
            ))
        ));
        assert_eq!(sink.calls, 1);
        task.await.unwrap();
    }
}
