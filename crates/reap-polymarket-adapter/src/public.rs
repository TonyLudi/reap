use reap_pm_core::{
    PmBookEvent, PmConnectionId, PmInstrumentHandle, PmMarketEvent, PmMarketMetadata,
    PmProductSource, PmPublicObservationGrant,
};
use reap_polymarket_wire::{PmBookParserConfig, PmWireScope};
use thiserror::Error;

mod sealed {
    pub trait Sealed {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPublicRoleError {
    #[error("public PM role requires a Polymarket market source")]
    WrongSource,
    #[error("public PM role source token does not match the instrument")]
    SourceTokenMismatch,
    #[error("public PM role instrument does not match the canonical observation grant")]
    GrantInstrumentMismatch,
    #[error("public PM role market does not match the canonical observation grant")]
    GrantMarketMismatch,
    #[error("public PM role token does not match the canonical observation grant")]
    GrantTokenMismatch,
}

/// Static public-observation capability contract.
///
/// Session bytes and parsing arrive in Phase 3. This seam binds only the
/// configured PM outcome and cannot be converted into a private or execution
/// capability.
pub trait PmPublicObservationRole: sealed::Sealed {
    type MarketObservation;
    type BookObservation;

    fn instrument(&self) -> PmInstrumentHandle;
    fn wire_scope(&self) -> PmWireScope;
    fn source(&self) -> PmProductSource;
    fn connection(&self) -> PmConnectionId;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPublicRole {
    observation_grant: PmPublicObservationGrant,
    instrument: PmInstrumentHandle,
    parser_config: PmBookParserConfig,
    source: PmProductSource,
    connection: PmConnectionId,
}

impl PmPublicRole {
    /// Constructs the one-outcome public role directly from the product's
    /// expected exact metadata contract.
    ///
    /// Live session readiness still requires a matching
    /// `PmAuthoritativeMetadata` observation; this constructor only prevents
    /// composition roots from needing raw wire configuration types.
    pub fn from_expected_metadata(
        observation_grant: PmPublicObservationGrant,
        expected: PmMarketMetadata,
        source: PmProductSource,
        connection: PmConnectionId,
    ) -> Result<Self, PmPublicRoleError> {
        let parser_config = PmBookParserConfig::new(
            PmWireScope::new(
                expected.condition(),
                expected.market(),
                expected.outcome().token(),
            ),
            expected.tick(),
            expected.minimum_order_size(),
            expected.negative_risk(),
        );
        Self::new(
            observation_grant,
            observation_grant.instrument(),
            parser_config,
            source,
            connection,
        )
    }

    pub fn new(
        observation_grant: PmPublicObservationGrant,
        instrument: PmInstrumentHandle,
        parser_config: PmBookParserConfig,
        source: PmProductSource,
        connection: PmConnectionId,
    ) -> Result<Self, PmPublicRoleError> {
        if instrument != observation_grant.instrument() {
            return Err(PmPublicRoleError::GrantInstrumentMismatch);
        }
        if parser_config.scope().market() != observation_grant.polymarket_instrument().market() {
            return Err(PmPublicRoleError::GrantMarketMismatch);
        }
        if parser_config.scope().token() != observation_grant.polymarket_instrument().token() {
            return Err(PmPublicRoleError::GrantTokenMismatch);
        }
        match source {
            PmProductSource::PolymarketMarket { token, .. } if token == instrument.token() => {
                Ok(Self {
                    observation_grant,
                    instrument,
                    parser_config,
                    source,
                    connection,
                })
            }
            PmProductSource::PolymarketMarket { .. } => Err(PmPublicRoleError::SourceTokenMismatch),
            PmProductSource::OkxReference { .. } | PmProductSource::PolymarketAccount { .. } => {
                Err(PmPublicRoleError::WrongSource)
            }
        }
    }

    #[must_use]
    pub const fn observation_grant(&self) -> PmPublicObservationGrant {
        self.observation_grant
    }

    #[must_use]
    pub const fn instrument(&self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn parser_config(&self) -> PmBookParserConfig {
        self.parser_config
    }

    #[must_use]
    pub const fn wire_scope(&self) -> PmWireScope {
        self.parser_config.scope()
    }

    #[must_use]
    pub const fn source(&self) -> PmProductSource {
        self.source
    }

    #[must_use]
    pub const fn connection(&self) -> PmConnectionId {
        self.connection
    }
}

impl sealed::Sealed for PmPublicRole {}

impl PmPublicObservationRole for PmPublicRole {
    type MarketObservation = PmMarketEvent;
    type BookObservation = PmBookEvent;

    fn instrument(&self) -> PmInstrumentHandle {
        self.instrument
    }

    fn wire_scope(&self) -> PmWireScope {
        self.parser_config.scope()
    }

    fn source(&self) -> PmProductSource {
        self.source
    }

    fn connection(&self) -> PmConnectionId {
        self.connection
    }
}
