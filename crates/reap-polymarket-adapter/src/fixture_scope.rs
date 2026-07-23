use std::fmt;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use reap_pm_core::{
    PmAccountScope, PmGoalFTradingDomain, PmInstrumentHandle, PmInstrumentId, PmMarketMetadata,
    PmMetadataError, PmProductSource, PmQuantity, PmTick,
};
use thiserror::Error;

/// Process-local identity of one read-role owner.
///
/// The numeric identity is deliberately opaque and never enters captured or
/// replayed evidence.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct PmFixtureOwnerId(NonZeroU64);

impl fmt::Debug for PmFixtureOwnerId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmFixtureOwnerId(<opaque>)")
    }
}

/// Move-only authority that can issue one complete set of PM read-role grants.
#[derive(Debug)]
pub struct PmFixtureReadOwnerGrant {
    owner_id: PmFixtureOwnerId,
}

impl PmFixtureReadOwnerGrant {
    #[must_use]
    pub fn allocate() -> Self {
        static NEXT_OWNER_ID: AtomicU64 = AtomicU64::new(1);
        let value = NEXT_OWNER_ID
            .fetch_update(
                AtomicOrdering::Relaxed,
                AtomicOrdering::Relaxed,
                |current| current.checked_add(1),
            )
            .expect("process exhausted all nonzero fixture read-owner identities");
        Self {
            owner_id: PmFixtureOwnerId(
                NonZeroU64::new(value).expect("fixture owner sequence starts at one"),
            ),
        }
    }

    /// Consumes the owner authority and issues each mandatory read-role
    /// constructor grant exactly once.
    #[must_use]
    pub fn split(
        self,
    ) -> (
        PmFixturePrivateRoleGrant,
        PmFixtureReconciliationRoleGrant,
        PmFixtureAccountRoleGrant,
    ) {
        (
            PmFixturePrivateRoleGrant {
                owner_id: self.owner_id,
            },
            PmFixtureReconciliationRoleGrant {
                owner_id: self.owner_id,
            },
            PmFixtureAccountRoleGrant {
                owner_id: self.owner_id,
            },
        )
    }
}

macro_rules! define_role_grant {
    ($name:ident) => {
        pub struct $name {
            owner_id: PmFixtureOwnerId,
        }

        impl $name {
            pub(crate) const fn into_owner_id(self) -> PmFixtureOwnerId {
                self.owner_id
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "(<opaque>)"))
            }
        }
    };
}

define_role_grant!(PmFixturePrivateRoleGrant);
define_role_grant!(PmFixtureReconciliationRoleGrant);
define_role_grant!(PmFixtureAccountRoleGrant);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmFixtureInstrumentScope {
    handle: PmInstrumentHandle,
    metadata: PmMarketMetadata,
    trading_domain: PmGoalFTradingDomain,
}

impl PmFixtureInstrumentScope {
    pub fn from_metadata(
        handle: PmInstrumentHandle,
        metadata: PmMarketMetadata,
    ) -> Result<Self, PmMetadataError> {
        let trading_domain = PmGoalFTradingDomain::from_metadata(metadata)?;
        Ok(Self {
            handle,
            metadata,
            trading_domain,
        })
    }

    #[must_use]
    pub const fn handle(self) -> PmInstrumentHandle {
        self.handle
    }

    #[must_use]
    pub const fn id(self) -> PmInstrumentId {
        self.trading_domain.instrument()
    }

    #[must_use]
    pub const fn metadata(self) -> PmMarketMetadata {
        self.metadata
    }

    #[must_use]
    pub const fn trading_domain(self) -> PmGoalFTradingDomain {
        self.trading_domain
    }

    #[must_use]
    pub const fn tick(self) -> PmTick {
        self.metadata.tick()
    }

    #[must_use]
    pub const fn minimum_order_size(self) -> PmQuantity {
        self.metadata.minimum_order_size()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmFixtureScopeError {
    #[error("fixture role requires a Polymarket account source")]
    WrongSource,
    #[error("fixture role source belongs to another account")]
    SourceAccountMismatch,
}

pub(crate) fn validate_account_source(
    account_scope: PmAccountScope,
    source: PmProductSource,
) -> Result<(), PmFixtureScopeError> {
    match source {
        PmProductSource::PolymarketAccount { account, .. } if account == account_scope.handle() => {
            Ok(())
        }
        PmProductSource::PolymarketAccount { .. } => {
            Err(PmFixtureScopeError::SourceAccountMismatch)
        }
        PmProductSource::OkxReference { .. } | PmProductSource::PolymarketMarket { .. } => {
            Err(PmFixtureScopeError::WrongSource)
        }
    }
}
