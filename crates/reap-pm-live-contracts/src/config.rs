use reap_pm_core::{
    MAX_OKX_REFERENCES_PER_MAPPING, OkxReferenceHandle, OkxReferenceInstrument, PmAccountHandle,
    PmAccountScope, PmAssetId, PmConfigurationFingerprint, PmConnectionId, PmGoalFTradingDomain,
    PmInstrumentHandle, PmInstrumentId, PmMarketMetadata, PmProductSource,
    PmPublicObservationGrant, PmReferenceMapping, PmSpenderId,
};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmConnectivityConfigError {
    #[error("the fixed PM profile requires exactly one checked OKX reference mapping")]
    ExpectedSingleReference,
    #[error("Goal F OKX reference handle must be canonical zero-based ordinal 0")]
    NonCanonicalReferenceHandle,
    #[error("Goal F PM market/token handles must be canonical zero-based ordinals 0/0")]
    NonCanonicalInstrumentHandle,
    #[error("the fixed PM profile is restricted to Polygon chain 137")]
    WrongGoalFChain,
    #[error("the fixed PM profile requires the EOA signer and funder to match")]
    SignerFunderMismatch,
    #[error("configured required spender belongs to another chain")]
    SpenderChainMismatch,
    #[error("public and account scopes name different PM instruments")]
    InstrumentScopeMismatch,
    #[error("the account market/grid contract differs from the configured public PM instrument")]
    AccountInstrumentScopeMismatch,
    #[error("metadata does not prove the exact Goal F Polygon trading domain")]
    UnsupportedGoalFTradingDomain,
    #[error("OKX route does not name the checked reference")]
    OkxRouteMismatch,
    #[error("PM public route does not name the checked outcome token")]
    PublicRouteMismatch,
    #[error("PM account route does not name the checked account")]
    AccountRouteMismatch,
    #[error("configured PM account chain differs from authoritative market metadata")]
    AccountMarketChainMismatch,
    #[error("account allowance spenders differ from the authoritative exact market spender set")]
    RequiredSpenderSetMismatch,
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
    observation_grant: PmPublicObservationGrant,
    mapping: PmReferenceMapping,
    okx_reference_instrument: OkxReferenceInstrument,
    expected_metadata: PmMarketMetadata,
    trading_domain: PmGoalFTradingDomain,
    okx_route: PmConnectionRoute,
    polymarket_route: PmConnectionRoute,
}

impl PmPublicConnectivityConfig {
    pub fn derive_goal_f(
        okx_reference_instrument: OkxReferenceInstrument,
        expected_metadata: PmMarketMetadata,
        okx_route: PmConnectionRoute,
        polymarket_route: PmConnectionRoute,
    ) -> Result<Self, PmConnectivityConfigError> {
        let observation_grant = PmPublicObservationGrant::derive_goal_f(
            okx_reference_instrument,
            PmInstrumentId::new(
                expected_metadata.market(),
                expected_metadata.outcome().token(),
            ),
        );
        let mut references = [None; MAX_OKX_REFERENCES_PER_MAPPING];
        references[0] = Some(observation_grant.okx_reference());
        let mapping = PmReferenceMapping::new(observation_grant.instrument(), references, 1)
            .expect("one canonical Goal-F reference");
        Self::new(
            mapping,
            okx_reference_instrument,
            expected_metadata,
            okx_route,
            polymarket_route,
        )
    }

    pub fn new(
        mapping: PmReferenceMapping,
        okx_reference_instrument: OkxReferenceInstrument,
        expected_metadata: PmMarketMetadata,
        okx_route: PmConnectionRoute,
        polymarket_route: PmConnectionRoute,
    ) -> Result<Self, PmConnectivityConfigError> {
        let trading_domain = PmGoalFTradingDomain::from_metadata(expected_metadata)
            .map_err(|_| PmConnectivityConfigError::UnsupportedGoalFTradingDomain)?;
        if mapping.reference_count() != 1 {
            return Err(PmConnectivityConfigError::ExpectedSingleReference);
        }
        let reference = mapping
            .references()
            .next()
            .expect("checked single-reference mapping");
        let observation_grant = PmPublicObservationGrant::derive_goal_f(
            okx_reference_instrument,
            PmInstrumentId::new(
                expected_metadata.market(),
                expected_metadata.outcome().token(),
            ),
        );
        if reference != observation_grant.okx_reference() {
            return Err(PmConnectivityConfigError::NonCanonicalReferenceHandle);
        }
        if mapping.target() != observation_grant.instrument() {
            return Err(PmConnectivityConfigError::NonCanonicalInstrumentHandle);
        }
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
            observation_grant,
            mapping,
            okx_reference_instrument,
            expected_metadata,
            trading_domain,
            okx_route,
            polymarket_route,
        })
    }

    #[must_use]
    pub const fn observation_grant(&self) -> PmPublicObservationGrant {
        self.observation_grant
    }

    #[must_use]
    pub const fn configuration_fingerprint(&self) -> PmConfigurationFingerprint {
        self.observation_grant.configuration_fingerprint()
    }

    #[must_use]
    pub const fn mapping(&self) -> PmReferenceMapping {
        self.mapping
    }

    #[must_use]
    pub fn okx_reference(&self) -> OkxReferenceHandle {
        self.observation_grant.okx_reference()
    }

    #[must_use]
    pub const fn okx_reference_instrument(&self) -> OkxReferenceInstrument {
        self.okx_reference_instrument
    }

    #[must_use]
    pub const fn expected_metadata(&self) -> PmMarketMetadata {
        self.expected_metadata
    }

    #[must_use]
    pub const fn trading_domain(&self) -> PmGoalFTradingDomain {
        self.trading_domain
    }

    #[must_use]
    pub const fn polymarket_instrument_id(&self) -> PmInstrumentId {
        PmInstrumentId::new(
            self.expected_metadata.market(),
            self.expected_metadata.outcome().token(),
        )
    }

    #[must_use]
    pub const fn instrument(&self) -> PmInstrumentHandle {
        self.observation_grant.instrument()
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
    instrument_id: PmInstrumentId,
    expected_metadata: PmMarketMetadata,
    trading_domain: PmGoalFTradingDomain,
    account_scope: PmAccountScope,
    account_route: PmConnectionRoute,
    required_spenders: [PmSpenderId; 2],
}

impl PmAccountConnectivityConfig {
    /// Derives the private/account scope from the already checked public
    /// market contract so tick, minimum, structural identity, chain, assets,
    /// and spenders cannot drift across the two capability roots.
    pub fn derive_goal_f(
        public: &PmPublicConnectivityConfig,
        account_scope: PmAccountScope,
        account_route: PmConnectionRoute,
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
        if public.trading_domain().chain() != account_scope.chain() {
            return Err(PmConnectivityConfigError::SpenderChainMismatch);
        }
        let mut required_spenders = public
            .trading_domain()
            .required_spenders()
            .map(|requirement| PmSpenderId::new(account_scope.handle(), requirement));
        required_spenders.sort_by_key(|spender| spender.requirement());
        Ok(Self {
            instrument: public.instrument(),
            instrument_id: public.polymarket_instrument_id(),
            expected_metadata: public.expected_metadata(),
            trading_domain: public.trading_domain(),
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
    pub const fn instrument_id(&self) -> PmInstrumentId {
        self.instrument_id
    }

    #[must_use]
    pub const fn expected_metadata(&self) -> PmMarketMetadata {
        self.expected_metadata
    }

    #[must_use]
    pub const fn trading_domain(&self) -> PmGoalFTradingDomain {
        self.trading_domain
    }

    #[must_use]
    pub const fn collateral_asset(&self) -> PmAssetId {
        self.trading_domain.collateral()
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
        if public.instrument() != account.instrument()
            || public.polymarket_instrument_id() != account.instrument_id()
        {
            return Err(PmConnectivityConfigError::InstrumentScopeMismatch);
        }
        if public.expected_metadata() != account.expected_metadata() {
            return Err(PmConnectivityConfigError::AccountInstrumentScopeMismatch);
        }
        if public.trading_domain() != account.trading_domain() {
            return Err(PmConnectivityConfigError::RequiredSpenderSetMismatch);
        }
        if public.expected_metadata().chain() != account.account_scope().chain() {
            return Err(PmConnectivityConfigError::AccountMarketChainMismatch);
        }
        let expected = public
            .expected_metadata()
            .required_spenders()
            .collect::<Vec<_>>();
        let actual = account
            .required_spenders()
            .iter()
            .map(|spender| spender.requirement())
            .collect::<Vec<_>>();
        if expected != actual {
            return Err(PmConnectivityConfigError::RequiredSpenderSetMismatch);
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
