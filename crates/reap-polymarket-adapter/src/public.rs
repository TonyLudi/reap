use reap_pm_core::{
    PmBookEvent, PmConnectionId, PmInstrumentHandle, PmMarketEvent, PmProductSource,
};
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
    fn source(&self) -> PmProductSource;
    fn connection(&self) -> PmConnectionId;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPublicRole {
    instrument: PmInstrumentHandle,
    source: PmProductSource,
    connection: PmConnectionId,
}

impl PmPublicRole {
    pub fn new(
        instrument: PmInstrumentHandle,
        source: PmProductSource,
        connection: PmConnectionId,
    ) -> Result<Self, PmPublicRoleError> {
        match source {
            PmProductSource::PolymarketMarket { token, .. } if token == instrument.token() => {
                Ok(Self {
                    instrument,
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
    pub const fn instrument(&self) -> PmInstrumentHandle {
        self.instrument
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

    fn source(&self) -> PmProductSource {
        self.source
    }

    fn connection(&self) -> PmConnectionId {
        self.connection
    }
}
