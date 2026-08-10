use std::time::Duration;

#[cfg(any(test, feature = "loopback-evidence"))]
use std::net::IpAddr;

use reap_polymarket_egress_binding::{
    PmFixedTlsPeerSelection, PmFixedTlsPeerSelectionError, PmLocalEgressSelection,
    PmLocalEgressSelectionError,
};
use reqwest::{Client, StatusCode, Url, redirect::Policy};
use sha2::{Digest, Sha256};
use thiserror::Error;
#[cfg(any(test, feature = "loopback-evidence"))]
use url::Host;

use crate::{
    PM_POLYGON_CHAIN_ID, PM_POLYGON_CONDITIONAL_TOKENS_ADDRESS, PM_POLYGON_PUSD_PROXY_ADDRESS,
    PM_POLYGON_RPC_ORIGIN, PmPolygonAuthorizationScope, PmPolygonFinalizedAuthorizationCommitment,
    PmPolygonFinalizedAuthorizationCut, PmPolygonFinalizedBlock, PmPolygonSystemClockObservation,
    rpc::{
        BLOCK_REREAD_REQUEST_ID, CHAIN_ID_REQUEST_ID, CONDITIONAL_TOKENS_APPROVAL_REQUEST_ID,
        FINALIZED_BLOCK_REQUEST_ID, PUSD_ALLOWANCE_REQUEST_ID, allowance_request, approval_request,
        block_reread_request, chain_id_request, decode_allowance, decode_approval, decode_chain_id,
        decode_finalized_block, finalized_block_request,
    },
};
use reap_pm_core::{EvmAddress, PmErc1155OperatorApproval, PmSpenderDomain, U256};

const PRODUCTION_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const PRODUCTION_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(any(test, feature = "loopback-evidence"))]
const MAX_LOOPBACK_REQUEST_TIMEOUT: Duration = PRODUCTION_REQUEST_TIMEOUT;
#[cfg(any(test, feature = "loopback-evidence"))]
const MIN_LOOPBACK_REQUEST_TIMEOUT: Duration = Duration::from_millis(1);
const MAX_JSON_RPC_RESPONSE_BYTES: usize = 16 * 1024;
const MAX_FINALIZED_BLOCK_AGE_SECONDS: u64 = 30;
const MAX_FINALIZED_BLOCK_FUTURE_SECONDS: u64 = 5;
const AUTHORIZATION_CUT_COMMITMENT_DOMAIN: &[u8] =
    b"reap.polymarket.chain-source.finalized-authorization-cut.v1\0";
const AUTHORIZATION_CUT_REQUEST_COUNT: u8 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPolygonChainSourceError {
    #[error("PM-T2 chain authorization scope must use Polygon chain 137")]
    WrongAccountChain,
    #[error("PM-T2 chain authorization scope requires distinct signer and funder identities")]
    SignerFunderNotDistinct,
    #[error("failed to build the closed Polygon JSON-RPC transport")]
    TransportBuild,
    #[error("selected local egress is supported only on Linux")]
    SelectedLocalEgressUnsupported,
    #[error(transparent)]
    LocalEgressSelection(#[from] PmLocalEgressSelectionError),
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

/// Construction errors confined to the additive fixed-peer plus selected
/// local-egress source path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPolygonFixedPeerSourceError {
    #[error(transparent)]
    FixedTlsPeerSelection(#[from] PmFixedTlsPeerSelectionError),
    #[error(transparent)]
    LocalEgressSelection(#[from] PmLocalEgressSelectionError),
    #[error("fixed TLS peer DNS name does not match the closed Polygon RPC origin")]
    DnsNameMismatch,
    #[error(transparent)]
    Source(#[from] PmPolygonChainSourceError),
}

/// Errors specific to obtaining a production-origin proof around an otherwise
/// unchanged finalized Polygon authorization cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmProductionPolygonFinalizedAuthorizationError {
    #[error("production Polygon authorization evidence requires the fixed production origin")]
    OriginRequired,
    #[error(transparent)]
    Source(#[from] PmPolygonChainSourceError),
}

#[derive(Clone, Copy)]
enum ClockSource {
    System,
    #[cfg(any(test, feature = "loopback-evidence"))]
    Fixed(PmPolygonSystemClockObservation),
}

#[derive(Clone, Copy)]
enum SourceMode {
    Production,
    #[cfg(any(test, feature = "loopback-evidence"))]
    LoopbackEvidence,
}

/// Move-only proof that one finalized Polygon authorization cut came from
/// this source's fixed production-origin mode.
///
/// This remains read-only evidence. It is not a credential, signature,
/// dispatch permit, or production order-entry authority.
pub struct PmProductionPolygonFinalizedAuthorizationCut {
    cut: PmPolygonFinalizedAuthorizationCut,
}

impl PmProductionPolygonFinalizedAuthorizationCut {
    fn from_source(
        _production_origin: ProductionPolygonOrigin,
        cut: PmPolygonFinalizedAuthorizationCut,
    ) -> Self {
        Self { cut }
    }

    #[must_use]
    pub const fn scope(&self) -> PmPolygonAuthorizationScope {
        self.cut.scope()
    }

    #[must_use]
    pub const fn block(&self) -> PmPolygonFinalizedBlock {
        self.cut.block()
    }

    #[must_use]
    pub const fn pusd_allowance(&self) -> U256 {
        self.cut.pusd_allowance()
    }

    #[must_use]
    pub const fn conditional_tokens_approval(&self) -> PmErc1155OperatorApproval {
        self.cut.conditional_tokens_approval()
    }

    #[must_use]
    pub const fn observed_clock(&self) -> PmPolygonSystemClockObservation {
        self.cut.observed_clock()
    }

    #[must_use]
    pub const fn commitment(&self) -> PmPolygonFinalizedAuthorizationCommitment {
        self.cut.commitment()
    }
}

impl std::fmt::Debug for PmProductionPolygonFinalizedAuthorizationCut {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(
            "PmProductionPolygonFinalizedAuthorizationCut(<production-origin; read-only; sealed>)",
        )
    }
}

struct ProductionPolygonOrigin;

impl ProductionPolygonOrigin {
    fn verify(mode: SourceMode) -> Result<Self, PmProductionPolygonFinalizedAuthorizationError> {
        match mode {
            SourceMode::Production => Ok(Self),
            #[cfg(any(test, feature = "loopback-evidence"))]
            SourceMode::LoopbackEvidence => {
                Err(PmProductionPolygonFinalizedAuthorizationError::OriginRequired)
            }
        }
    }
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
    mode: SourceMode,
}

impl PmPolygonAuthorizationSource {
    /// Builds the default production source without an explicit local socket
    /// selection. Its origin, redirect/retry/proxy policy, request methods,
    /// contracts, block selectors, and timeouts are fixed by this crate.
    pub fn production() -> Result<Self, PmPolygonChainSourceError> {
        Self::production_with_local_egress(None)
    }

    /// Builds the fixed production source with an additional non-authoritative
    /// local interface and source-IP socket selection. The selection changes
    /// no origin, JSON-RPC method, contract, block selector, timeout, or
    /// production-origin proof and grants no order-entry authority.
    pub fn production_on_selected_local_egress(
        local_egress: &PmLocalEgressSelection,
    ) -> Result<Self, PmPolygonChainSourceError> {
        local_egress.require_production()?;
        Self::production_with_local_egress(Some(local_egress))
    }

    /// Builds the fixed production source on one exact reviewed TLS peer and
    /// one selected local interface/source IP. Both inputs are
    /// non-authoritative configuration and must independently be in their
    /// production modes.
    pub fn production_on_fixed_tls_peer_and_selected_local_egress(
        fixed_peer: &PmFixedTlsPeerSelection,
        local_egress: &PmLocalEgressSelection,
    ) -> Result<Self, PmPolygonFixedPeerSourceError> {
        fixed_peer.require_production()?;
        local_egress.require_production()?;
        fixed_peer.require_same_address_family(local_egress)?;
        let origin = Url::parse(PM_POLYGON_RPC_ORIGIN).expect("frozen Polygon RPC origin");
        if origin.host_str() != Some(fixed_peer.dns_name()) {
            return Err(PmPolygonFixedPeerSourceError::DnsNameMismatch);
        }
        Ok(Self::production_with_network_selections(
            Some(local_egress),
            Some(fixed_peer),
        )?)
    }

    fn production_with_local_egress(
        local_egress: Option<&PmLocalEgressSelection>,
    ) -> Result<Self, PmPolygonChainSourceError> {
        Self::production_with_network_selections(local_egress, None)
    }

    fn production_with_network_selections(
        local_egress: Option<&PmLocalEgressSelection>,
        fixed_peer: Option<&PmFixedTlsPeerSelection>,
    ) -> Result<Self, PmPolygonChainSourceError> {
        let origin = Url::parse(PM_POLYGON_RPC_ORIGIN).expect("frozen Polygon RPC origin");
        Ok(Self {
            transport: PmPolygonRpcTransport::build(
                origin,
                PRODUCTION_CONNECT_TIMEOUT,
                PRODUCTION_REQUEST_TIMEOUT,
                true,
                local_egress,
                fixed_peer,
            )?,
            clock: ClockSource::System,
            mode: SourceMode::Production,
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
        Self::loopback_evidence_with_local_egress(origin, clock, request_timeout, None)
    }

    /// Build the same fixed loopback evidence source with a selected local
    /// loopback interface/address. This remains unavailable in production
    /// builds unless the dedicated evidence feature is enabled.
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub fn loopback_evidence_on_selected_local_egress(
        origin: &str,
        clock: PmPolygonSystemClockObservation,
        request_timeout: Duration,
        local_egress: &PmLocalEgressSelection,
    ) -> Result<Self, PmPolygonChainSourceError> {
        local_egress.require_loopback_evidence()?;
        Self::loopback_evidence_with_local_egress(
            origin,
            clock,
            request_timeout,
            Some(local_egress),
        )
    }

    /// Build a hostname-preserving loopback evidence source on exactly one
    /// loopback peer and one selected loopback source address.
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub fn loopback_evidence_on_fixed_tls_peer_and_selected_local_egress(
        clock: PmPolygonSystemClockObservation,
        request_timeout: Duration,
        fixed_peer: &PmFixedTlsPeerSelection,
        local_egress: &PmLocalEgressSelection,
    ) -> Result<Self, PmPolygonFixedPeerSourceError> {
        fixed_peer.require_loopback_evidence()?;
        local_egress.require_loopback_evidence()?;
        fixed_peer.require_same_address_family(local_egress)?;
        if !(MIN_LOOPBACK_REQUEST_TIMEOUT..=MAX_LOOPBACK_REQUEST_TIMEOUT).contains(&request_timeout)
        {
            return Err(PmPolygonChainSourceError::InvalidLoopbackTimeout.into());
        }
        let origin = Url::parse(&format!(
            "http://{}:{}/",
            fixed_peer.dns_name(),
            fixed_peer.peer_addr().port()
        ))
        .map_err(|_| PmPolygonChainSourceError::InvalidLoopbackOrigin)?;
        Ok(Self {
            transport: PmPolygonRpcTransport::build(
                origin,
                request_timeout.min(PRODUCTION_CONNECT_TIMEOUT),
                request_timeout,
                false,
                Some(local_egress),
                Some(fixed_peer),
            )?,
            clock: ClockSource::Fixed(clock),
            mode: SourceMode::LoopbackEvidence,
        })
    }

    #[cfg(any(test, feature = "loopback-evidence"))]
    fn loopback_evidence_with_local_egress(
        origin: &str,
        clock: PmPolygonSystemClockObservation,
        request_timeout: Duration,
        local_egress: Option<&PmLocalEgressSelection>,
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
                local_egress,
                None,
            )?,
            clock: ClockSource::Fixed(clock),
            mode: SourceMode::LoopbackEvidence,
        })
    }

    /// Verify the private production mode before I/O, then read and seal one
    /// exact finalized authorization cut as move-only production-origin
    /// evidence.
    pub async fn production_finalized_authorization_cut(
        &self,
        scope: PmPolygonAuthorizationScope,
    ) -> Result<
        PmProductionPolygonFinalizedAuthorizationCut,
        PmProductionPolygonFinalizedAuthorizationError,
    > {
        let production_origin = ProductionPolygonOrigin::verify(self.mode)?;
        let cut = self.finalized_authorization_cut(scope).await?;
        Ok(PmProductionPolygonFinalizedAuthorizationCut::from_source(
            production_origin,
            cut,
        ))
    }

    /// Reads the exact five-request cut in a fixed order and returns only
    /// after every response, block binding, and freshness check succeeds.
    pub async fn finalized_authorization_cut(
        &self,
        scope: PmPolygonAuthorizationScope,
    ) -> Result<PmPolygonFinalizedAuthorizationCut, PmPolygonChainSourceError> {
        let chain_id_body = self.transport.post(chain_id_request()?).await?;
        let chain_id = decode_chain_id(&chain_id_body)?;
        if chain_id != PM_POLYGON_CHAIN_ID {
            return Err(PmPolygonChainSourceError::WrongRpcChain);
        }

        let finalized_body = self.transport.post(finalized_block_request()?).await?;
        let finalized = decode_finalized_block(&finalized_body, FINALIZED_BLOCK_REQUEST_ID)?;
        let owner = scope.owner();
        let spender = scope.spender().address();
        let pusd = frozen_address(PM_POLYGON_PUSD_PROXY_ADDRESS);
        let conditional_tokens = frozen_address(PM_POLYGON_CONDITIONAL_TOKENS_ADDRESS);

        let allowance_body = self
            .transport
            .post(allowance_request(
                pusd,
                owner,
                spender,
                &finalized.canonical_number,
            )?)
            .await?;
        let pusd_allowance = decode_allowance(&allowance_body)?;
        let approval_body = self
            .transport
            .post(approval_request(
                conditional_tokens,
                owner,
                spender,
                &finalized.canonical_number,
            )?)
            .await?;
        let conditional_tokens_approval = decode_approval(&approval_body)?;
        let reread_body = self
            .transport
            .post(block_reread_request(&finalized.canonical_number)?)
            .await?;
        let reread = decode_finalized_block(&reread_body, BLOCK_REREAD_REQUEST_ID)?;
        if reread.identity != finalized.identity {
            return Err(PmPolygonChainSourceError::FinalizedBlockChanged);
        }

        let observed_clock = self.clock.observe()?;
        validate_freshness(finalized.identity.timestamp, observed_clock.unix_seconds())?;
        let commitment = authorization_cut_commitment(AuthorizationCommitmentBasis {
            mode: self.mode,
            scope,
            chain_id,
            finalized: finalized.identity,
            pusd_allowance,
            conditional_tokens_approval,
            reread: reread.identity,
            observed_clock,
            response_bodies: [
                &chain_id_body,
                &finalized_body,
                &allowance_body,
                &approval_body,
                &reread_body,
            ],
        });

        Ok(PmPolygonFinalizedAuthorizationCut {
            scope,
            block: finalized.identity,
            pusd_allowance,
            conditional_tokens_approval,
            observed_clock,
            commitment,
        })
    }
}

struct AuthorizationCommitmentBasis<'a> {
    mode: SourceMode,
    scope: PmPolygonAuthorizationScope,
    chain_id: u64,
    finalized: PmPolygonFinalizedBlock,
    pusd_allowance: U256,
    conditional_tokens_approval: PmErc1155OperatorApproval,
    reread: PmPolygonFinalizedBlock,
    observed_clock: PmPolygonSystemClockObservation,
    response_bodies: [&'a [u8]; 5],
}

fn authorization_cut_commitment(
    basis: AuthorizationCommitmentBasis<'_>,
) -> PmPolygonFinalizedAuthorizationCommitment {
    let mut hasher = Sha256::new();
    hasher.update(AUTHORIZATION_CUT_COMMITMENT_DOMAIN);
    update_commitment_bytes(&mut hasher, PM_POLYGON_RPC_ORIGIN.as_bytes());
    hasher.update([match basis.mode {
        SourceMode::Production => 0,
        #[cfg(any(test, feature = "loopback-evidence"))]
        SourceMode::LoopbackEvidence => 1,
    }]);

    let account = basis.scope.account_scope();
    update_commitment_bytes(&mut hasher, account.environment().as_str().as_bytes());
    hasher.update(account.chain().value().to_be_bytes());
    hasher.update(account.signer().address().bytes());
    hasher.update(account.funder().address().bytes());
    hasher.update(account.handle().ordinal().to_be_bytes());
    hasher.update([match basis.scope.spender() {
        crate::PmPolygonExchangeSpender::StandardV2 => 0,
        crate::PmPolygonExchangeSpender::NegativeRiskV2 => 1,
    }]);
    hasher.update([match basis.scope.spender().domain() {
        PmSpenderDomain::Standard => 0,
        PmSpenderDomain::NegativeRisk => 1,
    }]);
    hasher.update(basis.scope.owner().bytes());
    hasher.update(basis.scope.spender().address().bytes());
    hasher.update(frozen_address(PM_POLYGON_PUSD_PROXY_ADDRESS).bytes());
    hasher.update(frozen_address(PM_POLYGON_CONDITIONAL_TOKENS_ADDRESS).bytes());

    hasher.update([AUTHORIZATION_CUT_REQUEST_COUNT]);
    update_rpc_observation(
        &mut hasher,
        0,
        CHAIN_ID_REQUEST_ID,
        basis.response_bodies[0],
    );
    hasher.update(basis.chain_id.to_be_bytes());
    update_rpc_observation(
        &mut hasher,
        1,
        FINALIZED_BLOCK_REQUEST_ID,
        basis.response_bodies[1],
    );
    update_block(&mut hasher, basis.finalized);
    update_rpc_observation(
        &mut hasher,
        2,
        PUSD_ALLOWANCE_REQUEST_ID,
        basis.response_bodies[2],
    );
    hasher.update(basis.pusd_allowance.to_be_bytes());
    update_rpc_observation(
        &mut hasher,
        3,
        CONDITIONAL_TOKENS_APPROVAL_REQUEST_ID,
        basis.response_bodies[3],
    );
    hasher.update([u8::from(basis.conditional_tokens_approval.is_approved())]);
    update_rpc_observation(
        &mut hasher,
        4,
        BLOCK_REREAD_REQUEST_ID,
        basis.response_bodies[4],
    );
    update_block(&mut hasher, basis.reread);
    hasher.update(basis.observed_clock.unix_seconds().to_be_bytes());
    PmPolygonFinalizedAuthorizationCommitment::from_source_bytes(hasher.finalize().into())
}

fn update_rpc_observation(hasher: &mut Sha256, ordinal: u8, request_id: u64, body: &[u8]) {
    hasher.update([ordinal]);
    hasher.update(request_id.to_be_bytes());
    update_commitment_bytes(hasher, body);
}

fn update_block(hasher: &mut Sha256, block: PmPolygonFinalizedBlock) {
    hasher.update(block.number().to_be_bytes());
    hasher.update(block.hash());
    hasher.update(block.timestamp().to_be_bytes());
}

fn update_commitment_bytes(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(
        u64::try_from(value.len())
            .expect("bounded chain observation length")
            .to_be_bytes(),
    );
    hasher.update(value);
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
    expected_peer: Option<std::net::SocketAddr>,
}

impl PmPolygonRpcTransport {
    fn build(
        origin: Url,
        connect_timeout: Duration,
        request_timeout: Duration,
        production: bool,
        local_egress: Option<&PmLocalEgressSelection>,
        fixed_peer: Option<&PmFixedTlsPeerSelection>,
    ) -> Result<Self, PmPolygonChainSourceError> {
        let mut builder = Client::builder()
            .connect_timeout(connect_timeout)
            .timeout(request_timeout)
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy();
        if let Some(local_egress) = local_egress {
            #[cfg(target_os = "linux")]
            {
                builder = builder
                    .interface(local_egress.interface_name())
                    .local_address(local_egress.local_source_ip());
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = local_egress;
                return Err(PmPolygonChainSourceError::SelectedLocalEgressUnsupported);
            }
        }
        if let Some(fixed_peer) = fixed_peer {
            if origin.host_str() != Some(fixed_peer.dns_name()) {
                return Err(PmPolygonChainSourceError::TransportBuild);
            }
            builder = builder.resolve(fixed_peer.dns_name(), fixed_peer.peer_addr());
        }
        if production {
            builder = builder.https_only(true);
        }
        let client = builder
            .build()
            .map_err(|_| PmPolygonChainSourceError::TransportBuild)?;
        Ok(Self {
            client,
            origin,
            expected_peer: fixed_peer.map(PmFixedTlsPeerSelection::peer_addr),
        })
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
        if self
            .expected_peer
            .is_some_and(|expected_peer| response.remote_addr() != Some(expected_peer))
        {
            return Err(PmPolygonChainSourceError::RequestFailed);
        }
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
