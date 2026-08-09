use reap_polymarket_wire::{MAX_PM_GEOBLOCK_BODY_BYTES, PmGeoblockStatus, parse_pm_geoblock};
use sha2::{Digest as _, Sha256};

use crate::{
    PM_GEOBLOCK_PRODUCTION_ORIGIN, PmGeoblockHttpConfig, PmHttpReceiveClock, PmLiveAdapterError,
    config::OriginMode,
    http_transport::{PmHttpTransport, PmPublicRoute},
    observation_clock::PmHttpReceiveClockSource,
};

const GEOBLOCK_OBSERVATION_COMMITMENT_DOMAIN: &[u8] =
    b"reap.pm.live-adapter.geoblock-observation.v1\0";

/// Domain-separated SHA-256 commitment to one fixed geoblock observation.
/// It binds the production/evidence mode, exact route, raw response, parsed
/// status, and source-captured receive edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmGeoblockObservationCommitment([u8; 32]);

impl PmGeoblockObservationCommitment {
    const fn from_source_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Sealed result of the fixed public geoblock source. The raw response is
/// committed but cannot escape this carrier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmGeoblockObservation {
    status: PmGeoblockStatus,
    receive_clock: PmHttpReceiveClock,
    commitment: PmGeoblockObservationCommitment,
}

impl PmGeoblockObservation {
    fn from_source(
        status: PmGeoblockStatus,
        receive_clock: PmHttpReceiveClock,
        commitment: PmGeoblockObservationCommitment,
    ) -> Self {
        Self {
            status,
            receive_clock,
            commitment,
        }
    }

    #[must_use]
    pub const fn status(&self) -> &PmGeoblockStatus {
        &self.status
    }

    #[must_use]
    pub const fn receive_clock(&self) -> PmHttpReceiveClock {
        self.receive_clock
    }

    #[must_use]
    pub const fn commitment(&self) -> PmGeoblockObservationCommitment {
        self.commitment
    }

    #[must_use]
    pub fn into_status(self) -> PmGeoblockStatus {
        self.status
    }
}

/// Fixed public geoblock GET. It has no route, origin, body, retry, or mutation
/// input after construction.
pub struct PmGeoblockHttpRole {
    transport: PmHttpTransport,
    mode: OriginMode,
    clock: PmHttpReceiveClockSource,
}

impl PmGeoblockHttpRole {
    pub fn new(config: PmGeoblockHttpConfig) -> Result<Self, PmLiveAdapterError> {
        let mode = config.mode();
        Ok(Self {
            transport: PmHttpTransport::geoblock(&config)?,
            mode,
            clock: PmHttpReceiveClockSource::system(),
        })
    }

    pub async fn status(&self) -> Result<PmGeoblockStatus, PmLiveAdapterError> {
        Ok(self.status_observation().await?.into_status())
    }

    /// Fetch, source-time, strictly parse, and seal the fixed geoblock
    /// response. The receive edge is sampled after the bounded body completes
    /// and before parsing.
    pub async fn status_observation(&self) -> Result<PmGeoblockObservation, PmLiveAdapterError> {
        let body = self
            .transport
            .get(PmPublicRoute::Geoblock, MAX_PM_GEOBLOCK_BODY_BYTES)
            .await?;
        let receive_clock = self.clock.observe()?;
        let status = parse_pm_geoblock(&body)?;
        let commitment = geoblock_observation_commitment(self.mode, &body, &status, receive_clock);
        Ok(PmGeoblockObservation::from_source(
            status,
            receive_clock,
            commitment,
        ))
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }
}

fn geoblock_observation_commitment(
    mode: OriginMode,
    raw_response: &[u8],
    status: &PmGeoblockStatus,
    receive_clock: PmHttpReceiveClock,
) -> PmGeoblockObservationCommitment {
    let mut digest = Sha256::new();
    encode_geoblock_bytes(&mut digest, GEOBLOCK_OBSERVATION_COMMITMENT_DOMAIN);
    encode_geoblock_bytes(&mut digest, origin_mode_name(mode));
    encode_geoblock_bytes(&mut digest, PM_GEOBLOCK_PRODUCTION_ORIGIN.as_bytes());
    encode_geoblock_bytes(&mut digest, b"GET");
    encode_geoblock_bytes(&mut digest, b"/api/geoblock");
    digest.update(receive_clock.local_wall_receive_ns().to_be_bytes());
    digest.update(receive_clock.monotonic_receive_ns().to_be_bytes());
    encode_geoblock_bytes(&mut digest, raw_response);
    digest.update([u8::from(status.blocked())]);
    encode_geoblock_bytes(&mut digest, status.ip().to_string().as_bytes());
    encode_geoblock_bytes(&mut digest, status.country().as_bytes());
    encode_geoblock_bytes(&mut digest, status.region().as_bytes());
    PmGeoblockObservationCommitment::from_source_bytes(digest.finalize().into())
}

fn encode_geoblock_bytes(digest: &mut Sha256, value: &[u8]) {
    digest.update(
        u64::try_from(value.len())
            .expect("bounded geoblock commitment field length fits u64")
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

impl std::fmt::Debug for PmGeoblockHttpRole {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PmGeoblockHttpRole")
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::mpsc,
        task::JoinHandle,
        time::sleep,
    };

    use super::*;

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
                let _ = requests_tx.send(String::from_utf8(raw).unwrap());
                sleep(response.delay).await;
                let reason = match response.status {
                    200 => "OK",
                    302 => "Found",
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

    fn local_role(origin: &str, request_timeout: Duration) -> PmGeoblockHttpRole {
        let config = PmGeoblockHttpConfig::read_only_evidence(
            origin,
            Duration::from_millis(100),
            request_timeout,
        )
        .unwrap();
        PmGeoblockHttpRole::new(config).unwrap()
    }

    #[tokio::test]
    async fn fixed_get_returns_strict_typed_status() {
        let (origin, mut requests, task) = mock_server(vec![MockResponse::ok(
            br#"{"blocked":false,"ip":"203.0.113.9","country":"US","region":"NY"}"#,
        )])
        .await;
        let role = local_role(&origin, Duration::from_secs(1));
        let status = role.status().await.unwrap();
        assert!(!status.blocked());
        assert_eq!(status.ip().to_string(), "203.0.113.9");
        assert_eq!(status.country(), "US");
        assert_eq!(status.region(), "NY");
        assert!(!role.production_order_entry_authorized());

        let request = requests.recv().await.unwrap();
        assert_eq!(request.lines().next(), Some("GET /api/geoblock HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("accept: application/json")
        );
        task.await.unwrap();
        assert!(requests.try_recv().is_err());
    }

    #[tokio::test]
    async fn observation_seals_status_at_the_source_receive_edge() {
        let raw = br#"{"blocked":false,"ip":"203.0.113.9","country":"US","region":"NY"}"#;
        let (origin, mut requests, task) = mock_server(vec![MockResponse::ok(raw)]).await;
        let role = local_role(&origin, Duration::from_secs(1));
        let observation = role.status_observation().await.unwrap();

        assert!(!observation.status().blocked());
        assert_eq!(observation.status().ip().to_string(), "203.0.113.9");
        assert_eq!(observation.status().country(), "US");
        assert_eq!(observation.status().region(), "NY");
        assert!(observation.receive_clock().local_wall_receive_ns() > 0);
        assert!(observation.receive_clock().monotonic_receive_ns() > 0);
        assert_ne!(observation.commitment().bytes(), [0; 32]);
        assert_eq!(
            requests.recv().await.unwrap().lines().next(),
            Some("GET /api/geoblock HTTP/1.1")
        );
        task.await.unwrap();
    }

    #[tokio::test]
    async fn loopback_geoblock_commitment_changes_for_every_observed_input_class() {
        let raw = br#"{"blocked":false,"ip":"203.0.113.9","country":"US","region":"NY"}"#;
        let (origin, _requests, task) = mock_server(vec![MockResponse::ok(raw)]).await;
        let role = local_role(&origin, Duration::from_secs(1));
        let observation = role.status_observation().await.unwrap();
        let received = observation.receive_clock();
        let base = geoblock_observation_commitment(
            OriginMode::LocalEvidence,
            raw,
            observation.status(),
            received,
        );
        assert_eq!(base, observation.commitment());

        assert_ne!(
            base,
            geoblock_observation_commitment(
                OriginMode::Production,
                raw,
                observation.status(),
                received,
            )
        );

        let mut raw_mutation = raw.to_vec();
        raw_mutation.push(b' ');
        assert_ne!(
            base,
            geoblock_observation_commitment(
                OriginMode::LocalEvidence,
                &raw_mutation,
                observation.status(),
                received,
            )
        );

        let alternate = parse_pm_geoblock(
            br#"{"blocked":true,"ip":"203.0.113.9","country":"US","region":"NY"}"#,
        )
        .unwrap();
        assert_ne!(
            base,
            geoblock_observation_commitment(OriginMode::LocalEvidence, raw, &alternate, received,)
        );

        let later_receive = loop {
            let candidate = role.clock.observe().unwrap();
            if candidate != received {
                break candidate;
            }
            std::hint::spin_loop();
        };
        assert_ne!(
            base,
            geoblock_observation_commitment(
                OriginMode::LocalEvidence,
                raw,
                observation.status(),
                later_receive,
            )
        );
        task.await.unwrap();
    }

    #[tokio::test]
    async fn redirect_is_not_followed_or_retried() {
        let (origin, mut requests, task) = mock_server(vec![MockResponse {
            status: 302,
            body: b"{}".to_vec(),
            delay: Duration::ZERO,
            location: Some("/api/geoblock"),
            content_length: Some(2),
        }])
        .await;
        let role = local_role(&origin, Duration::from_secs(1));
        assert!(matches!(
            role.status().await,
            Err(PmLiveAdapterError::Redirect { status: 302 })
        ));
        assert!(requests.recv().await.is_some());
        task.await.unwrap();
        assert!(requests.try_recv().is_err());
    }

    #[tokio::test]
    async fn advertised_and_streamed_bodies_share_the_strict_cap() {
        let responses = vec![
            MockResponse {
                status: 200,
                body: Vec::new(),
                delay: Duration::ZERO,
                location: None,
                content_length: Some(MAX_PM_GEOBLOCK_BODY_BYTES + 1),
            },
            MockResponse {
                status: 200,
                body: vec![b'x'; MAX_PM_GEOBLOCK_BODY_BYTES + 1],
                delay: Duration::ZERO,
                location: None,
                content_length: None,
            },
        ];
        let (origin, _requests, task) = mock_server(responses).await;
        let role = local_role(&origin, Duration::from_secs(1));
        for _ in 0..2 {
            assert!(matches!(
                role.status().await,
                Err(PmLiveAdapterError::ResponseBodyTooLarge {
                    limit: MAX_PM_GEOBLOCK_BODY_BYTES
                })
            ));
        }
        task.await.unwrap();
    }

    #[tokio::test]
    async fn request_timeout_is_enforced() {
        let mut response = MockResponse::ok(
            br#"{"blocked":false,"ip":"203.0.113.9","country":"US","region":"NY"}"#,
        );
        response.delay = Duration::from_millis(100);
        let (origin, _requests, task) = mock_server(vec![response]).await;
        let role = local_role(&origin, Duration::from_millis(20));
        assert!(matches!(
            role.status().await,
            Err(PmLiveAdapterError::RequestTimeout)
        ));
        task.await.unwrap();
    }
}
