use std::{fmt, time::Duration};

use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    PM_CLOB_PRODUCTION_ORIGIN, PmHttpReceiveClock, PmLiveAdapterError, PmPublicHttpConfig,
    config::OriginMode,
    http_transport::{PmHttpTransport, PmPublicRoute},
    observation_clock::PmHttpReceiveClockSource,
};

/// The exact body currently returned by the documented CLOB health check.
/// Any response drift fails closed until it is reviewed and versioned.
const EXACT_CLOB_LIVENESS_HEALTH_BODY: &[u8; 4] = b"\"OK\"";
const CLOB_LIVENESS_HEALTH_OBSERVATION_COMMITMENT_DOMAIN: &[u8] =
    b"reap.pm.live-adapter.clob-liveness-health-observation.v1\0";

/// Closed error surface for the fixed CLOB liveness/health observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmClobLivenessHealthError {
    #[error(transparent)]
    Http(#[from] PmLiveAdapterError),
    #[error("CLOB liveness/health response did not match the exact reviewed body")]
    ExactBodyMismatch,
}

/// Domain-separated SHA-256 commitment to one exact fixed CLOB `/ok`
/// liveness/health response and its source-captured receive edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmClobLivenessHealthObservationCommitment([u8; 32]);

impl PmClobLivenessHealthObservationCommitment {
    const fn from_source_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Move-only observation of the fixed CLOB `/ok` response.
///
/// This is liveness/health evidence only. It is not evidence about matching
/// engine restart state, restricted/cancel-only/post-only mode, global order
/// admission, account state, or the egress used by any other connection.
pub struct PmClobLivenessHealthObservation {
    receive_clock: PmHttpReceiveClock,
    commitment: PmClobLivenessHealthObservationCommitment,
}

impl PmClobLivenessHealthObservation {
    fn from_source(
        receive_clock: PmHttpReceiveClock,
        commitment: PmClobLivenessHealthObservationCommitment,
    ) -> Self {
        Self {
            receive_clock,
            commitment,
        }
    }

    #[must_use]
    pub const fn receive_clock(&self) -> PmHttpReceiveClock {
        self.receive_clock
    }

    #[must_use]
    pub const fn commitment(&self) -> PmClobLivenessHealthObservationCommitment {
        self.commitment
    }
}

impl fmt::Debug for PmClobLivenessHealthObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmClobLivenessHealthObservation(<fixed-GET-/ok; sealed>)")
    }
}

/// Move-only proof that a fixed `/ok` observation came from the role's
/// private production-origin mode. LocalEvidence cannot construct it.
pub struct PmProductionClobLivenessHealthObservation {
    observation: PmClobLivenessHealthObservation,
}

impl PmProductionClobLivenessHealthObservation {
    fn from_source(
        _production_origin: ProductionClobHealthOrigin,
        observation: PmClobLivenessHealthObservation,
    ) -> Self {
        Self { observation }
    }

    #[must_use]
    pub const fn receive_clock(&self) -> PmHttpReceiveClock {
        self.observation.receive_clock()
    }

    #[must_use]
    pub const fn commitment(&self) -> PmClobLivenessHealthObservationCommitment {
        self.observation.commitment()
    }
}

impl fmt::Debug for PmProductionClobLivenessHealthObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "PmProductionClobLivenessHealthObservation(<production-origin; liveness-health-only; sealed>)",
        )
    }
}

struct ProductionClobHealthOrigin;

impl ProductionClobHealthOrigin {
    fn verify(mode: OriginMode) -> Result<Self, PmLiveAdapterError> {
        match mode {
            OriginMode::Production => Ok(Self),
            #[cfg(any(test, feature = "loopback-evidence", feature = "read-only-evidence"))]
            OriginMode::LocalEvidence => Err(PmLiveAdapterError::InvalidConfiguration(
                "production CLOB liveness/health observation requires the fixed production origin",
            )),
        }
    }
}

/// Purpose-closed fixed CLOB `GET /ok` role.
///
/// The default production constructor accepts only timeouts; the selected
/// form additionally accepts non-authoritative local socket configuration.
/// Neither accepts a caller-selected origin, path, method, body, retry, proxy,
/// hash, or clock. The underlying
/// transport is the same no-proxy/no-retry bounded CLOB transport used by the
/// other fixed public roles, but this independent observation does not prove
/// that another connection used the same route or egress.
pub struct PmClobLivenessHealthHttpRole {
    transport: PmHttpTransport,
    mode: OriginMode,
    clock: PmHttpReceiveClockSource,
}

impl PmClobLivenessHealthHttpRole {
    pub fn production(
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, PmLiveAdapterError> {
        let config = PmPublicHttpConfig::production(
            PM_CLOB_PRODUCTION_ORIGIN,
            connect_timeout,
            request_timeout,
        )?;
        Self::from_config(config)
    }

    /// Construct the fixed production `GET /ok` role with a validated,
    /// caller-provided interface name and local source IP applied to its
    /// private client. The selection remains non-authoritative configuration and
    /// grants no mutation authority.
    pub fn production_on_selected_local_egress(
        connect_timeout: Duration,
        request_timeout: Duration,
        selected_local_egress: reap_polymarket_egress_binding::PmLocalEgressSelection,
    ) -> Result<Self, PmLiveAdapterError> {
        let config = PmPublicHttpConfig::production_on_selected_local_egress(
            PM_CLOB_PRODUCTION_ORIGIN,
            connect_timeout,
            request_timeout,
            selected_local_egress,
        )?;
        Self::from_config(config)
    }

    #[cfg(any(test, feature = "read-only-evidence"))]
    pub fn read_only_evidence(
        origin: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, PmLiveAdapterError> {
        let config =
            PmPublicHttpConfig::read_only_evidence(origin, connect_timeout, request_timeout)?;
        Self::from_config(config)
    }

    fn from_config(config: PmPublicHttpConfig) -> Result<Self, PmLiveAdapterError> {
        let mode = config.mode();
        Ok(Self {
            transport: PmHttpTransport::new(&config)?,
            mode,
            clock: PmHttpReceiveClockSource::system(),
        })
    }

    /// Fetch and seal the exact current `GET /ok` response. The receive edge
    /// is sampled after the four-byte bounded body completes and before the
    /// exact-body comparison.
    pub async fn liveness_health_observation(
        &self,
    ) -> Result<PmClobLivenessHealthObservation, PmClobLivenessHealthError> {
        let body = self
            .transport
            .get(
                PmPublicRoute::ClobHealth,
                EXACT_CLOB_LIVENESS_HEALTH_BODY.len(),
            )
            .await?;
        let receive_clock = self.clock.observe()?;
        if body.as_slice() != EXACT_CLOB_LIVENESS_HEALTH_BODY {
            return Err(PmClobLivenessHealthError::ExactBodyMismatch);
        }
        let commitment =
            clob_liveness_health_observation_commitment(self.mode, &body, receive_clock);
        Ok(PmClobLivenessHealthObservation::from_source(
            receive_clock,
            commitment,
        ))
    }

    /// Verify the private production mode before I/O, then fetch and seal a
    /// move-only production-origin liveness/health observation.
    pub async fn production_liveness_health_observation(
        &self,
    ) -> Result<PmProductionClobLivenessHealthObservation, PmClobLivenessHealthError> {
        let production_origin = ProductionClobHealthOrigin::verify(self.mode)?;
        let observation = self.liveness_health_observation().await?;
        Ok(PmProductionClobLivenessHealthObservation::from_source(
            production_origin,
            observation,
        ))
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }
}

impl fmt::Debug for PmClobLivenessHealthHttpRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmClobLivenessHealthHttpRole(<fixed-GET-/ok>)")
    }
}

fn clob_liveness_health_observation_commitment(
    mode: OriginMode,
    raw_response: &[u8],
    receive_clock: PmHttpReceiveClock,
) -> PmClobLivenessHealthObservationCommitment {
    let mut digest = Sha256::new();
    encode_health_bytes(
        &mut digest,
        CLOB_LIVENESS_HEALTH_OBSERVATION_COMMITMENT_DOMAIN,
    );
    encode_health_bytes(&mut digest, origin_mode_name(mode));
    encode_health_bytes(&mut digest, PM_CLOB_PRODUCTION_ORIGIN.as_bytes());
    encode_health_bytes(&mut digest, b"GET");
    encode_health_bytes(&mut digest, b"/ok");
    digest.update(receive_clock.local_wall_receive_ns().to_be_bytes());
    digest.update(receive_clock.monotonic_receive_ns().to_be_bytes());
    encode_health_bytes(&mut digest, raw_response);
    PmClobLivenessHealthObservationCommitment::from_source_bytes(digest.finalize().into())
}

fn encode_health_bytes(digest: &mut Sha256, value: &[u8]) {
    digest.update(
        u64::try_from(value.len())
            .expect("bounded CLOB health commitment field length fits u64")
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

#[cfg(test)]
mod tests {
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
                requests_tx.send(String::from_utf8(raw).unwrap()).unwrap();
                sleep(response.delay).await;
                let reason = match response.status {
                    200 => "OK",
                    302 => "Found",
                    _ => "Mock",
                };
                let mut headers = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: text/plain; charset=UTF-8\r\nConnection: close\r\n",
                    response.status, reason,
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

    fn local_role(origin: &str, request_timeout: Duration) -> PmClobLivenessHealthHttpRole {
        PmClobLivenessHealthHttpRole::read_only_evidence(
            origin,
            Duration::from_millis(100),
            request_timeout,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn exact_current_body_is_the_only_success_and_route_is_fixed() {
        let (origin, mut requests, server) = mock_server(vec![MockResponse::ok(b"\"OK\"")]).await;
        let role = local_role(&origin, Duration::from_secs(1));
        let observation = role.liveness_health_observation().await.unwrap();
        assert!(observation.receive_clock().local_wall_receive_ns() > 0);
        assert!(observation.receive_clock().monotonic_receive_ns() > 0);
        assert_ne!(observation.commitment().bytes(), [0; 32]);
        assert!(!role.production_order_entry_authorized());
        assert_eq!(
            format!("{observation:?}"),
            "PmClobLivenessHealthObservation(<fixed-GET-/ok; sealed>)",
        );

        let request = requests.recv().await.unwrap();
        assert!(request.starts_with("GET /ok HTTP/1.1\r\n"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("accept: text/plain\r\n")
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn unquoted_case_newline_whitespace_and_trailing_bytes_fail_closed() {
        let invalid = [
            b"OK".as_slice(),
            b"\"ok\"".as_slice(),
            b"\"OK\"\n".as_slice(),
            b"\"OK\" ".as_slice(),
            b"true".as_slice(),
            b"".as_slice(),
        ];
        let responses = invalid
            .iter()
            .map(|body| MockResponse::ok(body.to_vec()))
            .collect();
        let (origin, _requests, server) = mock_server(responses).await;
        let role = local_role(&origin, Duration::from_secs(1));
        for _ in invalid {
            assert!(matches!(
                role.liveness_health_observation().await,
                Err(PmClobLivenessHealthError::ExactBodyMismatch)
                    | Err(PmClobLivenessHealthError::Http(
                        PmLiveAdapterError::ResponseBodyTooLarge { limit: 4 }
                    )),
            ));
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn declared_and_streamed_oversize_bodies_are_bounded() {
        let responses = vec![
            MockResponse {
                status: 200,
                body: b"\"OK\"x".to_vec(),
                delay: Duration::ZERO,
                location: None,
                content_length: Some(5),
            },
            MockResponse {
                status: 200,
                body: b"\"OK\"x".to_vec(),
                delay: Duration::ZERO,
                location: None,
                content_length: None,
            },
        ];
        let (origin, _requests, server) = mock_server(responses).await;
        let role = local_role(&origin, Duration::from_secs(1));
        for _ in 0..2 {
            assert!(matches!(
                role.liveness_health_observation().await,
                Err(PmClobLivenessHealthError::Http(
                    PmLiveAdapterError::ResponseBodyTooLarge { limit: 4 }
                )),
            ));
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn redirects_non_success_and_timeout_fail_closed() {
        let responses = vec![
            MockResponse {
                status: 302,
                body: Vec::new(),
                delay: Duration::ZERO,
                location: Some("/ok"),
                content_length: Some(0),
            },
            MockResponse {
                status: 503,
                body: Vec::new(),
                delay: Duration::ZERO,
                location: None,
                content_length: Some(0),
            },
            MockResponse {
                status: 200,
                body: b"\"OK\"".to_vec(),
                delay: Duration::from_millis(100),
                location: None,
                content_length: Some(4),
            },
        ];
        let (origin, _requests, server) = mock_server(responses).await;
        let role = local_role(&origin, Duration::from_millis(20));
        assert!(matches!(
            role.liveness_health_observation().await,
            Err(PmClobLivenessHealthError::Http(
                PmLiveAdapterError::Redirect { status: 302 }
            )),
        ));
        assert!(matches!(
            role.liveness_health_observation().await,
            Err(PmClobLivenessHealthError::Http(
                PmLiveAdapterError::UnexpectedStatus { status: 503 }
            )),
        ));
        assert!(matches!(
            role.liveness_health_observation().await,
            Err(PmClobLivenessHealthError::Http(
                PmLiveAdapterError::RequestTimeout
            )),
        ));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn local_evidence_cannot_issue_production_proof_and_checks_before_io() {
        let role = local_role("http://127.0.0.1:9", Duration::from_millis(50));
        assert!(matches!(
            role.production_liveness_health_observation().await,
            Err(PmClobLivenessHealthError::Http(
                PmLiveAdapterError::InvalidConfiguration(
                    "production CLOB liveness/health observation requires the fixed production origin"
                )
            )),
        ));
    }

    #[test]
    fn production_origin_proof_accepts_only_production_mode() {
        assert!(ProductionClobHealthOrigin::verify(OriginMode::Production).is_ok());
        assert!(matches!(
            ProductionClobHealthOrigin::verify(OriginMode::LocalEvidence),
            Err(PmLiveAdapterError::InvalidConfiguration(_)),
        ));
    }

    #[tokio::test]
    async fn receive_edge_is_committed_and_production_wrapper_debug_is_redacted() {
        let (origin, _requests, server) = mock_server(vec![
            MockResponse::ok(b"\"OK\""),
            MockResponse::ok(b"\"OK\""),
        ])
        .await;
        let role = local_role(&origin, Duration::from_secs(1));
        let first = role.liveness_health_observation().await.unwrap();
        let second = role.liveness_health_observation().await.unwrap();
        assert_ne!(first.receive_clock(), second.receive_clock());
        assert_ne!(first.commitment(), second.commitment());

        let production = PmProductionClobLivenessHealthObservation::from_source(
            ProductionClobHealthOrigin,
            first,
        );
        assert_eq!(
            format!("{production:?}"),
            "PmProductionClobLivenessHealthObservation(<production-origin; liveness-health-only; sealed>)",
        );
        server.await.unwrap();
    }
}
