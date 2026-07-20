use reap_pm_core::{
    PmAccountHandle, PmAccountScope, PmConnectionId, PmFillEvent, PmOrderEvent, PmProductSource,
};
use thiserror::Error;

mod sealed {
    pub trait Sealed {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPrivateLifecycleRoleError {
    #[error("private PM role requires a Polymarket account source")]
    WrongSource,
    #[error("private PM role source belongs to another account")]
    SourceAccountMismatch,
}

/// Fixture-only PM order/fill lifecycle observation capability.
pub trait PmPrivateLifecycleRole: sealed::Sealed {
    type OrderObservation;
    type FillObservation;

    fn account_scope(&self) -> PmAccountScope;
    fn account(&self) -> PmAccountHandle;
    fn source(&self) -> PmProductSource;
    fn connection(&self) -> PmConnectionId;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmFixturePrivateLifecycle {
    account_scope: PmAccountScope,
    source: PmProductSource,
    connection: PmConnectionId,
}

impl PmFixturePrivateLifecycle {
    pub fn new(
        account_scope: PmAccountScope,
        source: PmProductSource,
        connection: PmConnectionId,
    ) -> Result<Self, PmPrivateLifecycleRoleError> {
        match source {
            PmProductSource::PolymarketAccount { account, .. }
                if account == account_scope.handle() =>
            {
                Ok(Self {
                    account_scope,
                    source,
                    connection,
                })
            }
            PmProductSource::PolymarketAccount { .. } => {
                Err(PmPrivateLifecycleRoleError::SourceAccountMismatch)
            }
            PmProductSource::OkxReference { .. } | PmProductSource::PolymarketMarket { .. } => {
                Err(PmPrivateLifecycleRoleError::WrongSource)
            }
        }
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
    pub const fn source(&self) -> PmProductSource {
        self.source
    }

    #[must_use]
    pub const fn connection(&self) -> PmConnectionId {
        self.connection
    }
}

impl sealed::Sealed for PmFixturePrivateLifecycle {}

impl PmPrivateLifecycleRole for PmFixturePrivateLifecycle {
    type OrderObservation = PmOrderEvent;
    type FillObservation = PmFillEvent;

    fn account_scope(&self) -> PmAccountScope {
        self.account_scope
    }

    fn account(&self) -> PmAccountHandle {
        self.account_scope.handle()
    }

    fn source(&self) -> PmProductSource {
        self.source
    }

    fn connection(&self) -> PmConnectionId {
        self.connection
    }
}
