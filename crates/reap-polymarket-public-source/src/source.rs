use std::{collections::BTreeSet, time::Duration};

use reqwest::{
    Client, StatusCode, Url,
    header::{ACCEPT, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_TYPE, HeaderMap, HeaderName},
    redirect::Policy,
};

use crate::{
    PmConfiguredTokenPosition, PmDataApiPositionConfig, PmDataApiPositionScope,
    PmMonitoredPositionObservation, PmPublicPositionError,
    config::OriginMode,
    position::{MAX_POSITION_PAGE_ROWS, parse_position_page},
};

pub const MAX_POSITION_PAGE_BODY_BYTES: usize = 1_048_576;
const POSITION_PAGE_LIMIT: u16 = 500;
const MAX_POSITION_OFFSET: u16 = 10_000;

struct PmDataApiPositionTransport {
    client: Client,
    origin: Url,
    scope: PmDataApiPositionScope,
}

impl PmDataApiPositionTransport {
    fn new(config: &PmDataApiPositionConfig) -> Result<Self, PmPublicPositionError> {
        let mut builder = Client::builder()
            .connect_timeout(config.connect_timeout())
            .timeout(config.request_timeout())
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy();
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
}

impl PmDataApiCurrentPositionSource {
    pub fn production(
        scope: PmDataApiPositionScope,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, PmPublicPositionError> {
        Self::new(PmDataApiPositionConfig::production(
            scope,
            connect_timeout,
            request_timeout,
        )?)
    }

    fn new(config: PmDataApiPositionConfig) -> Result<Self, PmPublicPositionError> {
        Ok(Self {
            transport: PmDataApiPositionTransport::new(&config)?,
        })
    }

    #[cfg(test)]
    fn numeric_loopback_evidence(
        origin: &str,
        scope: PmDataApiPositionScope,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, PmPublicPositionError> {
        Self::new(PmDataApiPositionConfig::numeric_loopback_evidence(
            origin,
            scope,
            connect_timeout,
            request_timeout,
        )?)
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }

    pub async fn observe_configured_token(
        &self,
    ) -> Result<PmMonitoredPositionObservation, PmPublicPositionError> {
        let scope = self.transport.scope;
        let mut seen_assets = BTreeSet::new();
        let mut configured_token = None;
        let mut offset = 0_u16;
        let mut pages_observed = 0_u8;
        let mut rows_observed = 0_u16;

        loop {
            let body = self.transport.fetch_page(offset).await?;
            let rows = parse_position_page(&body, scope)?;
            let page_rows = rows.len();
            pages_observed = pages_observed
                .checked_add(1)
                .expect("fixed 21-page position bound");
            rows_observed = rows_observed
                .checked_add(u16::try_from(page_rows).expect("500-row page bound"))
                .expect("fixed position aggregate bound");

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

        Ok(PmMonitoredPositionObservation::new(
            scope,
            pages_observed,
            rows_observed,
            configured_token.map_or(PmConfiguredTokenPosition::Absent, |position| {
                PmConfiguredTokenPosition::Present(Box::new(position))
            }),
        ))
    }
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
    use std::collections::VecDeque;

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
    ) -> (String, mpsc::UnboundedReceiver<String>, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (requests_tx, requests_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut responses = VecDeque::from(responses);
            while let Some(response) = responses.pop_front() {
                let (mut stream, _) = listener.accept().await.unwrap();
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
                let _ = requests_tx.send(String::from_utf8(raw).unwrap());

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
        (format!("http://{address}"), requests_rx, task)
    }

    fn source(origin: &str, token: u64) -> PmDataApiCurrentPositionSource {
        PmDataApiCurrentPositionSource::numeric_loopback_evidence(
            origin,
            scope(token),
            Duration::from_secs(2),
            Duration::from_secs(5),
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
            last = request;
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
