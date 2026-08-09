use reap_polymarket_wire::{MAX_PM_GEOBLOCK_BODY_BYTES, PmGeoblockStatus, parse_pm_geoblock};

use crate::{
    PmGeoblockHttpConfig, PmLiveAdapterError,
    http_transport::{PmHttpTransport, PmPublicRoute},
};

/// Fixed public geoblock GET. It has no route, origin, body, retry, or mutation
/// input after construction.
pub struct PmGeoblockHttpRole {
    transport: PmHttpTransport,
}

impl PmGeoblockHttpRole {
    pub fn new(config: PmGeoblockHttpConfig) -> Result<Self, PmLiveAdapterError> {
        Ok(Self {
            transport: PmHttpTransport::geoblock(&config)?,
        })
    }

    pub async fn status(&self) -> Result<PmGeoblockStatus, PmLiveAdapterError> {
        let body = self
            .transport
            .get(PmPublicRoute::Geoblock, MAX_PM_GEOBLOCK_BODY_BYTES)
            .await?;
        parse_pm_geoblock(&body).map_err(Into::into)
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
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
