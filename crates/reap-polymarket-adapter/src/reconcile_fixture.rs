use reap_pm_core::{
    PmAccountHandle, PmAccountScope, PmConnectionId, PmFillEvent, PmOrderEvent, PmProductSource,
    PmSnapshotCompleteness, PmSnapshotEvidence, PmSourceBound,
};
use thiserror::Error;

pub const MAX_PM_RECONCILIATION_ORDERS: usize = 1_024;
pub const MAX_PM_RECONCILIATION_FILLS: usize = 8_192;

mod sealed {
    pub trait Sealed {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmReconciliationContractError {
    #[error("reconciliation output requires a Polymarket account source")]
    WrongSource,
    #[error("reconciliation source belongs to another account")]
    SourceAccountMismatch,
    #[error("reconciliation output must carry complete snapshot evidence")]
    IncompleteSnapshot,
    #[error("reconciliation output exceeds its fixed order bound")]
    TooManyOrders,
    #[error("reconciliation output exceeds its fixed fill bound")]
    TooManyFills,
    #[error("reconciled event belongs to another account")]
    EventAccountMismatch,
    #[error("reconciled event belongs to another source")]
    EventSourceMismatch,
    #[error("fill watermark belongs to another account")]
    WatermarkAccountMismatch,
}

/// Account-scoped opaque cursor from the pinned reconciliation fixture.
///
/// No ordering or last-fill relationship is inferred from its bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmFixtureFillWatermark {
    account_scope: PmAccountScope,
    cursor: [u8; 32],
}

impl PmFixtureFillWatermark {
    #[must_use]
    pub const fn new(account_scope: PmAccountScope, cursor: [u8; 32]) -> Self {
        Self {
            account_scope,
            cursor,
        }
    }

    #[must_use]
    pub const fn account_scope(self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn cursor(self) -> [u8; 32] {
        self.cursor
    }
}

/// Atomic complete open-order replacement evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmCompleteOpenOrdersSnapshot {
    source: PmProductSource,
    account_scope: PmAccountScope,
    snapshot: PmSnapshotEvidence,
    orders: Vec<PmOrderEvent>,
}

impl PmCompleteOpenOrdersSnapshot {
    pub fn new(
        source: PmProductSource,
        account_scope: PmAccountScope,
        snapshot: PmSnapshotEvidence,
        orders: Vec<PmOrderEvent>,
    ) -> Result<Self, PmReconciliationContractError> {
        validate_source(source, account_scope)?;
        validate_complete(snapshot)?;
        if orders.len() > MAX_PM_RECONCILIATION_ORDERS {
            return Err(PmReconciliationContractError::TooManyOrders);
        }
        validate_orders(source, account_scope, &orders)?;
        Ok(Self {
            source,
            account_scope,
            snapshot,
            orders,
        })
    }

    #[must_use]
    pub const fn account_scope(&self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn snapshot(&self) -> PmSnapshotEvidence {
        self.snapshot
    }

    #[must_use]
    pub fn orders(&self) -> &[PmOrderEvent] {
        &self.orders
    }
}

impl PmSourceBound for PmCompleteOpenOrdersSnapshot {
    fn source(&self) -> PmProductSource {
        self.source
    }
}

/// Exact detail for one known order, distinct from a complete open-order set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmExactOrderDetail {
    source: PmProductSource,
    account_scope: PmAccountScope,
    order: PmOrderEvent,
}

impl PmExactOrderDetail {
    pub fn new(
        source: PmProductSource,
        account_scope: PmAccountScope,
        order: PmOrderEvent,
    ) -> Result<Self, PmReconciliationContractError> {
        validate_source(source, account_scope)?;
        validate_orders(source, account_scope, &[order])?;
        Ok(Self {
            source,
            account_scope,
            order,
        })
    }

    #[must_use]
    pub const fn account_scope(self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn order(self) -> PmOrderEvent {
        self.order
    }
}

impl PmSourceBound for PmExactOrderDetail {
    fn source(&self) -> PmProductSource {
        self.source
    }
}

/// Complete fill page with the exact requested and resulting watermarks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmCompleteFillPage {
    source: PmProductSource,
    account_scope: PmAccountScope,
    snapshot: PmSnapshotEvidence,
    requested_after: Option<PmFixtureFillWatermark>,
    next_after: Option<PmFixtureFillWatermark>,
    fills: Vec<PmFillEvent>,
}

impl PmCompleteFillPage {
    pub fn new(
        source: PmProductSource,
        account_scope: PmAccountScope,
        snapshot: PmSnapshotEvidence,
        requested_after: Option<PmFixtureFillWatermark>,
        next_after: Option<PmFixtureFillWatermark>,
        fills: Vec<PmFillEvent>,
    ) -> Result<Self, PmReconciliationContractError> {
        validate_source(source, account_scope)?;
        validate_complete(snapshot)?;
        if fills.len() > MAX_PM_RECONCILIATION_FILLS {
            return Err(PmReconciliationContractError::TooManyFills);
        }
        for watermark in [requested_after, next_after].into_iter().flatten() {
            if watermark.account_scope() != account_scope {
                return Err(PmReconciliationContractError::WatermarkAccountMismatch);
            }
        }
        for fill in &fills {
            if fill.account() != account_scope.handle() {
                return Err(PmReconciliationContractError::EventAccountMismatch);
            }
            if fill.source() != source {
                return Err(PmReconciliationContractError::EventSourceMismatch);
            }
        }
        Ok(Self {
            source,
            account_scope,
            snapshot,
            requested_after,
            next_after,
            fills,
        })
    }

    #[must_use]
    pub const fn account_scope(&self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn snapshot(&self) -> PmSnapshotEvidence {
        self.snapshot
    }

    #[must_use]
    pub const fn requested_after(&self) -> Option<PmFixtureFillWatermark> {
        self.requested_after
    }

    #[must_use]
    pub const fn next_after(&self) -> Option<PmFixtureFillWatermark> {
        self.next_after
    }

    #[must_use]
    pub fn fills(&self) -> &[PmFillEvent] {
        &self.fills
    }
}

impl PmSourceBound for PmCompleteFillPage {
    fn source(&self) -> PmProductSource {
        self.source
    }
}

/// Fixture-only, read-only order reconciliation capability.
pub trait PmReconciliationRole: sealed::Sealed {
    type CompleteOpenOrders;
    type ExactOrderDetail;
    type CompleteFillPage;

    fn account_scope(&self) -> PmAccountScope;
    fn account(&self) -> PmAccountHandle;
    fn source(&self) -> PmProductSource;
    fn connection(&self) -> PmConnectionId;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmFixtureReconciliation {
    account_scope: PmAccountScope,
    source: PmProductSource,
    connection: PmConnectionId,
}

impl PmFixtureReconciliation {
    pub fn new(
        account_scope: PmAccountScope,
        source: PmProductSource,
        connection: PmConnectionId,
    ) -> Result<Self, PmReconciliationContractError> {
        validate_source(source, account_scope)?;
        Ok(Self {
            account_scope,
            source,
            connection,
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
    pub const fn source(&self) -> PmProductSource {
        self.source
    }

    #[must_use]
    pub const fn connection(&self) -> PmConnectionId {
        self.connection
    }
}

impl sealed::Sealed for PmFixtureReconciliation {}

impl PmReconciliationRole for PmFixtureReconciliation {
    type CompleteOpenOrders = PmCompleteOpenOrdersSnapshot;
    type ExactOrderDetail = PmExactOrderDetail;
    type CompleteFillPage = PmCompleteFillPage;

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

fn validate_source(
    source: PmProductSource,
    account_scope: PmAccountScope,
) -> Result<(), PmReconciliationContractError> {
    match source {
        PmProductSource::PolymarketAccount { account, .. } if account == account_scope.handle() => {
            Ok(())
        }
        PmProductSource::PolymarketAccount { .. } => {
            Err(PmReconciliationContractError::SourceAccountMismatch)
        }
        PmProductSource::OkxReference { .. } | PmProductSource::PolymarketMarket { .. } => {
            Err(PmReconciliationContractError::WrongSource)
        }
    }
}

fn validate_complete(snapshot: PmSnapshotEvidence) -> Result<(), PmReconciliationContractError> {
    if snapshot.completeness() == PmSnapshotCompleteness::Complete {
        Ok(())
    } else {
        Err(PmReconciliationContractError::IncompleteSnapshot)
    }
}

fn validate_orders(
    source: PmProductSource,
    account_scope: PmAccountScope,
    orders: &[PmOrderEvent],
) -> Result<(), PmReconciliationContractError> {
    for order in orders {
        if order.account() != account_scope.handle() {
            return Err(PmReconciliationContractError::EventAccountMismatch);
        }
        if order.source() != source {
            return Err(PmReconciliationContractError::EventSourceMismatch);
        }
    }
    Ok(())
}
