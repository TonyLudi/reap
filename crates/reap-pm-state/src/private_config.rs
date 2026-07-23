use reap_pm_core::{
    PmAccountHandle, PmAccountScope, PmAssetId, PmGoalFTradingDomain, PmInstrumentHandle,
    PmInstrumentId, PmMarketMetadata, PmProductSource, PmQuantity, PmSpenderId, PmTick,
};
use thiserror::Error;

/// Immutable, locally-authoritative scope for one PM private-state owner.
///
/// Snapshot-declared expected scopes are checked against this configuration;
/// they never redefine it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmPrivateStateConfig {
    source: PmProductSource,
    account_scope: PmAccountScope,
    instrument: PmInstrumentHandle,
    metadata: PmMarketMetadata,
    trading_domain: PmGoalFTradingDomain,
    required_spenders: [PmSpenderId; 2],
}

impl PmPrivateStateConfig {
    pub fn new(
        source: PmProductSource,
        account_scope: PmAccountScope,
        instrument: PmInstrumentHandle,
        metadata: PmMarketMetadata,
    ) -> Result<Self, PmPrivateConfigError> {
        validate_source(source, account_scope)?;
        let trading_domain = PmGoalFTradingDomain::from_metadata(metadata)?;
        if trading_domain.chain() != account_scope.chain() {
            return Err(PmPrivateConfigError::TradingDomainChainMismatch);
        }
        let [first, second] = trading_domain.required_spenders();
        let mut required_spenders = [
            PmSpenderId::new(account_scope.handle(), first),
            PmSpenderId::new(account_scope.handle(), second),
        ];
        required_spenders.sort_by_key(|spender| (spender.account(), spender.requirement()));
        Ok(Self {
            source,
            account_scope,
            instrument,
            metadata,
            trading_domain,
            required_spenders,
        })
    }

    #[must_use]
    pub const fn source(&self) -> PmProductSource {
        self.source
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
    pub const fn instrument(&self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn instrument_id(&self) -> PmInstrumentId {
        self.trading_domain.instrument()
    }

    #[must_use]
    pub const fn metadata(&self) -> PmMarketMetadata {
        self.metadata
    }

    #[must_use]
    pub const fn tick(&self) -> PmTick {
        self.metadata.tick()
    }

    #[must_use]
    pub const fn minimum_order_size(&self) -> PmQuantity {
        self.metadata.minimum_order_size()
    }

    #[must_use]
    pub const fn collateral_asset(&self) -> PmAssetId {
        self.trading_domain.collateral()
    }

    #[must_use]
    pub const fn outcome_asset(&self) -> PmAssetId {
        self.trading_domain.outcome()
    }

    #[must_use]
    pub fn required_spenders(&self) -> &[PmSpenderId] {
        &self.required_spenders
    }

    pub(crate) fn expected_assets_match(&self, actual: &[PmAssetId]) -> bool {
        actual.len() == 2
            && actual.contains(&self.trading_domain.collateral())
            && actual.contains(&self.trading_domain.outcome())
    }

    pub(crate) fn expected_spenders_match(&self, actual: &[PmSpenderId]) -> bool {
        actual.len() == self.required_spenders.len()
            && self
                .required_spenders
                .iter()
                .all(|spender| actual.contains(spender))
    }

    pub(crate) fn expected_instruments_match(&self, actual: &[PmInstrumentHandle]) -> bool {
        actual == [self.instrument]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPrivateConfigError {
    #[error("private-state source must be the exact configured PM account source")]
    SourceMismatch,
    #[error("private-state account chain differs from the Goal F trading domain")]
    TradingDomainChainMismatch,
    #[error(transparent)]
    TradingDomain(#[from] reap_pm_core::PmMetadataError),
}

fn validate_source(
    source: PmProductSource,
    account_scope: PmAccountScope,
) -> Result<(), PmPrivateConfigError> {
    match source {
        PmProductSource::PolymarketAccount { account, .. } if account == account_scope.handle() => {
            Ok(())
        }
        PmProductSource::PolymarketAccount { .. }
        | PmProductSource::PolymarketMarket { .. }
        | PmProductSource::OkxReference { .. } => Err(PmPrivateConfigError::SourceMismatch),
    }
}
