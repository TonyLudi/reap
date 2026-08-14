//! Isolated credential-wide Polymarket emergency cancellation plane.
//!
//! Its complete HTTP allowlist is public time, unfiltered paginated open
//! orders, and `DELETE /cancel-all`. It has no place, exact cancel, signing,
//! market selection, generic method/path, retry, transfer, or redemption
//! operation. The caller coordinates repeated cleanup attempts and deadlines.

#![forbid(unsafe_code)]

use std::{collections::BTreeSet, fmt, net::SocketAddr, time::Duration};

use reap_polymarket_auth::{FixedCancelAllRequestSink, L2Credentials, L2HeaderSink, L2Timestamp};
use reap_polymarket_egress_binding::{PmFixedTlsPeerSelection, PmLocalEgressSelection};
use reap_polymarket_wire::{
    MAX_PM_LIVE_BODY_BYTES, PmLiveOpenOrderPage, parse_live_cancel_result,
    parse_live_open_order_page, parse_server_time,
};
use reqwest::{
    Client, Request, StatusCode, Url,
    header::{ACCEPT, CONNECTION, CONTENT_TYPE, HeaderName, HeaderValue},
    redirect::Policy,
};
use thiserror::Error;
use zeroize::Zeroizing;

const PM_CLOB_PRODUCTION_ORIGIN: &str = "https://clob.polymarket.com";
const PM_CLOB_PRODUCTION_DNS_NAME: &str = "clob.polymarket.com";
const FIRST_PAGE_CURSOR: &str = "MA==";
const MAX_OPEN_ORDER_PAGES: usize = 1_024;
const POLY_ADDRESS: HeaderName = HeaderName::from_static("poly_address");
const POLY_SIGNATURE: HeaderName = HeaderName::from_static("poly_signature");
const POLY_TIMESTAMP: HeaderName = HeaderName::from_static("poly_timestamp");
const POLY_API_KEY: HeaderName = HeaderName::from_static("poly_api_key");
const POLY_PASSPHRASE: HeaderName = HeaderName::from_static("poly_passphrase");

pub const POLYMARKET_EMERGENCY_HTTP_ALLOWLIST: &[(&str, &str)] = &[
    ("GET", "/time"),
    ("GET", "/data/orders"),
    ("DELETE", "/cancel-all"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmEmergencyAdapterError {
    #[error("invalid Polymarket emergency production configuration")]
    InvalidConfiguration,
    #[error("Polymarket emergency HTTP client construction failed")]
    TransportBuild,
    #[error("Polymarket emergency authentication failed")]
    Authentication,
    #[error("Polymarket emergency request construction failed")]
    RequestBuild,
    #[error("Polymarket emergency request timed out")]
    RequestTimeout,
    #[error("Polymarket emergency transport failed")]
    TransportFailure,
    #[error("Polymarket emergency connected peer did not match the fixed peer")]
    ConnectedPeerMismatch,
    #[error("Polymarket emergency response redirected")]
    Redirect,
    #[error("Polymarket emergency response exceeded its byte bound")]
    ResponseTooLarge,
    #[error("Polymarket emergency response body failed")]
    ResponseBodyFailure,
    #[error("Polymarket emergency endpoint rejected the request with HTTP {0}")]
    Rejected(u16),
    #[error("Polymarket emergency response was malformed or inconsistent")]
    MalformedResponse,
    #[error("Polymarket emergency open-order cut exceeded its page/row bound")]
    CutBoundExceeded,
    #[error("Polymarket emergency open-order cut repeated a cursor")]
    RepeatedCursor,
    #[error("Polymarket emergency open-order response owner mismatched the credential")]
    CredentialOwnerMismatch,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PmEmergencyProductionConfig {
    connect_timeout: Duration,
    request_timeout: Duration,
    fixed_peer: PmFixedTlsPeerSelection,
    local_egress: PmLocalEgressSelection,
}

impl PmEmergencyProductionConfig {
    pub fn production_on_fixed_tls_peer_and_selected_local_egress(
        connect_timeout: Duration,
        request_timeout: Duration,
        fixed_peer: PmFixedTlsPeerSelection,
        local_egress: PmLocalEgressSelection,
    ) -> Result<Self, PmEmergencyAdapterError> {
        if connect_timeout.is_zero()
            || request_timeout.is_zero()
            || connect_timeout > request_timeout
            || request_timeout > Duration::from_secs(60)
            || fixed_peer.require_production().is_err()
            || fixed_peer.dns_name() != PM_CLOB_PRODUCTION_DNS_NAME
            || fixed_peer.peer_addr().port() != 443
            || local_egress.require_production().is_err()
            || fixed_peer
                .require_same_address_family(&local_egress)
                .is_err()
        {
            return Err(PmEmergencyAdapterError::InvalidConfiguration);
        }
        Ok(Self {
            connect_timeout,
            request_timeout,
            fixed_peer,
            local_egress,
        })
    }
}

impl fmt::Debug for PmEmergencyProductionConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmEmergencyProductionConfig(<fixed production peer/egress>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmEmergencyCancelAllOutcome {
    canceled_orders: usize,
    not_canceled_orders: usize,
}

impl PmEmergencyCancelAllOutcome {
    #[must_use]
    pub const fn canceled_orders(self) -> usize {
        self.canceled_orders
    }

    #[must_use]
    pub const fn not_canceled_orders(self) -> usize {
        self.not_canceled_orders
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmEmergencyOpenOrdersCut {
    pages: usize,
    open_orders: usize,
}

impl PmEmergencyOpenOrdersCut {
    #[must_use]
    pub const fn pages(self) -> usize {
        self.pages
    }

    #[must_use]
    pub const fn open_orders(self) -> usize {
        self.open_orders
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.open_orders == 0
    }
}

/// Move-only emergency role owning one L2 bundle and only the allowlisted
/// operations documented at the crate boundary.
pub struct PmEmergencyAccountStopRole {
    client: Client,
    origin: Url,
    expected_peer: SocketAddr,
    credentials: L2Credentials,
}

impl PmEmergencyAccountStopRole {
    pub fn new(
        config: PmEmergencyProductionConfig,
        credentials: L2Credentials,
    ) -> Result<Self, PmEmergencyAdapterError> {
        let origin = Url::parse(PM_CLOB_PRODUCTION_ORIGIN)
            .map_err(|_| PmEmergencyAdapterError::InvalidConfiguration)?;
        let mut builder = Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .retry(reqwest::retry::never())
            .redirect(Policy::none())
            .no_proxy()
            .https_only(true)
            .pool_max_idle_per_host(0)
            .resolve(config.fixed_peer.dns_name(), config.fixed_peer.peer_addr());
        #[cfg(target_os = "linux")]
        {
            builder = builder
                .interface(config.local_egress.interface_name())
                .local_address(config.local_egress.local_source_ip());
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (builder, config.local_egress);
            return Err(PmEmergencyAdapterError::InvalidConfiguration);
        }
        Ok(Self {
            client: builder
                .build()
                .map_err(|_| PmEmergencyAdapterError::TransportBuild)?,
            origin,
            expected_peer: config.fixed_peer.peer_addr(),
            credentials,
        })
    }

    /// Dispatch one credential-wide `DELETE /cancel-all` after sampling the
    /// venue clock. There is deliberately no retry in this method.
    pub async fn cancel_all(&self) -> Result<PmEmergencyCancelAllOutcome, PmEmergencyAdapterError> {
        let timestamp = self.server_timestamp().await?;
        let authenticated = self
            .credentials
            .authenticate_cancel_all(timestamp)
            .map_err(|_| PmEmergencyAdapterError::Authentication)?;
        let mut sink = CancelAllRequestBuilder {
            client: &self.client,
            origin: &self.origin,
        };
        let request = authenticated
            .dispatch(&mut sink)
            .map_err(|_| PmEmergencyAdapterError::RequestBuild)?;
        let (status, body) = self.execute(request).await?;
        if status != StatusCode::OK {
            return Err(PmEmergencyAdapterError::Rejected(status.as_u16()));
        }
        let result = parse_live_cancel_result(&body)
            .map_err(|_| PmEmergencyAdapterError::MalformedResponse)?;
        Ok(PmEmergencyCancelAllOutcome {
            canceled_orders: result.canceled().len(),
            not_canceled_orders: result.not_canceled().len(),
        })
    }

    /// Fetch a complete, unfiltered, credential-owned paginated order cut.
    /// Only a terminal cursor can produce the returned zero proof.
    pub async fn complete_open_orders(
        &self,
    ) -> Result<PmEmergencyOpenOrdersCut, PmEmergencyAdapterError> {
        let mut cursor = FIRST_PAGE_CURSOR.to_owned();
        let mut seen = BTreeSet::new();
        let mut pages = 0_usize;
        let mut rows = 0_usize;
        loop {
            if pages >= MAX_OPEN_ORDER_PAGES || !seen.insert(cursor.clone()) {
                return Err(if pages >= MAX_OPEN_ORDER_PAGES {
                    PmEmergencyAdapterError::CutBoundExceeded
                } else {
                    PmEmergencyAdapterError::RepeatedCursor
                });
            }
            let page = self.open_orders_page(&cursor).await?;
            pages += 1;
            rows = rows
                .checked_add(page.orders().len())
                .ok_or(PmEmergencyAdapterError::CutBoundExceeded)?;
            if rows > MAX_OPEN_ORDER_PAGES.saturating_mul(128) {
                return Err(PmEmergencyAdapterError::CutBoundExceeded);
            }
            if page
                .orders()
                .iter()
                .any(|order| !self.credentials.matches_credential_owner(order.owner()))
            {
                return Err(PmEmergencyAdapterError::CredentialOwnerMismatch);
            }
            if page.terminal() {
                return Ok(PmEmergencyOpenOrdersCut {
                    pages,
                    open_orders: rows,
                });
            }
            cursor = page
                .next_cursor()
                .ok_or(PmEmergencyAdapterError::MalformedResponse)?
                .as_str()
                .to_owned();
        }
    }

    async fn open_orders_page(
        &self,
        cursor: &str,
    ) -> Result<PmLiveOpenOrderPage, PmEmergencyAdapterError> {
        let timestamp = self.server_timestamp().await?;
        let headers = self
            .credentials
            .authenticate_open_orders(timestamp)
            .map_err(|_| PmEmergencyAdapterError::Authentication)?;
        let mut url = self.origin.clone();
        url.set_path("/data/orders");
        url.query_pairs_mut().append_pair("next_cursor", cursor);
        let request = self
            .client
            .get(url)
            .header(ACCEPT, HeaderValue::from_static("application/json"))
            .header(CONNECTION, HeaderValue::from_static("close"));
        let mut sink = HeaderRequestBuilder(Some(request));
        headers
            .apply_to(&mut sink)
            .map_err(|_| PmEmergencyAdapterError::RequestBuild)?;
        let request = sink
            .0
            .take()
            .ok_or(PmEmergencyAdapterError::RequestBuild)?
            .build()
            .map_err(|_| PmEmergencyAdapterError::RequestBuild)?;
        let (status, body) = self.execute(request).await?;
        if status != StatusCode::OK {
            return Err(PmEmergencyAdapterError::Rejected(status.as_u16()));
        }
        parse_live_open_order_page(&body).map_err(|_| PmEmergencyAdapterError::MalformedResponse)
    }

    async fn server_timestamp(&self) -> Result<L2Timestamp, PmEmergencyAdapterError> {
        let mut url = self.origin.clone();
        url.set_path("/time");
        let request = self
            .client
            .get(url)
            .header(ACCEPT, HeaderValue::from_static("application/json"))
            .header(CONNECTION, HeaderValue::from_static("close"))
            .build()
            .map_err(|_| PmEmergencyAdapterError::RequestBuild)?;
        let (status, body) = self.execute(request).await?;
        if status != StatusCode::OK {
            return Err(PmEmergencyAdapterError::Rejected(status.as_u16()));
        }
        let seconds =
            parse_server_time(&body).map_err(|_| PmEmergencyAdapterError::MalformedResponse)?;
        L2Timestamp::from_unix_seconds(seconds)
            .map_err(|_| PmEmergencyAdapterError::MalformedResponse)
    }

    async fn execute(
        &self,
        request: Request,
    ) -> Result<(StatusCode, Zeroizing<Vec<u8>>), PmEmergencyAdapterError> {
        let mut response = self.client.execute(request).await.map_err(|error| {
            if error.is_timeout() {
                PmEmergencyAdapterError::RequestTimeout
            } else {
                PmEmergencyAdapterError::TransportFailure
            }
        })?;
        let status = response.status();
        if response.remote_addr() != Some(self.expected_peer) {
            return Err(PmEmergencyAdapterError::ConnectedPeerMismatch);
        }
        if status.is_redirection() {
            return Err(PmEmergencyAdapterError::Redirect);
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_PM_LIVE_BODY_BYTES as u64)
        {
            return Err(PmEmergencyAdapterError::ResponseTooLarge);
        }
        let mut body = Zeroizing::new(Vec::new());
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| PmEmergencyAdapterError::ResponseBodyFailure)?
        {
            if body.len().saturating_add(chunk.len()) > MAX_PM_LIVE_BODY_BYTES {
                return Err(PmEmergencyAdapterError::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok((status, body))
    }
}

impl fmt::Debug for PmEmergencyAccountStopRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmEmergencyAccountStopRole([REDACTED; CANCEL-ALL ONLY])")
    }
}

struct CancelAllRequestBuilder<'a> {
    client: &'a Client,
    origin: &'a Url,
}

impl FixedCancelAllRequestSink for CancelAllRequestBuilder<'_> {
    type Output = Request;
    type Error = ();

    fn send_cancel_all(
        &mut self,
        poly_address: &str,
        poly_signature: &str,
        poly_timestamp: &str,
        poly_api_key: &str,
        poly_passphrase: &str,
    ) -> Result<Self::Output, Self::Error> {
        let mut url = self.origin.clone();
        url.set_path("/cancel-all");
        self.client
            .delete(url)
            .header(ACCEPT, HeaderValue::from_static("application/json"))
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .header(CONNECTION, HeaderValue::from_static("close"))
            .header(POLY_ADDRESS, sensitive(poly_address)?)
            .header(POLY_SIGNATURE, sensitive(poly_signature)?)
            .header(POLY_TIMESTAMP, sensitive(poly_timestamp)?)
            .header(POLY_API_KEY, sensitive(poly_api_key)?)
            .header(POLY_PASSPHRASE, sensitive(poly_passphrase)?)
            .build()
            .map_err(|_| ())
    }
}

struct HeaderRequestBuilder(Option<reqwest::RequestBuilder>);

impl L2HeaderSink for HeaderRequestBuilder {
    type Error = ();

    fn set_polymarket_l2_headers(
        &mut self,
        poly_address: &str,
        poly_signature: &str,
        poly_timestamp: &str,
        poly_api_key: &str,
        poly_passphrase: &str,
    ) -> Result<(), Self::Error> {
        let request = self.0.take().ok_or(())?;
        self.0 = Some(
            request
                .header(POLY_ADDRESS, sensitive(poly_address)?)
                .header(POLY_SIGNATURE, sensitive(poly_signature)?)
                .header(POLY_TIMESTAMP, sensitive(poly_timestamp)?)
                .header(POLY_API_KEY, sensitive(poly_api_key)?)
                .header(POLY_PASSPHRASE, sensitive(poly_passphrase)?),
        );
        Ok(())
    }
}

fn sensitive(value: &str) -> Result<HeaderValue, ()> {
    let mut value = HeaderValue::from_str(value).map_err(|_| ())?;
    value.set_sensitive(true);
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authority_is_exactly_three_read_cleanup_routes() {
        assert_eq!(
            POLYMARKET_EMERGENCY_HTTP_ALLOWLIST,
            &[
                ("GET", "/time"),
                ("GET", "/data/orders"),
                ("DELETE", "/cancel-all")
            ]
        );
        let source = include_str!("lib.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production source");
        for forbidden in [
            ".post(url)",
            "set_path(\"/order\")",
            "set_path(\"/orders\")",
            "set_path(\"/withdraw\")",
            "set_path(\"/transfer\")",
            "set_path(\"/redeem\")",
        ] {
            assert!(
                !source.contains(forbidden),
                "forbidden authority: {forbidden}"
            );
        }
    }
}
