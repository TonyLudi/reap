use std::net::SocketAddr;

use reap_pm_core::{PmConditionId, PmTokenId};
use reap_polymarket_egress_binding::{PmFixedTlsPeerSelection, PmLocalEgressSelection};
use reqwest::{Client, StatusCode, Url, redirect::Policy};

use crate::{
    PmLiveAdapterError,
    config::{OriginMode, PmGeoblockHttpConfig, PmPublicHttpConfig, PmStatusHttpConfig},
};

pub(crate) enum PmPublicRoute {
    ClobHealth,
    ServerTime,
    Book(PmTokenId),
    MarketMetadata(PmConditionId),
    ClobV2Metadata(PmConditionId),
    Geoblock,
    StatusSummary,
    StatusComponents,
}

impl PmPublicRoute {
    const fn accept(&self) -> &'static str {
        match self {
            Self::ClobHealth => "text/plain",
            Self::ServerTime
            | Self::Book(_)
            | Self::MarketMetadata(_)
            | Self::ClobV2Metadata(_)
            | Self::Geoblock
            | Self::StatusSummary
            | Self::StatusComponents => "application/json",
        }
    }
}

#[derive(Clone)]
pub(crate) struct PmHttpTransport {
    client: Client,
    origin: Url,
    expected_peer: Option<SocketAddr>,
}

impl PmHttpTransport {
    pub(crate) fn new(config: &PmPublicHttpConfig) -> Result<Self, PmLiveAdapterError> {
        Self::build(
            config.origin().clone(),
            config.connect_timeout(),
            config.request_timeout(),
            config.mode(),
            config.selected_local_egress(),
            config.fixed_tls_peer(),
        )
    }

    pub(crate) fn geoblock(config: &PmGeoblockHttpConfig) -> Result<Self, PmLiveAdapterError> {
        Self::build(
            config.origin().clone(),
            config.connect_timeout(),
            config.request_timeout(),
            config.mode(),
            config.selected_local_egress(),
            config.fixed_tls_peer(),
        )
    }

    pub(crate) fn status(config: &PmStatusHttpConfig) -> Result<Self, PmLiveAdapterError> {
        Self::build(
            config.origin().clone(),
            config.connect_timeout(),
            config.request_timeout(),
            config.mode(),
            config.selected_local_egress(),
            config.fixed_tls_peer(),
        )
    }

    fn build(
        origin: Url,
        connect_timeout: std::time::Duration,
        request_timeout: std::time::Duration,
        mode: OriginMode,
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
            expected_peer: fixed_tls_peer.map(PmFixedTlsPeerSelection::peer_addr),
        })
    }

    pub(crate) async fn get(
        &self,
        route: PmPublicRoute,
        maximum_body_bytes: usize,
    ) -> Result<Vec<u8>, PmLiveAdapterError> {
        let accept = route.accept();
        let url = self.route_url(route);
        let mut response = self
            .client
            .get(url)
            .header(reqwest::header::ACCEPT, accept)
            .send()
            .await
            .map_err(map_request_error)?;
        validate_response_peer(self.expected_peer, response.remote_addr())?;
        let status = response.status();
        if status.is_redirection() {
            return Err(PmLiveAdapterError::Redirect {
                status: status.as_u16(),
            });
        }
        if status != StatusCode::OK {
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
        let mut body = Vec::with_capacity(capacity);
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
        Ok(body)
    }

    fn route_url(&self, route: PmPublicRoute) -> Url {
        let mut url = self.origin.clone();
        match route {
            PmPublicRoute::ClobHealth => url.set_path("/ok"),
            PmPublicRoute::ServerTime => url.set_path("/time"),
            PmPublicRoute::Book(token) => {
                url.set_path("/book");
                url.query_pairs_mut()
                    .append_pair("token_id", &token.units().to_string());
            }
            PmPublicRoute::MarketMetadata(condition) => {
                url.set_path(&format!("/markets/{condition}"));
            }
            PmPublicRoute::ClobV2Metadata(condition) => {
                url.set_path(&format!("/clob-markets/{condition}"));
            }
            PmPublicRoute::Geoblock => url.set_path("/api/geoblock"),
            PmPublicRoute::StatusSummary => url.set_path("/v3/summary.json"),
            PmPublicRoute::StatusComponents => url.set_path("/v3/components.json"),
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

    use reap_polymarket_egress_binding::{PmFixedTlsPeerSelection, PmLocalEgressSelection};
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::TcpListener,
        time::timeout,
    };

    use super::*;
    use crate::PmPublicHttpConfig;

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn selected_client_uses_the_nondefault_loopback_source_ip_for_the_fixed_get() {
        let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut stream, peer) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 2_048];
            let read = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8(request[..read].to_vec()).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                )
                .await
                .unwrap();
            (peer.ip(), request)
        });
        let selected_ip = "127.0.0.2".parse::<IpAddr>().unwrap();
        let selection = PmLocalEgressSelection::loopback_evidence("lo", selected_ip).unwrap();
        let config = PmPublicHttpConfig::local_evidence_on_selected_local_egress(
            &format!("http://127.0.0.1:{port}"),
            Duration::from_secs(1),
            Duration::from_secs(1),
            selection,
        )
        .unwrap();
        let transport = PmHttpTransport::new(&config).unwrap();
        assert_eq!(
            transport.get(PmPublicRoute::ServerTime, 32).await.unwrap(),
            b"{}"
        );
        let (peer_ip, request) = server.await.unwrap();
        assert_eq!(peer_ip, selected_ip);
        assert!(request.starts_with("GET /time HTTP/1.1\r\n"));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn nonexistent_selected_interface_fails_before_any_fixed_get_arrives() {
        let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let selection =
            PmLocalEgressSelection::loopback_evidence("missing0", "127.0.0.2".parse().unwrap())
                .unwrap();
        let config = PmPublicHttpConfig::local_evidence_on_selected_local_egress(
            &format!("http://127.0.0.1:{port}"),
            Duration::from_millis(100),
            Duration::from_millis(100),
            selection,
        )
        .unwrap();
        let transport = PmHttpTransport::new(&config).unwrap();
        assert!(transport.get(PmPublicRoute::ServerTime, 32).await.is_err());
        assert!(
            timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn fixed_peer_client_uses_exact_host_source_and_peer_while_decoy_stays_idle() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let exact_peer = listener.local_addr().unwrap();
        let decoy = TcpListener::bind(("127.0.0.3", exact_peer.port()))
            .await
            .unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, source) = listener.accept().await.unwrap();
            assert_eq!(stream.peer_addr().unwrap(), source);
            let mut request = vec![0_u8; 2_048];
            let read = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8(request[..read].to_vec()).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
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
            PmPublicHttpConfig::loopback_evidence_on_fixed_tls_peer_and_selected_local_egress(
                Duration::from_secs(1),
                Duration::from_secs(1),
                fixed_peer,
                local_egress,
            )
            .unwrap();
        let transport = PmHttpTransport::new(&config).unwrap();
        assert_eq!(
            transport.get(PmPublicRoute::ServerTime, 32).await.unwrap(),
            b"{}"
        );
        let (source, request) = server.await.unwrap();
        assert_eq!(source.ip(), selected_source);
        assert!(request.starts_with("GET /time HTTP/1.1\r\n"));
        assert!(request.to_ascii_lowercase().contains(&format!(
            "host: clob.polymarket.test:{}\r\n",
            exact_peer.port()
        )));
        assert!(
            timeout(Duration::from_millis(100), decoy.accept())
                .await
                .is_err()
        );
    }

    #[test]
    fn fixed_peer_response_validation_rejects_missing_or_different_peer() {
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
}
