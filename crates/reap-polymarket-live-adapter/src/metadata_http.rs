use std::fmt;

use reap_polymarket_wire::{MAX_PUBLIC_REST_BODY_BYTES, PmWireScope};

use crate::{
    PmLiveAdapterError, PmPublicHttpConfig, PmPublicMetadataDeliveryError,
    http_transport::{PmHttpTransport, PmPublicRoute},
};

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

/// Exact two-route public metadata capability for one configured PM outcome.
#[derive(Clone)]
pub struct PmPublicMetadataHttpRole {
    transport: PmHttpTransport,
    scope: PmWireScope,
}

impl PmPublicMetadataHttpRole {
    pub fn new(config: PmPublicHttpConfig, scope: PmWireScope) -> Result<Self, PmLiveAdapterError> {
        let transport = PmHttpTransport::new(&config)?;
        Ok(Self { transport, scope })
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
        let market_bytes = self
            .transport
            .get(
                PmPublicRoute::MarketMetadata(self.scope.condition()),
                MAX_PUBLIC_REST_BODY_BYTES,
            )
            .await
            .map_err(PmPublicMetadataDeliveryError::Http)?;
        let clob_v2_bytes = self
            .transport
            .get(
                PmPublicRoute::ClobV2Metadata(self.scope.condition()),
                MAX_PUBLIC_REST_BODY_BYTES,
            )
            .await
            .map_err(PmPublicMetadataDeliveryError::Http)?;
        sink.deliver_native_metadata_pair(PmLiveMetadataPair {
            scope: self.scope,
            market_bytes: &market_bytes,
            clob_v2_bytes: &clob_v2_bytes,
        })
        .map_err(PmPublicMetadataDeliveryError::Sink)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use reap_pm_core::{PmConditionId, PmMarketId, PmQuantity, PmTokenId, U256};
    use reap_polymarket_wire::{
        PmBookMarketBinding, PmBookParserConfig, PmClobV2RequestScope, PmWireError,
        parse_live_clob_market_lifecycle, parse_live_clob_v2_metadata,
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
            r#"{{"condition_id":"{CONDITION}","question_id":"{MARKET}","active":true,"closed":false,"archived":false,"accepting_orders":true,"enable_order_book":true}}"#
        )
    }

    fn short_market() -> String {
        format!(
            r#"{{"c":"{CONDITION}","t":[{{"t":"123","o":"Yes"}},{{"t":"456","o":"No"}}],"mts":0.01,"mos":5,"nr":false}}"#
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
