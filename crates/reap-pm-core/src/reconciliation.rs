use thiserror::Error;

use crate::event::{
    PmAllowanceEvent, PmBalanceEvent, PmFillEvent, PmOrderEvent, PmPositionEvent,
    PmSnapshotEvidence,
};
use crate::identity::{
    IngressSequence, PmAccountScope, PmAssetId, PmInstrumentHandle, PmProductSource, PmSourceBound,
    PmSpenderId, PmVenueOrderKey,
};
use crate::metadata::MAX_REQUIRED_SPENDERS;

pub const MAX_PM_RECONCILIATION_ORDERS: usize = 1_024;
pub const MAX_PM_RECONCILIATION_FILLS: usize = 8_192;
pub const MAX_PM_ACCOUNT_EXPECTED_ASSETS: usize = MAX_REQUIRED_SPENDERS;
pub const MAX_PM_ACCOUNT_EXPECTED_SPENDERS: usize = MAX_REQUIRED_SPENDERS;
pub const MAX_PM_ACCOUNT_EXPECTED_INSTRUMENTS: usize = MAX_REQUIRED_SPENDERS;
/// Bounded diagnostic rows retained in addition to a fully populated expected
/// scope. They are observable but never become authority through an
/// `expected_*` lookup.
pub const MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS: usize = MAX_REQUIRED_SPENDERS;
pub const MAX_PM_ACCOUNT_SNAPSHOT_ROWS: usize =
    MAX_REQUIRED_SPENDERS + MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmAggregateError {
    #[error("reconciliation request sequence must be nonzero")]
    ZeroRequestSequence,
    #[error("reconciliation completion sequence must be nonzero")]
    ZeroCompletionSequence,
    #[error("reconciliation completion must follow its exact request")]
    CompletionDoesNotFollowRequest,
    #[error("complete aggregate requires a Polymarket account source")]
    WrongSource,
    #[error("complete aggregate source belongs to another account")]
    SourceAccountMismatch,
    #[error("complete open-order snapshot exceeds its fixed bound")]
    TooManyOrders,
    #[error("complete fill query exceeds its fixed bound")]
    TooManyFills,
    #[error("complete account snapshot exceeds the expected-asset bound")]
    TooManyExpectedAssets,
    #[error("complete account snapshot exceeds the required-spender bound")]
    TooManyExpectedSpenders,
    #[error("complete account snapshot exceeds the expected-instrument bound")]
    TooManyExpectedInstruments,
    #[error("complete account snapshot exceeds its expected plus diagnostic balance-row bound")]
    TooManyBalanceRows,
    #[error("complete account snapshot exceeds its expected plus diagnostic allowance-row bound")]
    TooManyAllowanceRows,
    #[error("complete account snapshot exceeds its expected plus diagnostic position-row bound")]
    TooManyPositionRows,
    #[error("aggregate row belongs to another account")]
    RowAccountMismatch,
    #[error("aggregate row belongs to another source")]
    RowSourceMismatch,
    #[error("aggregate row carries another snapshot revision")]
    RowRevisionMismatch,
    #[error("open-order snapshot contains a duplicate client or venue order key")]
    DuplicateOrderKey,
    #[error("open-order snapshot row lacks an exact venue-order identity")]
    MissingOpenOrderVenueKey,
    #[error("fill query contains a duplicate exact fill leg")]
    DuplicateFillKey,
    #[error("requested order detail belongs to another account")]
    RequestedOrderAccountMismatch,
    #[error("order-detail row lacks the requested venue-order identity")]
    MissingOrderDetailVenueKey,
    #[error("order-detail row names another venue-order identity")]
    OrderDetailVenueMismatch,
    #[error("fill-query cursor belongs to another full account scope")]
    CursorAccountScopeMismatch,
    #[error("expected asset occurs more than once")]
    DuplicateExpectedAsset,
    #[error("expected spender occurs more than once")]
    DuplicateExpectedSpender,
    #[error("expected instrument occurs more than once")]
    DuplicateExpectedInstrument,
    #[error("expected spender belongs to another account")]
    ExpectedSpenderAccountMismatch,
    #[error("expected spender belongs to another chain")]
    ExpectedSpenderChainMismatch,
    #[error("expected spender asset is absent from the expected asset scope")]
    ExpectedSpenderAssetNotExpected,
    #[error("balance asset occurs more than once")]
    DuplicateBalanceAsset,
    #[error("allowance spender occurs more than once")]
    DuplicateAllowanceSpender,
    #[error("position instrument occurs more than once")]
    DuplicatePositionInstrument,
    #[error("allowance row belongs to another chain")]
    AllowanceSpenderChainMismatch,
}

/// Exact owner-loop occurrence boundary linking one complete result to the
/// reconciliation request that caused it.
///
/// These are local causal sequences, not venue cursors and not venue event
/// ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmReconciliationRequestBoundary {
    request_sequence: IngressSequence,
    completion_sequence: IngressSequence,
}

impl PmReconciliationRequestBoundary {
    pub fn new(
        request_sequence: IngressSequence,
        completion_sequence: IngressSequence,
    ) -> Result<Self, PmAggregateError> {
        if request_sequence.value() == 0 {
            return Err(PmAggregateError::ZeroRequestSequence);
        }
        if completion_sequence.value() == 0 {
            return Err(PmAggregateError::ZeroCompletionSequence);
        }
        if completion_sequence <= request_sequence {
            return Err(PmAggregateError::CompletionDoesNotFollowRequest);
        }
        Ok(Self {
            request_sequence,
            completion_sequence,
        })
    }

    #[must_use]
    pub const fn request_sequence(self) -> IngressSequence {
        self.request_sequence
    }

    #[must_use]
    pub const fn completion_sequence(self) -> IngressSequence {
        self.completion_sequence
    }
}

/// Atomic replacement of the complete open-order query result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmCompleteOpenOrdersSnapshot {
    source: PmProductSource,
    account_scope: PmAccountScope,
    snapshot: PmSnapshotEvidence,
    boundary: PmReconciliationRequestBoundary,
    orders: Box<[PmOrderEvent]>,
}

impl PmCompleteOpenOrdersSnapshot {
    pub fn new(
        source: PmProductSource,
        account_scope: PmAccountScope,
        snapshot: PmSnapshotEvidence,
        boundary: PmReconciliationRequestBoundary,
        orders: Box<[PmOrderEvent]>,
    ) -> Result<Self, PmAggregateError> {
        validate_source(source, account_scope)?;
        if orders.len() > MAX_PM_RECONCILIATION_ORDERS {
            return Err(PmAggregateError::TooManyOrders);
        }
        for order in &orders {
            validate_row(source, account_scope, order.source(), order.account())?;
            if order.order().venue_order_key().is_none() {
                return Err(PmAggregateError::MissingOpenOrderVenueKey);
            }
        }
        if has_overlapping_order_keys(&orders) {
            return Err(PmAggregateError::DuplicateOrderKey);
        }
        Ok(Self {
            source,
            account_scope,
            snapshot,
            boundary,
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
    pub const fn boundary(&self) -> PmReconciliationRequestBoundary {
        self.boundary
    }

    #[must_use]
    pub fn orders(&self) -> &[PmOrderEvent] {
        &self.orders
    }

    #[must_use]
    pub fn into_orders(self) -> Box<[PmOrderEvent]> {
        self.orders
    }
}

impl PmSourceBound for PmCompleteOpenOrdersSnapshot {
    fn source(&self) -> PmProductSource {
        self.source
    }
}

/// Complete answer for one exact requested venue order.
///
/// `None` is an explicit absent result from the complete detail query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmExactOrderDetail {
    source: PmProductSource,
    account_scope: PmAccountScope,
    snapshot: PmSnapshotEvidence,
    boundary: PmReconciliationRequestBoundary,
    requested_order: PmVenueOrderKey,
    order: Option<PmOrderEvent>,
}

impl PmExactOrderDetail {
    pub fn new(
        source: PmProductSource,
        account_scope: PmAccountScope,
        snapshot: PmSnapshotEvidence,
        boundary: PmReconciliationRequestBoundary,
        requested_order: PmVenueOrderKey,
        order: Option<PmOrderEvent>,
    ) -> Result<Self, PmAggregateError> {
        validate_source(source, account_scope)?;
        if requested_order.account() != account_scope.handle() {
            return Err(PmAggregateError::RequestedOrderAccountMismatch);
        }
        if let Some(order) = order {
            validate_row(source, account_scope, order.source(), order.account())?;
            let observed = order
                .order()
                .venue_order_key()
                .ok_or(PmAggregateError::MissingOrderDetailVenueKey)?;
            if observed != requested_order {
                return Err(PmAggregateError::OrderDetailVenueMismatch);
            }
        }
        Ok(Self {
            source,
            account_scope,
            snapshot,
            boundary,
            requested_order,
            order,
        })
    }

    #[must_use]
    pub const fn account_scope(self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn snapshot(self) -> PmSnapshotEvidence {
        self.snapshot
    }

    #[must_use]
    pub const fn boundary(self) -> PmReconciliationRequestBoundary {
        self.boundary
    }

    #[must_use]
    pub const fn requested_order(self) -> PmVenueOrderKey {
        self.requested_order
    }

    #[must_use]
    pub const fn order(self) -> Option<PmOrderEvent> {
        self.order
    }
}

impl PmSourceBound for PmExactOrderDetail {
    fn source(&self) -> PmProductSource {
        self.source
    }
}

/// Full-account-scoped opaque fill-query cursor.
///
/// Its bytes have equality semantics only. No ordering or last-fill
/// relationship is inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmFillQueryCursor {
    account_scope: PmAccountScope,
    opaque: [u8; 32],
}

impl PmFillQueryCursor {
    #[must_use]
    pub const fn new(account_scope: PmAccountScope, opaque: [u8; 32]) -> Self {
        Self {
            account_scope,
            opaque,
        }
    }

    #[must_use]
    pub const fn account_scope(self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn opaque(self) -> [u8; 32] {
        self.opaque
    }
}

/// Terminal result of one complete bounded cursor query.
///
/// This is deliberately not a page type. An adapter may construct it only
/// after consuming the complete cursor chain through its terminal page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmCompleteFillQuery {
    source: PmProductSource,
    account_scope: PmAccountScope,
    snapshot: PmSnapshotEvidence,
    boundary: PmReconciliationRequestBoundary,
    requested_after: Option<PmFillQueryCursor>,
    resulting_watermark: PmFillQueryCursor,
    fills: Box<[PmFillEvent]>,
}

impl PmCompleteFillQuery {
    #[allow(
        clippy::too_many_arguments,
        reason = "the whole-query carrier keeps its exact causal and cursor boundaries explicit"
    )]
    pub fn new(
        source: PmProductSource,
        account_scope: PmAccountScope,
        snapshot: PmSnapshotEvidence,
        boundary: PmReconciliationRequestBoundary,
        requested_after: Option<PmFillQueryCursor>,
        resulting_watermark: PmFillQueryCursor,
        fills: Box<[PmFillEvent]>,
    ) -> Result<Self, PmAggregateError> {
        validate_source(source, account_scope)?;
        if fills.len() > MAX_PM_RECONCILIATION_FILLS {
            return Err(PmAggregateError::TooManyFills);
        }
        for cursor in requested_after.into_iter().chain([resulting_watermark]) {
            if cursor.account_scope() != account_scope {
                return Err(PmAggregateError::CursorAccountScopeMismatch);
            }
        }
        for fill in &fills {
            validate_row(source, account_scope, fill.source(), fill.account())?;
        }
        if has_duplicate_fill_keys(&fills) {
            return Err(PmAggregateError::DuplicateFillKey);
        }
        Ok(Self {
            source,
            account_scope,
            snapshot,
            boundary,
            requested_after,
            resulting_watermark,
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
    pub const fn boundary(&self) -> PmReconciliationRequestBoundary {
        self.boundary
    }

    #[must_use]
    pub const fn requested_after(&self) -> Option<PmFillQueryCursor> {
        self.requested_after
    }

    #[must_use]
    pub const fn resulting_watermark(&self) -> PmFillQueryCursor {
        self.resulting_watermark
    }

    #[must_use]
    pub fn fills(&self) -> &[PmFillEvent] {
        &self.fills
    }

    #[must_use]
    pub fn into_fills(self) -> Box<[PmFillEvent]> {
        self.fills
    }
}

impl PmSourceBound for PmCompleteFillQuery {
    fn source(&self) -> PmProductSource {
        self.source
    }
}

/// One atomic, complete account/allowance/position result.
///
/// Missing expected rows are explicit absence because the containing result is
/// complete. Rows outside the expected scopes are retained within the fixed
/// bound for diagnosis but the `expected_*` accessors never authorize them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmCompleteAccountSnapshot {
    source: PmProductSource,
    account_scope: PmAccountScope,
    snapshot: PmSnapshotEvidence,
    boundary: PmReconciliationRequestBoundary,
    expected_assets: Box<[PmAssetId]>,
    expected_spenders: Box<[PmSpenderId]>,
    expected_instruments: Box<[PmInstrumentHandle]>,
    balances: Box<[PmBalanceEvent]>,
    allowances: Box<[PmAllowanceEvent]>,
    positions: Box<[PmPositionEvent]>,
}

impl PmCompleteAccountSnapshot {
    #[allow(
        clippy::too_many_arguments,
        reason = "atomic account completeness keeps all expected scopes and row families together"
    )]
    pub fn new(
        source: PmProductSource,
        account_scope: PmAccountScope,
        snapshot: PmSnapshotEvidence,
        boundary: PmReconciliationRequestBoundary,
        expected_assets: Box<[PmAssetId]>,
        expected_spenders: Box<[PmSpenderId]>,
        expected_instruments: Box<[PmInstrumentHandle]>,
        balances: Box<[PmBalanceEvent]>,
        allowances: Box<[PmAllowanceEvent]>,
        positions: Box<[PmPositionEvent]>,
    ) -> Result<Self, PmAggregateError> {
        validate_source(source, account_scope)?;
        validate_expected_scopes(
            account_scope,
            &expected_assets,
            &expected_spenders,
            &expected_instruments,
        )?;
        validate_balance_rows(source, account_scope, snapshot, &expected_assets, &balances)?;
        validate_allowance_rows(
            source,
            account_scope,
            snapshot,
            &expected_spenders,
            &allowances,
        )?;
        validate_position_rows(
            source,
            account_scope,
            snapshot,
            &expected_instruments,
            &positions,
        )?;
        Ok(Self {
            source,
            account_scope,
            snapshot,
            boundary,
            expected_assets,
            expected_spenders,
            expected_instruments,
            balances,
            allowances,
            positions,
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
    pub const fn boundary(&self) -> PmReconciliationRequestBoundary {
        self.boundary
    }

    #[must_use]
    pub fn expected_assets(&self) -> &[PmAssetId] {
        &self.expected_assets
    }

    #[must_use]
    pub fn expected_spenders(&self) -> &[PmSpenderId] {
        &self.expected_spenders
    }

    #[must_use]
    pub fn expected_instruments(&self) -> &[PmInstrumentHandle] {
        &self.expected_instruments
    }

    #[must_use]
    pub fn balances(&self) -> &[PmBalanceEvent] {
        &self.balances
    }

    #[must_use]
    pub fn allowances(&self) -> &[PmAllowanceEvent] {
        &self.allowances
    }

    #[must_use]
    pub fn positions(&self) -> &[PmPositionEvent] {
        &self.positions
    }

    /// `None` means the asset was not configured. `Some(None)` is explicit
    /// absence in this complete snapshot.
    #[must_use]
    pub fn expected_balance(&self, asset: PmAssetId) -> Option<Option<&PmBalanceEvent>> {
        self.expected_assets
            .contains(&asset)
            .then(|| self.balances.iter().find(|row| row.asset() == asset))
    }

    /// `None` means the spender was not configured. `Some(None)` is explicit
    /// absence in this complete snapshot.
    #[must_use]
    pub fn expected_allowance(&self, spender: PmSpenderId) -> Option<Option<&PmAllowanceEvent>> {
        self.expected_spenders
            .contains(&spender)
            .then(|| self.allowances.iter().find(|row| row.spender() == spender))
    }

    /// `None` means the instrument was not configured. `Some(None)` is
    /// explicit absence in this complete snapshot.
    #[must_use]
    pub fn expected_position(
        &self,
        instrument: PmInstrumentHandle,
    ) -> Option<Option<&PmPositionEvent>> {
        self.expected_instruments.contains(&instrument).then(|| {
            self.positions
                .iter()
                .find(|row| row.instrument() == instrument)
        })
    }
}

impl PmSourceBound for PmCompleteAccountSnapshot {
    fn source(&self) -> PmProductSource {
        self.source
    }
}

fn validate_source(
    source: PmProductSource,
    account_scope: PmAccountScope,
) -> Result<(), PmAggregateError> {
    match source {
        PmProductSource::PolymarketAccount { account, .. } if account == account_scope.handle() => {
            Ok(())
        }
        PmProductSource::PolymarketAccount { .. } => Err(PmAggregateError::SourceAccountMismatch),
        PmProductSource::OkxReference { .. } | PmProductSource::PolymarketMarket { .. } => {
            Err(PmAggregateError::WrongSource)
        }
    }
}

fn validate_row(
    expected_source: PmProductSource,
    account_scope: PmAccountScope,
    actual_source: PmProductSource,
    actual_account: crate::identity::PmAccountHandle,
) -> Result<(), PmAggregateError> {
    if actual_account != account_scope.handle() {
        return Err(PmAggregateError::RowAccountMismatch);
    }
    if actual_source != expected_source {
        return Err(PmAggregateError::RowSourceMismatch);
    }
    Ok(())
}

fn has_overlapping_order_keys(orders: &[PmOrderEvent]) -> bool {
    let mut client_keys = Vec::with_capacity(orders.len());
    let mut venue_keys = Vec::with_capacity(orders.len());
    for order in orders {
        let identity = order.order();
        client_keys.extend(identity.client_order_key());
        venue_keys.extend(identity.venue_order_key());
    }
    client_keys.sort_unstable();
    venue_keys.sort_unstable();
    client_keys.windows(2).any(|pair| pair[0] == pair[1])
        || venue_keys.windows(2).any(|pair| pair[0] == pair[1])
}

fn has_duplicate_fill_keys(fills: &[PmFillEvent]) -> bool {
    let mut keys = Vec::with_capacity(fills.len());
    keys.extend(fills.iter().map(|fill| fill.fill_key()));
    keys.sort_unstable();
    keys.windows(2).any(|pair| pair[0] == pair[1])
}

fn has_duplicate<T: Eq>(values: &[T]) -> bool {
    values
        .iter()
        .enumerate()
        .any(|(index, value)| values[..index].contains(value))
}

fn validate_expected_scopes(
    account_scope: PmAccountScope,
    assets: &[PmAssetId],
    spenders: &[PmSpenderId],
    instruments: &[PmInstrumentHandle],
) -> Result<(), PmAggregateError> {
    if assets.len() > MAX_PM_ACCOUNT_EXPECTED_ASSETS {
        return Err(PmAggregateError::TooManyExpectedAssets);
    }
    if spenders.len() > MAX_PM_ACCOUNT_EXPECTED_SPENDERS {
        return Err(PmAggregateError::TooManyExpectedSpenders);
    }
    if instruments.len() > MAX_PM_ACCOUNT_EXPECTED_INSTRUMENTS {
        return Err(PmAggregateError::TooManyExpectedInstruments);
    }
    if has_duplicate(assets) {
        return Err(PmAggregateError::DuplicateExpectedAsset);
    }
    if has_duplicate(spenders) {
        return Err(PmAggregateError::DuplicateExpectedSpender);
    }
    if has_duplicate(instruments) {
        return Err(PmAggregateError::DuplicateExpectedInstrument);
    }
    for spender in spenders {
        if spender.account() != account_scope.handle() {
            return Err(PmAggregateError::ExpectedSpenderAccountMismatch);
        }
        if spender.requirement().chain() != account_scope.chain() {
            return Err(PmAggregateError::ExpectedSpenderChainMismatch);
        }
        if !assets.contains(&spender.requirement().asset()) {
            return Err(PmAggregateError::ExpectedSpenderAssetNotExpected);
        }
    }
    Ok(())
}

fn validate_balance_rows(
    source: PmProductSource,
    account_scope: PmAccountScope,
    snapshot: PmSnapshotEvidence,
    expected_assets: &[PmAssetId],
    balances: &[PmBalanceEvent],
) -> Result<(), PmAggregateError> {
    if balances.len() > expected_assets.len() + MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS
        || balances
            .iter()
            .filter(|row| !expected_assets.contains(&row.asset()))
            .count()
            > MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS
    {
        return Err(PmAggregateError::TooManyBalanceRows);
    }
    for (index, row) in balances.iter().enumerate() {
        validate_row(source, account_scope, row.source(), row.account())?;
        validate_row_revision(snapshot, row.snapshot())?;
        if balances[..index]
            .iter()
            .any(|prior| prior.asset() == row.asset())
        {
            return Err(PmAggregateError::DuplicateBalanceAsset);
        }
    }
    Ok(())
}

fn validate_allowance_rows(
    source: PmProductSource,
    account_scope: PmAccountScope,
    snapshot: PmSnapshotEvidence,
    expected_spenders: &[PmSpenderId],
    allowances: &[PmAllowanceEvent],
) -> Result<(), PmAggregateError> {
    if allowances.len() > expected_spenders.len() + MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS
        || allowances
            .iter()
            .filter(|row| !expected_spenders.contains(&row.spender()))
            .count()
            > MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS
    {
        return Err(PmAggregateError::TooManyAllowanceRows);
    }
    for (index, row) in allowances.iter().enumerate() {
        validate_row(source, account_scope, row.source(), row.account())?;
        validate_row_revision(snapshot, row.snapshot())?;
        if row.spender().requirement().chain() != account_scope.chain() {
            return Err(PmAggregateError::AllowanceSpenderChainMismatch);
        }
        if allowances[..index]
            .iter()
            .any(|prior| prior.spender() == row.spender())
        {
            return Err(PmAggregateError::DuplicateAllowanceSpender);
        }
    }
    Ok(())
}

fn validate_position_rows(
    source: PmProductSource,
    account_scope: PmAccountScope,
    snapshot: PmSnapshotEvidence,
    expected_instruments: &[PmInstrumentHandle],
    positions: &[PmPositionEvent],
) -> Result<(), PmAggregateError> {
    if positions.len() > expected_instruments.len() + MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS
        || positions
            .iter()
            .filter(|row| !expected_instruments.contains(&row.instrument()))
            .count()
            > MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS
    {
        return Err(PmAggregateError::TooManyPositionRows);
    }
    for (index, row) in positions.iter().enumerate() {
        validate_row(source, account_scope, row.source(), row.account())?;
        validate_row_revision(snapshot, row.snapshot())?;
        if positions[..index]
            .iter()
            .any(|prior| prior.instrument() == row.instrument())
        {
            return Err(PmAggregateError::DuplicatePositionInstrument);
        }
    }
    Ok(())
}

fn validate_row_revision(
    expected: PmSnapshotEvidence,
    actual: PmSnapshotEvidence,
) -> Result<(), PmAggregateError> {
    if actual == expected {
        Ok(())
    } else {
        Err(PmAggregateError::RowRevisionMismatch)
    }
}
