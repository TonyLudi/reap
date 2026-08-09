use std::time::Duration;

#[cfg(any(test, feature = "loopback-evidence"))]
use std::net::IpAddr;

use reqwest::{Client, StatusCode, Url, redirect::Policy};
use thiserror::Error;
#[cfg(any(test, feature = "loopback-evidence"))]
use url::Host;

use crate::{
    PM_POLYGON_CHAIN_ID, PM_POLYGON_CONDITIONAL_TOKENS_ADDRESS, PM_POLYGON_PUSD_PROXY_ADDRESS,
    PM_POLYGON_RPC_ORIGIN, PmPolygonAuthorizationScope, PmPolygonFinalizedAuthorizationCut,
    PmPolygonSystemClockObservation,
    rpc::{
        BLOCK_REREAD_REQUEST_ID, allowance_request, approval_request, block_reread_request,
        chain_id_request, decode_allowance, decode_approval, decode_chain_id,
        decode_finalized_block, finalized_block_request,
    },
};
use reap_pm_core::EvmAddress;

const PRODUCTION_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const PRODUCTION_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(any(test, feature = "loopback-evidence"))]
const MAX_LOOPBACK_REQUEST_TIMEOUT: Duration = PRODUCTION_REQUEST_TIMEOUT;
#[cfg(any(test, feature = "loopback-evidence"))]
const MIN_LOOPBACK_REQUEST_TIMEOUT: Duration = Duration::from_millis(1);
const MAX_JSON_RPC_RESPONSE_BYTES: usize = 16 * 1024;
const MAX_FINALIZED_BLOCK_AGE_SECONDS: u64 = 30;
const MAX_FINALIZED_BLOCK_FUTURE_SECONDS: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPolygonChainSourceError {
    #[error("PM-T2 chain authorization scope must use Polygon chain 137")]
    WrongAccountChain,
    #[error("PM-T2 chain authorization scope requires distinct signer and funder identities")]
    SignerFunderNotDistinct,
    #[error("failed to build the closed Polygon JSON-RPC transport")]
    TransportBuild,
    #[error("numeric-loopback evidence origin is outside the closed local origin policy")]
    InvalidLoopbackOrigin,
    #[error("numeric-loopback evidence timeout is outside its closed bound")]
    InvalidLoopbackTimeout,
    #[error("system clock precedes the Unix epoch")]
    SystemClockBeforeUnixEpoch,
    #[error("failed to encode a fixed JSON-RPC request")]
    RequestEncoding,
    #[error("Polygon JSON-RPC request timed out")]
    RequestTimeout,
    #[error("Polygon JSON-RPC request failed")]
    RequestFailed,
    #[error("Polygon JSON-RPC response body could not be read")]
    ResponseBodyRead,
    #[error("Polygon JSON-RPC endpoint attempted an HTTP redirect ({status})")]
    Redirect { status: u16 },
    #[error("Polygon JSON-RPC endpoint returned unexpected HTTP status {status}")]
    UnexpectedStatus { status: u16 },
    #[error("Polygon JSON-RPC response exceeds {limit} bytes")]
    ResponseBodyTooLarge { limit: usize },
    #[error("Polygon JSON-RPC response is malformed or outside the closed schema")]
    MalformedJsonRpc,
    #[error("Polygon JSON-RPC response does not use version 2.0")]
    WrongJsonRpcVersion,
    #[error("Polygon JSON-RPC response ID {actual} does not match request ID {expected}")]
    WrongResponseId { expected: u64, actual: u64 },
    #[error("Polygon JSON-RPC response contains neither or both result and error")]
    InvalidJsonRpcOutcome,
    #[error("Polygon JSON-RPC error object is outside its closed bound")]
    MalformedJsonRpcError,
    #[error("Polygon JSON-RPC endpoint returned error code {code}")]
    RemoteRpcError { code: i64 },
    #[error("Polygon JSON-RPC quantity is not canonical lowercase hexadecimal")]
    NonCanonicalQuantity,
    #[error("Polygon endpoint is not connected to chain 137")]
    WrongRpcChain,
    #[error("finalized Polygon block hash is not a canonical 32-byte word")]
    InvalidBlockHash,
    #[error("finalized Polygon block hash must not be zero")]
    ZeroBlockHash,
    #[error("pUSD allowance result is not a canonical 32-byte word")]
    InvalidAllowanceWord,
    #[error("Conditional Tokens approval result is not a canonical 32-byte word")]
    InvalidApprovalWord,
    #[error("Conditional Tokens approval is not the canonical ABI boolean 0 or 1")]
    NonCanonicalApprovalBoolean,
    #[error("exact finalized block changed across the authorization observation")]
    FinalizedBlockChanged,
    #[error("finalized Polygon block is more than 30 seconds old")]
    StaleFinalizedBlock,
    #[error("finalized Polygon block is more than 5 seconds in the future")]
    FutureFinalizedBlock,
}

#[derive(Clone, Copy)]
enum ClockSource {
    System,
    #[cfg(any(test, feature = "loopback-evidence"))]
    Fixed(PmPolygonSystemClockObservation),
}

impl ClockSource {
    fn observe(self) -> Result<PmPolygonSystemClockObservation, PmPolygonChainSourceError> {
        match self {
            Self::System => PmPolygonSystemClockObservation::capture(),
            #[cfg(any(test, feature = "loopback-evidence"))]
            Self::Fixed(observation) => Ok(observation),
        }
    }
}

/// Closed read-only Polygon JSON-RPC source for one exact PM-T2 approval cut.
#[derive(Clone)]
pub struct PmPolygonAuthorizationSource {
    transport: PmPolygonRpcTransport,
    clock: ClockSource,
}

impl PmPolygonAuthorizationSource {
    /// Builds the only production source. Its origin, redirect/retry/proxy
    /// policy, request methods, contracts, block selectors, and timeouts are
    /// fixed by this crate.
    pub fn production() -> Result<Self, PmPolygonChainSourceError> {
        let origin = Url::parse(PM_POLYGON_RPC_ORIGIN).expect("frozen Polygon RPC origin");
        Ok(Self {
            transport: PmPolygonRpcTransport::build(
                origin,
                PRODUCTION_CONNECT_TIMEOUT,
                PRODUCTION_REQUEST_TIMEOUT,
                true,
            )?,
            clock: ClockSource::System,
        })
    }

    /// Builds evidence transport only for a canonical HTTP URL whose host is
    /// a numeric loopback address and whose port is explicit. This seam is
    /// unavailable unless tests or the dedicated evidence feature enable it.
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub fn loopback_evidence(
        origin: &str,
        clock: PmPolygonSystemClockObservation,
        request_timeout: Duration,
    ) -> Result<Self, PmPolygonChainSourceError> {
        if !(MIN_LOOPBACK_REQUEST_TIMEOUT..=MAX_LOOPBACK_REQUEST_TIMEOUT).contains(&request_timeout)
        {
            return Err(PmPolygonChainSourceError::InvalidLoopbackTimeout);
        }
        let origin = parse_numeric_loopback_origin(origin)?;
        Ok(Self {
            transport: PmPolygonRpcTransport::build(
                origin,
                request_timeout.min(PRODUCTION_CONNECT_TIMEOUT),
                request_timeout,
                false,
            )?,
            clock: ClockSource::Fixed(clock),
        })
    }

    /// Reads the exact five-request cut in a fixed order and returns only
    /// after every response, block binding, and freshness check succeeds.
    pub async fn finalized_authorization_cut(
        &self,
        scope: PmPolygonAuthorizationScope,
    ) -> Result<PmPolygonFinalizedAuthorizationCut, PmPolygonChainSourceError> {
        let chain_id = decode_chain_id(&self.transport.post(chain_id_request()?).await?)?;
        if chain_id != PM_POLYGON_CHAIN_ID {
            return Err(PmPolygonChainSourceError::WrongRpcChain);
        }

        let finalized = decode_finalized_block(
            &self.transport.post(finalized_block_request()?).await?,
            crate::rpc::FINALIZED_BLOCK_REQUEST_ID,
        )?;
        let owner = scope.owner();
        let spender = scope.spender().address();
        let pusd = frozen_address(PM_POLYGON_PUSD_PROXY_ADDRESS);
        let conditional_tokens = frozen_address(PM_POLYGON_CONDITIONAL_TOKENS_ADDRESS);

        let pusd_allowance = decode_allowance(
            &self
                .transport
                .post(allowance_request(
                    pusd,
                    owner,
                    spender,
                    &finalized.canonical_number,
                )?)
                .await?,
        )?;
        let conditional_tokens_approval = decode_approval(
            &self
                .transport
                .post(approval_request(
                    conditional_tokens,
                    owner,
                    spender,
                    &finalized.canonical_number,
                )?)
                .await?,
        )?;
        let reread = decode_finalized_block(
            &self
                .transport
                .post(block_reread_request(&finalized.canonical_number)?)
                .await?,
            BLOCK_REREAD_REQUEST_ID,
        )?;
        if reread.identity != finalized.identity {
            return Err(PmPolygonChainSourceError::FinalizedBlockChanged);
        }

        let observed_clock = self.clock.observe()?;
        validate_freshness(finalized.identity.timestamp, observed_clock.unix_seconds())?;

        Ok(PmPolygonFinalizedAuthorizationCut {
            scope,
            block: finalized.identity,
            pusd_allowance,
            conditional_tokens_approval,
            observed_clock,
        })
    }
}

fn frozen_address(encoded: &str) -> EvmAddress {
    EvmAddress::parse(encoded).expect("frozen Polygon contract address")
}

fn validate_freshness(
    block_timestamp: u64,
    observed_timestamp: u64,
) -> Result<(), PmPolygonChainSourceError> {
    if block_timestamp > observed_timestamp {
        if block_timestamp - observed_timestamp > MAX_FINALIZED_BLOCK_FUTURE_SECONDS {
            return Err(PmPolygonChainSourceError::FutureFinalizedBlock);
        }
    } else if observed_timestamp - block_timestamp > MAX_FINALIZED_BLOCK_AGE_SECONDS {
        return Err(PmPolygonChainSourceError::StaleFinalizedBlock);
    }
    Ok(())
}

#[cfg(any(test, feature = "loopback-evidence"))]
fn parse_numeric_loopback_origin(origin: &str) -> Result<Url, PmPolygonChainSourceError> {
    let parsed =
        Url::parse(origin).map_err(|_| PmPolygonChainSourceError::InvalidLoopbackOrigin)?;
    if parsed.scheme() != "http"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_none()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(PmPolygonChainSourceError::InvalidLoopbackOrigin);
    }
    let host = parsed
        .host()
        .ok_or(PmPolygonChainSourceError::InvalidLoopbackOrigin)?;
    let ip = match host {
        Host::Ipv4(address) => IpAddr::V4(address),
        Host::Ipv6(address) => IpAddr::V6(address),
        Host::Domain(_) => return Err(PmPolygonChainSourceError::InvalidLoopbackOrigin),
    };
    if !ip.is_loopback() {
        return Err(PmPolygonChainSourceError::InvalidLoopbackOrigin);
    }
    let canonical = match ip {
        IpAddr::V4(address) => {
            format!("http://{address}:{}/", parsed.port().expect("checked port"))
        }
        IpAddr::V6(address) => {
            format!(
                "http://[{address}]:{}/",
                parsed.port().expect("checked port")
            )
        }
    };
    if origin != canonical {
        return Err(PmPolygonChainSourceError::InvalidLoopbackOrigin);
    }
    Ok(parsed)
}

#[derive(Clone)]
struct PmPolygonRpcTransport {
    client: Client,
    origin: Url,
}

impl PmPolygonRpcTransport {
    fn build(
        origin: Url,
        connect_timeout: Duration,
        request_timeout: Duration,
        production: bool,
    ) -> Result<Self, PmPolygonChainSourceError> {
        let mut builder = Client::builder()
            .connect_timeout(connect_timeout)
            .timeout(request_timeout)
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy();
        if production {
            builder = builder.https_only(true);
        }
        let client = builder
            .build()
            .map_err(|_| PmPolygonChainSourceError::TransportBuild)?;
        Ok(Self { client, origin })
    }

    async fn post(&self, body: Vec<u8>) -> Result<Vec<u8>, PmPolygonChainSourceError> {
        let mut response = self
            .client
            .post(self.origin.clone())
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(map_request_error)?;
        let status = response.status();
        if status.is_redirection() {
            return Err(PmPolygonChainSourceError::Redirect {
                status: status.as_u16(),
            });
        }
        if status != StatusCode::OK {
            return Err(PmPolygonChainSourceError::UnexpectedStatus {
                status: status.as_u16(),
            });
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_JSON_RPC_RESPONSE_BYTES as u64)
        {
            return Err(PmPolygonChainSourceError::ResponseBodyTooLarge {
                limit: MAX_JSON_RPC_RESPONSE_BYTES,
            });
        }

        let capacity = response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or(0)
            .min(MAX_JSON_RPC_RESPONSE_BYTES);
        let mut body = Vec::with_capacity(capacity);
        while let Some(chunk) = response.chunk().await.map_err(map_body_error)? {
            let next_length = body.len().checked_add(chunk.len()).ok_or(
                PmPolygonChainSourceError::ResponseBodyTooLarge {
                    limit: MAX_JSON_RPC_RESPONSE_BYTES,
                },
            )?;
            if next_length > MAX_JSON_RPC_RESPONSE_BYTES {
                return Err(PmPolygonChainSourceError::ResponseBodyTooLarge {
                    limit: MAX_JSON_RPC_RESPONSE_BYTES,
                });
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

fn map_request_error(error: reqwest::Error) -> PmPolygonChainSourceError {
    if error.is_timeout() {
        PmPolygonChainSourceError::RequestTimeout
    } else {
        PmPolygonChainSourceError::RequestFailed
    }
}

fn map_body_error(error: reqwest::Error) -> PmPolygonChainSourceError {
    if error.is_timeout() {
        PmPolygonChainSourceError::RequestTimeout
    } else {
        PmPolygonChainSourceError::ResponseBodyRead
    }
}

#[cfg(test)]
mod tests;
