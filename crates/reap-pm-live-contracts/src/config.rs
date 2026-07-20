use reap_pm_core::{
    MAX_REQUIRED_SPENDERS, OkxReferenceHandle, PmAccountHandle, PmAccountScope, PmConnectionId,
    PmInstrumentHandle, PmProductSource, PmReferenceMapping, PmSpenderId,
};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmConnectivityConfigError {
    #[error("the fixed PM profile requires exactly one checked OKX reference mapping")]
    ExpectedSingleReference,
    #[error("the fixed PM profile is restricted to Polygon chain 137")]
    WrongGoalFChain,
    #[error("the fixed PM profile requires the EOA signer and funder to match")]
    SignerFunderMismatch,
    #[error("the fixed PM profile requires at least one exact allowance spender")]
    MissingRequiredSpender,
    #[error("configured required-spender count exceeds the domain bound")]
    TooManyRequiredSpenders,
    #[error("configured required spender belongs to another account")]
    SpenderAccountMismatch,
    #[error("configured required spender belongs to another chain")]
    SpenderChainMismatch,
    #[error("configured required spender occurs more than once")]
    DuplicateRequiredSpender,
    #[error("public and account scopes name different PM instruments")]
    InstrumentScopeMismatch,
    #[error("OKX route does not name the checked reference")]
    OkxRouteMismatch,
    #[error("PM public route does not name the checked outcome token")]
    PublicRouteMismatch,
    #[error("PM account route does not name the checked account")]
    AccountRouteMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmConnectionRoute {
    source: PmProductSource,
    connection: PmConnectionId,
}

impl PmConnectionRoute {
    #[must_use]
    pub const fn new(source: PmProductSource, connection: PmConnectionId) -> Self {
        Self { source, connection }
    }

    #[must_use]
    pub const fn source(self) -> PmProductSource {
        self.source
    }

    #[must_use]
    pub const fn connection(self) -> PmConnectionId {
        self.connection
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmPublicConnectivityConfig {
    mapping: PmReferenceMapping,
    okx_route: PmConnectionRoute,
    polymarket_route: PmConnectionRoute,
}

impl PmPublicConnectivityConfig {
    pub fn new(
        mapping: PmReferenceMapping,
        okx_route: PmConnectionRoute,
        polymarket_route: PmConnectionRoute,
    ) -> Result<Self, PmConnectivityConfigError> {
        if mapping.reference_count() != 1 {
            return Err(PmConnectivityConfigError::ExpectedSingleReference);
        }
        let reference = mapping
            .references()
            .next()
            .expect("checked single-reference mapping");
        if !matches!(
            okx_route.source(),
            PmProductSource::OkxReference {
                reference: source_reference,
                ..
            } if source_reference == reference
        ) {
            return Err(PmConnectivityConfigError::OkxRouteMismatch);
        }
        if !matches!(
            polymarket_route.source(),
            PmProductSource::PolymarketMarket { token, .. }
                if token == mapping.target().token()
        ) {
            return Err(PmConnectivityConfigError::PublicRouteMismatch);
        }
        Ok(Self {
            mapping,
            okx_route,
            polymarket_route,
        })
    }

    #[must_use]
    pub const fn mapping(&self) -> PmReferenceMapping {
        self.mapping
    }

    #[must_use]
    pub fn okx_reference(&self) -> OkxReferenceHandle {
        self.mapping
            .references()
            .next()
            .expect("checked single-reference mapping")
    }

    #[must_use]
    pub const fn instrument(&self) -> PmInstrumentHandle {
        self.mapping.target()
    }

    #[must_use]
    pub const fn okx_route(&self) -> PmConnectionRoute {
        self.okx_route
    }

    #[must_use]
    pub const fn polymarket_route(&self) -> PmConnectionRoute {
        self.polymarket_route
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmAccountConnectivityConfig {
    instrument: PmInstrumentHandle,
    account_scope: PmAccountScope,
    account_route: PmConnectionRoute,
    required_spenders: Vec<PmSpenderId>,
}

impl PmAccountConnectivityConfig {
    pub fn new(
        instrument: PmInstrumentHandle,
        account_scope: PmAccountScope,
        account_route: PmConnectionRoute,
        mut required_spenders: Vec<PmSpenderId>,
    ) -> Result<Self, PmConnectivityConfigError> {
        if account_scope.chain().value() != 137 {
            return Err(PmConnectivityConfigError::WrongGoalFChain);
        }
        if account_scope.signer().address() != account_scope.funder().address() {
            return Err(PmConnectivityConfigError::SignerFunderMismatch);
        }
        if !matches!(
            account_route.source(),
            PmProductSource::PolymarketAccount { account, .. }
                if account == account_scope.handle()
        ) {
            return Err(PmConnectivityConfigError::AccountRouteMismatch);
        }
        if required_spenders.is_empty() {
            return Err(PmConnectivityConfigError::MissingRequiredSpender);
        }
        if required_spenders.len() > MAX_REQUIRED_SPENDERS {
            return Err(PmConnectivityConfigError::TooManyRequiredSpenders);
        }
        if required_spenders
            .iter()
            .any(|spender| spender.account() != account_scope.handle())
        {
            return Err(PmConnectivityConfigError::SpenderAccountMismatch);
        }
        if required_spenders
            .iter()
            .any(|spender| spender.requirement().chain() != account_scope.chain())
        {
            return Err(PmConnectivityConfigError::SpenderChainMismatch);
        }
        required_spenders.sort_by_key(|spender| spender.requirement());
        if required_spenders.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(PmConnectivityConfigError::DuplicateRequiredSpender);
        }
        Ok(Self {
            instrument,
            account_scope,
            account_route,
            required_spenders,
        })
    }

    #[must_use]
    pub const fn instrument(&self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn account_scope(&self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn account(&self) -> PmAccountHandle {
        self.account_scope.handle()
    }

    #[must_use]
    pub const fn account_route(&self) -> PmConnectionRoute {
        self.account_route
    }

    #[must_use]
    pub fn required_spenders(&self) -> &[PmSpenderId] {
        &self.required_spenders
    }
}

/// Secret-free full-product connectivity scope.
///
/// The independent public-capture and read-only-monitor roots accept only
/// their narrower component types. This combined type is admitted solely by
/// full product construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmConnectivityConfig {
    public: PmPublicConnectivityConfig,
    account: PmAccountConnectivityConfig,
}

impl PmConnectivityConfig {
    pub fn new(
        public: PmPublicConnectivityConfig,
        account: PmAccountConnectivityConfig,
    ) -> Result<Self, PmConnectivityConfigError> {
        if public.instrument() != account.instrument() {
            return Err(PmConnectivityConfigError::InstrumentScopeMismatch);
        }
        Ok(Self { public, account })
    }

    #[must_use]
    pub const fn public(&self) -> &PmPublicConnectivityConfig {
        &self.public
    }

    #[must_use]
    pub const fn account(&self) -> &PmAccountConnectivityConfig {
        &self.account
    }
}
