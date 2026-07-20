use reap_pm_core::{
    MAX_REQUIRED_SPENDERS, PmAccountHandle, PmAccountScope, PmAllowanceEvent, PmBalanceEvent,
    PmConnectionId, PmInstrumentHandle, PmPositionEvent, PmProductSource, PmSpenderId,
};
use thiserror::Error;

mod sealed {
    pub trait Sealed {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmAccountPositionRoleError {
    #[error("account/position role exceeds the bounded required-spender capacity")]
    TooManySpenders,
    #[error("required spender belongs to a different account")]
    SpenderAccountMismatch,
    #[error("required spender belongs to a different chain")]
    SpenderChainMismatch,
    #[error("required spender occurs more than once")]
    DuplicateSpender,
    #[error("account snapshot role requires a Polymarket account source")]
    WrongSource,
    #[error("account snapshot role source belongs to another account")]
    SourceAccountMismatch,
}

/// Fixture-only collateral, allowance, inventory, and position snapshot role.
pub trait PmAccountPositionSnapshotRole: sealed::Sealed {
    type BalanceObservation;
    type AllowanceObservation;
    type PositionObservation;

    fn account_scope(&self) -> PmAccountScope;
    fn account(&self) -> PmAccountHandle;
    fn instrument(&self) -> PmInstrumentHandle;
    fn source(&self) -> PmProductSource;
    fn connection(&self) -> PmConnectionId;
    fn required_spenders(&self) -> &[PmSpenderId];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmFixtureAccountPositionSnapshot {
    account_scope: PmAccountScope,
    instrument: PmInstrumentHandle,
    source: PmProductSource,
    connection: PmConnectionId,
    required_spenders: Vec<PmSpenderId>,
}

impl PmFixtureAccountPositionSnapshot {
    pub fn new(
        account_scope: PmAccountScope,
        instrument: PmInstrumentHandle,
        source: PmProductSource,
        connection: PmConnectionId,
        mut required_spenders: Vec<PmSpenderId>,
    ) -> Result<Self, PmAccountPositionRoleError> {
        if required_spenders.len() > MAX_REQUIRED_SPENDERS {
            return Err(PmAccountPositionRoleError::TooManySpenders);
        }
        match source {
            PmProductSource::PolymarketAccount { account, .. }
                if account == account_scope.handle() => {}
            PmProductSource::PolymarketAccount { .. } => {
                return Err(PmAccountPositionRoleError::SourceAccountMismatch);
            }
            PmProductSource::OkxReference { .. } | PmProductSource::PolymarketMarket { .. } => {
                return Err(PmAccountPositionRoleError::WrongSource);
            }
        }
        if required_spenders
            .iter()
            .any(|spender| spender.account() != account_scope.handle())
        {
            return Err(PmAccountPositionRoleError::SpenderAccountMismatch);
        }
        if required_spenders
            .iter()
            .any(|spender| spender.requirement().chain() != account_scope.chain())
        {
            return Err(PmAccountPositionRoleError::SpenderChainMismatch);
        }
        required_spenders.sort_by_key(|spender| spender.requirement());
        if required_spenders.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(PmAccountPositionRoleError::DuplicateSpender);
        }
        Ok(Self {
            account_scope,
            instrument,
            source,
            connection,
            required_spenders,
        })
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
    pub const fn source(&self) -> PmProductSource {
        self.source
    }

    #[must_use]
    pub const fn connection(&self) -> PmConnectionId {
        self.connection
    }

    #[must_use]
    pub fn required_spenders(&self) -> &[PmSpenderId] {
        &self.required_spenders
    }
}

impl sealed::Sealed for PmFixtureAccountPositionSnapshot {}

impl PmAccountPositionSnapshotRole for PmFixtureAccountPositionSnapshot {
    type BalanceObservation = PmBalanceEvent;
    type AllowanceObservation = PmAllowanceEvent;
    type PositionObservation = PmPositionEvent;

    fn account_scope(&self) -> PmAccountScope {
        self.account_scope
    }

    fn account(&self) -> PmAccountHandle {
        self.account_scope.handle()
    }

    fn instrument(&self) -> PmInstrumentHandle {
        self.instrument
    }

    fn source(&self) -> PmProductSource {
        self.source
    }

    fn connection(&self) -> PmConnectionId {
        self.connection
    }

    fn required_spenders(&self) -> &[PmSpenderId] {
        &self.required_spenders
    }
}
