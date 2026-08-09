use reap_pm_core::{PmConditionId, PmTokenId};
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
}

impl PmHttpTransport {
    pub(crate) fn new(config: &PmPublicHttpConfig) -> Result<Self, PmLiveAdapterError> {
        Self::build(
            config.origin().clone(),
            config.connect_timeout(),
            config.request_timeout(),
            config.mode(),
        )
    }

    pub(crate) fn geoblock(config: &PmGeoblockHttpConfig) -> Result<Self, PmLiveAdapterError> {
        Self::build(
            config.origin().clone(),
            config.connect_timeout(),
            config.request_timeout(),
            config.mode(),
        )
    }

    pub(crate) fn status(config: &PmStatusHttpConfig) -> Result<Self, PmLiveAdapterError> {
        Self::build(
            config.origin().clone(),
            config.connect_timeout(),
            config.request_timeout(),
            config.mode(),
        )
    }

    fn build(
        origin: Url,
        connect_timeout: std::time::Duration,
        request_timeout: std::time::Duration,
        mode: OriginMode,
    ) -> Result<Self, PmLiveAdapterError> {
        let mut builder = Client::builder()
            .connect_timeout(connect_timeout)
            .timeout(request_timeout)
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy();
        if mode == OriginMode::Production {
            builder = builder.https_only(true);
        }
        let client = builder
            .build()
            .map_err(|_| PmLiveAdapterError::TransportBuild)?;
        Ok(Self { client, origin })
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
