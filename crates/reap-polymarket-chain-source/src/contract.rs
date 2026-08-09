use std::time::{SystemTime, UNIX_EPOCH};

use reap_pm_core::{EvmAddress, PmAccountScope, PmErc1155OperatorApproval, PmSpenderDomain, U256};

use crate::PmPolygonChainSourceError;

pub const PM_POLYGON_CHAIN_ID: u64 = 137;
pub const PM_POLYGON_RPC_ORIGIN: &str = "https://polygon.drpc.org";
pub const PM_POLYGON_STANDARD_V2_EXCHANGE_ADDRESS: &str =
    "0xE111180000d2663C0091e4f400237545B87B996B";
pub const PM_POLYGON_NEGATIVE_RISK_V2_EXCHANGE_ADDRESS: &str =
    "0xe2222d279d744050d28e00520010520000310F59";
pub const PM_POLYGON_PUSD_PROXY_ADDRESS: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
pub const PM_POLYGON_CONDITIONAL_TOKENS_ADDRESS: &str =
    "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";

/// The only Polygon exchange contracts admitted by the PM-T2 authorization
/// observation. Callers cannot supply an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PmPolygonExchangeSpender {
    StandardV2,
    NegativeRiskV2,
}

impl PmPolygonExchangeSpender {
    #[must_use]
    pub fn address(self) -> EvmAddress {
        EvmAddress::parse(match self {
            Self::StandardV2 => PM_POLYGON_STANDARD_V2_EXCHANGE_ADDRESS,
            Self::NegativeRiskV2 => PM_POLYGON_NEGATIVE_RISK_V2_EXCHANGE_ADDRESS,
        })
        .expect("frozen Polygon exchange address")
    }

    #[must_use]
    pub const fn domain(self) -> PmSpenderDomain {
        match self {
            Self::StandardV2 => PmSpenderDomain::Standard,
            Self::NegativeRiskV2 => PmSpenderDomain::NegativeRisk,
        }
    }
}

/// A closed PM-T2 proxy account and one frozen exchange spender.
///
/// The allowance owner is never caller-supplied: it is always the account's
/// proxy funder. Construction rejects equal signer/funder identities and
/// non-Polygon scopes before any request can be sent. Distinct identities are
/// structural proxy scope here, not independent signature-type attestation;
/// the outer connectivity/preflight contract must bind `ProxyType1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmPolygonAuthorizationScope {
    account_scope: PmAccountScope,
    spender: PmPolygonExchangeSpender,
}

impl PmPolygonAuthorizationScope {
    pub fn new_pm_t2_proxy(
        account_scope: PmAccountScope,
        spender: PmPolygonExchangeSpender,
    ) -> Result<Self, PmPolygonChainSourceError> {
        if account_scope.chain().value() != PM_POLYGON_CHAIN_ID {
            return Err(PmPolygonChainSourceError::WrongAccountChain);
        }
        if account_scope.signer().address() == account_scope.funder().address() {
            return Err(PmPolygonChainSourceError::SignerFunderNotDistinct);
        }
        Ok(Self {
            account_scope,
            spender,
        })
    }

    #[must_use]
    pub const fn account_scope(self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn spender(self) -> PmPolygonExchangeSpender {
        self.spender
    }

    #[must_use]
    pub const fn owner(self) -> EvmAddress {
        self.account_scope.funder().address()
    }
}

/// Wall-clock evidence used only to assess finalized-block freshness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmPolygonSystemClockObservation {
    unix_seconds: u64,
}

impl PmPolygonSystemClockObservation {
    pub(crate) fn capture() -> Result<Self, PmPolygonChainSourceError> {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| PmPolygonChainSourceError::SystemClockBeforeUnixEpoch)?;
        Ok(Self {
            unix_seconds: elapsed.as_secs(),
        })
    }

    /// Constructs deterministic clock evidence only for the explicitly
    /// feature-gated numeric-loopback source.
    #[cfg(any(test, feature = "loopback-evidence"))]
    #[must_use]
    pub const fn for_loopback_evidence(unix_seconds: u64) -> Self {
        Self { unix_seconds }
    }

    #[must_use]
    pub const fn unix_seconds(self) -> u64 {
        self.unix_seconds
    }
}

/// Exact identity of the finalized Polygon block shared by every observation
/// in one authorization cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmPolygonFinalizedBlock {
    pub(crate) number: u64,
    pub(crate) hash: [u8; 32],
    pub(crate) timestamp: u64,
}

impl PmPolygonFinalizedBlock {
    #[must_use]
    pub const fn number(self) -> u64 {
        self.number
    }

    #[must_use]
    pub const fn hash(self) -> [u8; 32] {
        self.hash
    }

    #[must_use]
    pub const fn timestamp(self) -> u64 {
        self.timestamp
    }
}

/// One all-or-nothing, finalized-block observation of the two on-chain
/// approvals needed by a PM-T2 proxy account.
///
/// This value is evidence only. It contains no credential, signature,
/// dispatch permit, or production order-entry authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmPolygonFinalizedAuthorizationCut {
    pub(crate) scope: PmPolygonAuthorizationScope,
    pub(crate) block: PmPolygonFinalizedBlock,
    pub(crate) pusd_allowance: U256,
    pub(crate) conditional_tokens_approval: PmErc1155OperatorApproval,
    pub(crate) observed_clock: PmPolygonSystemClockObservation,
}

impl PmPolygonFinalizedAuthorizationCut {
    #[must_use]
    pub const fn scope(self) -> PmPolygonAuthorizationScope {
        self.scope
    }

    #[must_use]
    pub const fn block(self) -> PmPolygonFinalizedBlock {
        self.block
    }

    #[must_use]
    pub const fn pusd_allowance(self) -> U256 {
        self.pusd_allowance
    }

    #[must_use]
    pub const fn conditional_tokens_approval(self) -> PmErc1155OperatorApproval {
        self.conditional_tokens_approval
    }

    #[must_use]
    pub const fn observed_clock(self) -> PmPolygonSystemClockObservation {
        self.observed_clock
    }

    /// This read-only observation is deliberately not mutation authority.
    #[must_use]
    pub const fn production_order_entry_authorized(self) -> bool {
        false
    }
}
