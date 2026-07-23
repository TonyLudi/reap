use thiserror::Error;

use crate::identity::{
    OkxReferenceHandle, PmAccountHandle, PmAssetId, PmClientOrderKey, PmFillKey,
    PmInstrumentHandle, PmProductSource, PmSourceBound, PmSpenderId, PmVenueOrderKey,
    SnapshotRevision,
};
use crate::metadata::PmMarketMetadata;
use crate::numeric::{
    OkxReferencePrice, PmBookQuantity, PmErc1155OperatorApproval, PmOrderSide, PmPrice, PmQuantity,
    PmSignedUnits, PmTick, U256,
};

pub const MAX_PM_BOOK_LEVELS: u16 = 2_048;
pub const MAX_PM_VENUE_CHANGE_HASH_BYTES: usize = 96;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmEventError {
    #[error("event requires a Polymarket market source")]
    WrongMarketSource,
    #[error("Polymarket market source token does not match the event instrument")]
    MarketSourceTokenMismatch,
    #[error("event requires a Polymarket account source")]
    WrongAccountSource,
    #[error("Polymarket account source does not match the event account")]
    AccountSourceMismatch,
    #[error("event revision must be nonzero")]
    ZeroRevision,
    #[error("snapshot levels cannot carry the delete-level representation")]
    SnapshotLevelIsDelete,
    #[error("book snapshot exceeds the fixed normalized level bound")]
    TooManyBookLevels,
    #[error("venue change hash must not be empty")]
    EmptyVenueChangeHash,
    #[error("venue change hash exceeds the fixed normalized byte bound")]
    VenueChangeHashTooLong,
    #[error("venue change hash must contain only ASCII bytes")]
    NonAsciiVenueChangeHash,
    #[error("delta level and venue change-hash counts do not match")]
    ChangeHashCountMismatch,
    #[error("best bid is not strictly below best ask")]
    CrossedBookTop,
    #[error("order identity must contain a client or venue order identifier")]
    MissingOrderIdentity,
    #[error("client and venue order identifiers belong to different accounts")]
    OrderIdentityAccountMismatch,
    #[error("fill requires an exact venue-order identity on its order")]
    MissingFillVenueOrderIdentity,
    #[error("fill key and order identity name different venue-order legs")]
    FillVenueOrderMismatch,
    #[error("cumulative fill exceeds original order quantity")]
    CumulativeFillExceedsOriginal,
    #[error("order status is inconsistent with cumulative fill")]
    OrderStatusFillMismatch,
    #[error("allowance kind does not match the exact spender asset")]
    AllowanceAssetKindMismatch,
}

/// A metadata observation bound to its configured market and outcome handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmMarketEvent {
    source: PmProductSource,
    instrument: PmInstrumentHandle,
    metadata_revision: SnapshotRevision,
    metadata: PmMarketMetadata,
}

impl PmMarketEvent {
    pub fn new(
        source: PmProductSource,
        instrument: PmInstrumentHandle,
        metadata_revision: SnapshotRevision,
        metadata: PmMarketMetadata,
    ) -> Result<Self, PmEventError> {
        validate_market_source(source, instrument)?;
        validate_revision(metadata_revision)?;
        Ok(Self {
            source,
            instrument,
            metadata_revision,
            metadata,
        })
    }

    #[must_use]
    pub const fn source(self) -> PmProductSource {
        self.source
    }

    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn metadata_revision(self) -> SnapshotRevision {
        self.metadata_revision
    }

    #[must_use]
    pub const fn metadata(self) -> PmMarketMetadata {
        self.metadata
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PmBookSide {
    Bid,
    Ask,
}

/// One exact price level. Zero quantity is represented only by
/// `PmBookQuantity::Delete`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmBookLevel {
    side: PmBookSide,
    price: PmPrice,
    quantity: PmBookQuantity,
}

impl PmBookLevel {
    #[must_use]
    pub const fn new(side: PmBookSide, price: PmPrice, quantity: PmBookQuantity) -> Self {
        Self {
            side,
            price,
            quantity,
        }
    }

    #[must_use]
    pub const fn side(self) -> PmBookSide {
        self.side
    }

    #[must_use]
    pub const fn price(self) -> PmPrice {
        self.price
    }

    #[must_use]
    pub const fn quantity(self) -> PmBookQuantity {
        self.quantity
    }
}

/// Exact bounded venue evidence attached to one public price change.
///
/// Polymarket calls this field `hash`, but observed values are not guaranteed
/// to be a fixed-width cryptographic digest. The normalized boundary therefore
/// preserves the exact nonempty ASCII lexeme without assigning stronger
/// semantics.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmVenueChangeHash {
    length: u8,
    bytes: [u8; MAX_PM_VENUE_CHANGE_HASH_BYTES],
}

impl PmVenueChangeHash {
    pub fn new(value: &str) -> Result<Self, PmEventError> {
        if value.is_empty() {
            return Err(PmEventError::EmptyVenueChangeHash);
        }
        if value.len() > MAX_PM_VENUE_CHANGE_HASH_BYTES {
            return Err(PmEventError::VenueChangeHashTooLong);
        }
        if !value.is_ascii() {
            return Err(PmEventError::NonAsciiVenueChangeHash);
        }
        let mut bytes = [0_u8; MAX_PM_VENUE_CHANGE_HASH_BYTES];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            length: value.len() as u8,
            bytes,
        })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.length)])
            .expect("checked ASCII venue change hash")
    }
}

impl std::fmt::Debug for PmVenueChangeHash {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("PmVenueChangeHash")
            .field(&self.as_str())
            .finish()
    }
}

/// A positive level suitable for a reduced best-bid/ask observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmBookPoint {
    price: PmPrice,
    quantity: PmQuantity,
}

impl PmBookPoint {
    #[must_use]
    pub const fn new(price: PmPrice, quantity: PmQuantity) -> Self {
        Self { price, quantity }
    }

    #[must_use]
    pub const fn price(self) -> PmPrice {
        self.price
    }

    #[must_use]
    pub const fn quantity(self) -> PmQuantity {
        self.quantity
    }
}

/// A derived book top. Empty sides remain explicit and a crossed top is never
/// admitted as a normalized value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmBookTop {
    bid: Option<PmBookPoint>,
    ask: Option<PmBookPoint>,
}

impl PmBookTop {
    pub fn new(bid: Option<PmBookPoint>, ask: Option<PmBookPoint>) -> Result<Self, PmEventError> {
        if matches!((bid, ask), (Some(bid), Some(ask)) if bid.price() >= ask.price()) {
            return Err(PmEventError::CrossedBookTop);
        }
        Ok(Self { bid, ask })
    }

    #[must_use]
    pub const fn bid(self) -> Option<PmBookPoint> {
        self.bid
    }

    #[must_use]
    pub const fn ask(self) -> Option<PmBookPoint> {
        self.ask
    }
}

/// One whole normalized snapshot. Construction preserves the venue frame as
/// one bounded unit so queue admission cannot expose a partial snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PmBookSnapshot {
    levels: Box<[PmBookLevel]>,
}

impl PmBookSnapshot {
    pub fn new(levels: Box<[PmBookLevel]>) -> Result<Self, PmEventError> {
        if levels.len() > usize::from(MAX_PM_BOOK_LEVELS) {
            return Err(PmEventError::TooManyBookLevels);
        }
        if levels
            .iter()
            .any(|level| level.quantity() == PmBookQuantity::Delete)
        {
            return Err(PmEventError::SnapshotLevelIsDelete);
        }
        Ok(Self { levels })
    }

    #[must_use]
    pub fn levels(&self) -> &[PmBookLevel] {
        &self.levels
    }

    #[must_use]
    pub fn into_levels(self) -> Box<[PmBookLevel]> {
        self.levels
    }
}

/// One whole normalized `price_change` frame.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PmBookDeltaBatch {
    changes: Box<[PmBookLevel]>,
    venue_change_hashes: Box<[Option<PmVenueChangeHash>]>,
    expected_top: PmBookTopCheck,
}

/// Owned components of one atomic venue `price_change` frame.
pub type PmBookDeltaParts = (
    Box<[PmBookLevel]>,
    Box<[Option<PmVenueChangeHash>]>,
    PmBookTopCheck,
);

impl PmBookDeltaBatch {
    pub fn new(
        changes: Box<[PmBookLevel]>,
        expected_top: PmBookTopCheck,
    ) -> Result<Self, PmEventError> {
        let venue_change_hashes = vec![None; changes.len()].into_boxed_slice();
        Self::new_with_venue_hashes(changes, venue_change_hashes, expected_top)
    }

    pub fn new_with_venue_hashes(
        changes: Box<[PmBookLevel]>,
        venue_change_hashes: Box<[Option<PmVenueChangeHash>]>,
        expected_top: PmBookTopCheck,
    ) -> Result<Self, PmEventError> {
        if changes.len() > usize::from(MAX_PM_BOOK_LEVELS) {
            return Err(PmEventError::TooManyBookLevels);
        }
        if changes.len() != venue_change_hashes.len() {
            return Err(PmEventError::ChangeHashCountMismatch);
        }
        Ok(Self {
            changes,
            venue_change_hashes,
            expected_top,
        })
    }

    #[must_use]
    pub fn changes(&self) -> &[PmBookLevel] {
        &self.changes
    }

    /// Ordered optional venue `hash` lexemes, one-for-one with `changes`.
    #[must_use]
    pub fn venue_change_hashes(&self) -> &[Option<PmVenueChangeHash>] {
        &self.venue_change_hashes
    }

    /// Venue-supplied final best prices for the whole delta frame.
    ///
    /// The state reducer validates this against the post-delta candidate
    /// before committing any level, so frame integrity cannot be split across
    /// independently admitted queue items.
    #[must_use]
    pub const fn expected_top(&self) -> PmBookTopCheck {
        self.expected_top
    }

    #[must_use]
    pub fn into_parts(self) -> PmBookDeltaParts {
        (self.changes, self.venue_change_hashes, self.expected_top)
    }
}

/// Venue-supplied best prices used only to check the reduced book.
///
/// Empty, locked, or crossed values remain representable here so the state
/// boundary can invalidate readiness with the exact typed reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmBookTopCheck {
    bid: Option<PmPrice>,
    ask: Option<PmPrice>,
}

impl PmBookTopCheck {
    #[must_use]
    pub const fn new(bid: Option<PmPrice>, ask: Option<PmPrice>) -> Self {
        Self { bid, ask }
    }

    #[must_use]
    pub const fn bid(self) -> Option<PmPrice> {
        self.bid
    }

    #[must_use]
    pub const fn ask(self) -> Option<PmPrice> {
        self.ask
    }
}

/// Normalized book messages retain venue-frame atomicity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PmBookUpdate {
    Snapshot(PmBookSnapshot),
    DeltaBatch(PmBookDeltaBatch),
    TopCheck(PmBookTopCheck),
    TickSizeChanged { old: PmTick, new: PmTick },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PmBookEvent {
    source: PmProductSource,
    instrument: PmInstrumentHandle,
    metadata_revision: SnapshotRevision,
    update: PmBookUpdate,
}

impl PmBookEvent {
    pub fn new(
        source: PmProductSource,
        instrument: PmInstrumentHandle,
        metadata_revision: SnapshotRevision,
        update: PmBookUpdate,
    ) -> Result<Self, PmEventError> {
        validate_market_source(source, instrument)?;
        validate_revision(metadata_revision)?;
        Ok(Self {
            source,
            instrument,
            metadata_revision,
            update,
        })
    }

    #[must_use]
    pub const fn source(&self) -> PmProductSource {
        self.source
    }

    #[must_use]
    pub const fn instrument(&self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn metadata_revision(&self) -> SnapshotRevision {
        self.metadata_revision
    }

    #[must_use]
    pub const fn update(&self) -> &PmBookUpdate {
        &self.update
    }
}

/// Structural order identity. Remote-only observations and locally-created
/// orders use the same carrier without treating either identifier as the
/// other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmOrderIdentity {
    client_order_key: Option<PmClientOrderKey>,
    venue_order_key: Option<PmVenueOrderKey>,
}

impl PmOrderIdentity {
    pub fn new(
        client_order_key: Option<PmClientOrderKey>,
        venue_order_key: Option<PmVenueOrderKey>,
    ) -> Result<Self, PmEventError> {
        if client_order_key.is_none() && venue_order_key.is_none() {
            return Err(PmEventError::MissingOrderIdentity);
        }
        if matches!(
            (client_order_key, venue_order_key),
            (Some(client), Some(venue)) if client.account() != venue.account()
        ) {
            return Err(PmEventError::OrderIdentityAccountMismatch);
        }
        Ok(Self {
            client_order_key,
            venue_order_key,
        })
    }

    #[must_use]
    pub const fn account(self) -> PmAccountHandle {
        match self.client_order_key {
            Some(client) => client.account(),
            None => match self.venue_order_key {
                Some(venue) => venue.account(),
                None => unreachable!(),
            },
        }
    }

    #[must_use]
    pub const fn client_order_key(self) -> Option<PmClientOrderKey> {
        self.client_order_key
    }

    #[must_use]
    pub const fn venue_order_key(self) -> Option<PmVenueOrderKey> {
        self.venue_order_key
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PmOrderStatus {
    Pending,
    Open,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
    Expired,
}

impl PmOrderStatus {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Filled | Self::Cancelled | Self::Rejected | Self::Expired
        )
    }
}

/// Checked exact order progress with no redundant remaining-quantity field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmOrderProgress {
    original_quantity: PmQuantity,
    cumulative_filled: U256,
    status: PmOrderStatus,
}

impl PmOrderProgress {
    pub fn new(
        original_quantity: PmQuantity,
        cumulative_filled: U256,
        status: PmOrderStatus,
    ) -> Result<Self, PmEventError> {
        let original_units = original_quantity.protocol_units();
        if cumulative_filled > original_units {
            return Err(PmEventError::CumulativeFillExceedsOriginal);
        }
        let status_matches = match status {
            PmOrderStatus::Pending | PmOrderStatus::Open => cumulative_filled.is_zero(),
            PmOrderStatus::PartiallyFilled => {
                !cumulative_filled.is_zero() && cumulative_filled < original_units
            }
            PmOrderStatus::Filled => cumulative_filled == original_units,
            PmOrderStatus::Rejected => cumulative_filled.is_zero(),
            PmOrderStatus::Cancelled | PmOrderStatus::Expired => true,
        };
        if !status_matches {
            return Err(PmEventError::OrderStatusFillMismatch);
        }
        Ok(Self {
            original_quantity,
            cumulative_filled,
            status,
        })
    }

    #[must_use]
    pub const fn original_quantity(self) -> PmQuantity {
        self.original_quantity
    }

    #[must_use]
    pub const fn cumulative_filled(self) -> U256 {
        self.cumulative_filled
    }

    #[must_use]
    pub const fn status(self) -> PmOrderStatus {
        self.status
    }

    #[must_use]
    pub fn remaining_quantity_units(self) -> U256 {
        self.original_quantity
            .protocol_units()
            .checked_sub(self.cumulative_filled)
            .expect("checked order progress")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmOrderEvent {
    source: PmProductSource,
    instrument: PmInstrumentHandle,
    order: PmOrderIdentity,
    side: PmOrderSide,
    price: PmPrice,
    progress: PmOrderProgress,
}

impl PmOrderEvent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: PmProductSource,
        instrument: PmInstrumentHandle,
        order: PmOrderIdentity,
        side: PmOrderSide,
        price: PmPrice,
        progress: PmOrderProgress,
    ) -> Result<Self, PmEventError> {
        validate_account_source(source, order.account())?;
        Ok(Self {
            source,
            instrument,
            order,
            side,
            price,
            progress,
        })
    }

    #[must_use]
    pub const fn source(self) -> PmProductSource {
        self.source
    }

    #[must_use]
    pub const fn account(self) -> PmAccountHandle {
        self.order.account()
    }

    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn order(self) -> PmOrderIdentity {
        self.order
    }

    #[must_use]
    pub const fn side(self) -> PmOrderSide {
        self.side
    }

    #[must_use]
    pub const fn price(self) -> PmPrice {
        self.price
    }

    #[must_use]
    pub const fn progress(self) -> PmOrderProgress {
        self.progress
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PmFillRole {
    Maker,
    Taker,
}

/// Exact Polymarket trade-settlement lifecycle.
///
/// A matched trade affects provisional inventory immediately, but only
/// authoritative reconciliation may clear retry/failure uncertainty. This
/// value never implies that a balance or position snapshot changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PmFillSettlementStatus {
    Matched,
    Mined,
    Confirmed,
    Retrying,
    Failed,
}

/// A fee is never collapsed to zero when the venue response is absent or
/// partial. Known deltas retain the exact affected asset and sign.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PmFillFee {
    Known {
        asset: PmAssetId,
        delta: PmSignedUnits,
    },
    Unknown,
    Incomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmFillExecution {
    side: PmOrderSide,
    role: PmFillRole,
    settlement: PmFillSettlementStatus,
    price: PmPrice,
    quantity: PmQuantity,
    fee: PmFillFee,
}

impl PmFillExecution {
    #[must_use]
    pub const fn new(
        side: PmOrderSide,
        role: PmFillRole,
        settlement: PmFillSettlementStatus,
        price: PmPrice,
        quantity: PmQuantity,
        fee: PmFillFee,
    ) -> Self {
        Self {
            side,
            role,
            settlement,
            price,
            quantity,
            fee,
        }
    }

    #[must_use]
    pub const fn side(self) -> PmOrderSide {
        self.side
    }

    #[must_use]
    pub const fn role(self) -> PmFillRole {
        self.role
    }

    #[must_use]
    pub const fn settlement(self) -> PmFillSettlementStatus {
        self.settlement
    }

    #[must_use]
    pub const fn price(self) -> PmPrice {
        self.price
    }

    #[must_use]
    pub const fn quantity(self) -> PmQuantity {
        self.quantity
    }

    #[must_use]
    pub const fn fee(self) -> PmFillFee {
        self.fee
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmFillEvent {
    source: PmProductSource,
    instrument: PmInstrumentHandle,
    fill_key: PmFillKey,
    order: PmOrderIdentity,
    execution: PmFillExecution,
}

impl PmFillEvent {
    pub fn new(
        source: PmProductSource,
        instrument: PmInstrumentHandle,
        fill_key: PmFillKey,
        order: PmOrderIdentity,
        execution: PmFillExecution,
    ) -> Result<Self, PmEventError> {
        let order_venue = order
            .venue_order_key()
            .ok_or(PmEventError::MissingFillVenueOrderIdentity)?;
        if fill_key.venue_order() != order_venue {
            return Err(PmEventError::FillVenueOrderMismatch);
        }
        validate_account_source(source, fill_key.account())?;
        Ok(Self {
            source,
            instrument,
            fill_key,
            order,
            execution,
        })
    }

    #[must_use]
    pub const fn source(self) -> PmProductSource {
        self.source
    }

    #[must_use]
    pub const fn account(self) -> PmAccountHandle {
        self.fill_key.account()
    }

    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn fill_key(self) -> PmFillKey {
        self.fill_key
    }

    #[must_use]
    pub const fn order(self) -> PmOrderIdentity {
        self.order
    }

    #[must_use]
    pub const fn execution(self) -> PmFillExecution {
        self.execution
    }
}

/// Validated nonzero revision shared by rows from one snapshot response.
///
/// This row-level value carries no completeness authority. Only a checked
/// aggregate reconciliation/account carrier can assert whole-query
/// completeness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmSnapshotEvidence {
    revision: SnapshotRevision,
}

impl PmSnapshotEvidence {
    pub fn new(revision: SnapshotRevision) -> Result<Self, PmEventError> {
        validate_revision(revision)?;
        Ok(Self { revision })
    }

    #[must_use]
    pub const fn revision(self) -> SnapshotRevision {
        self.revision
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmBalanceEvent {
    source: PmProductSource,
    account: PmAccountHandle,
    asset: PmAssetId,
    balance: U256,
    snapshot: PmSnapshotEvidence,
}

impl PmBalanceEvent {
    pub fn new(
        source: PmProductSource,
        account: PmAccountHandle,
        asset: PmAssetId,
        balance: U256,
        snapshot: PmSnapshotEvidence,
    ) -> Result<Self, PmEventError> {
        validate_account_source(source, account)?;
        Ok(Self {
            source,
            account,
            asset,
            balance,
            snapshot,
        })
    }

    #[must_use]
    pub const fn source(self) -> PmProductSource {
        self.source
    }

    #[must_use]
    pub const fn account(self) -> PmAccountHandle {
        self.account
    }

    #[must_use]
    pub const fn asset(self) -> PmAssetId {
        self.asset
    }

    #[must_use]
    pub const fn balance(self) -> U256 {
        self.balance
    }

    #[must_use]
    pub const fn snapshot(self) -> PmSnapshotEvidence {
        self.snapshot
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PmAllowanceValue {
    Erc20(U256),
    Erc1155Operator(PmErc1155OperatorApproval),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmAllowanceEvent {
    source: PmProductSource,
    spender: PmSpenderId,
    value: PmAllowanceValue,
    snapshot: PmSnapshotEvidence,
}

impl PmAllowanceEvent {
    pub fn new(
        source: PmProductSource,
        spender: PmSpenderId,
        value: PmAllowanceValue,
        snapshot: PmSnapshotEvidence,
    ) -> Result<Self, PmEventError> {
        validate_account_source(source, spender.account())?;
        let matching_kind = matches!(
            (spender.requirement().asset(), value),
            (PmAssetId::Collateral { .. }, PmAllowanceValue::Erc20(_))
                | (
                    PmAssetId::Outcome { .. },
                    PmAllowanceValue::Erc1155Operator(_)
                )
        );
        if !matching_kind {
            return Err(PmEventError::AllowanceAssetKindMismatch);
        }
        Ok(Self {
            source,
            spender,
            value,
            snapshot,
        })
    }

    #[must_use]
    pub const fn source(self) -> PmProductSource {
        self.source
    }

    #[must_use]
    pub const fn account(self) -> PmAccountHandle {
        self.spender.account()
    }

    #[must_use]
    pub const fn spender(self) -> PmSpenderId {
        self.spender
    }

    #[must_use]
    pub const fn asset(self) -> PmAssetId {
        self.spender.requirement().asset()
    }

    #[must_use]
    pub const fn value(self) -> PmAllowanceValue {
        self.value
    }

    #[must_use]
    pub const fn snapshot(self) -> PmSnapshotEvidence {
        self.snapshot
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PmPositionAvailability {
    Tradable,
    ResolvedUnredeemed,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmPositionEvent {
    source: PmProductSource,
    account: PmAccountHandle,
    instrument: PmInstrumentHandle,
    quantity: U256,
    availability: PmPositionAvailability,
    snapshot: PmSnapshotEvidence,
}

impl PmPositionEvent {
    pub fn new(
        source: PmProductSource,
        account: PmAccountHandle,
        instrument: PmInstrumentHandle,
        quantity: U256,
        availability: PmPositionAvailability,
        snapshot: PmSnapshotEvidence,
    ) -> Result<Self, PmEventError> {
        validate_account_source(source, account)?;
        Ok(Self {
            source,
            account,
            instrument,
            quantity,
            availability,
            snapshot,
        })
    }

    #[must_use]
    pub const fn source(self) -> PmProductSource {
        self.source
    }

    #[must_use]
    pub const fn account(self) -> PmAccountHandle {
        self.account
    }

    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn quantity(self) -> U256 {
        self.quantity
    }

    #[must_use]
    pub const fn availability(self) -> PmPositionAvailability {
        self.availability
    }

    #[must_use]
    pub const fn snapshot(self) -> PmSnapshotEvidence {
        self.snapshot
    }
}

/// One exact reference observation bound to its configured OKX source handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OkxReferenceEvent {
    source: PmProductSource,
    reference: OkxReferenceHandle,
    price: OkxReferencePrice,
}

impl OkxReferenceEvent {
    pub fn new(
        source: PmProductSource,
        reference: OkxReferenceHandle,
        price: OkxReferencePrice,
    ) -> Result<Self, OkxReferenceEventError> {
        match source {
            PmProductSource::OkxReference {
                reference: source_reference,
                ..
            } if source_reference == reference => Ok(Self {
                source,
                reference,
                price,
            }),
            PmProductSource::OkxReference { .. } => {
                Err(OkxReferenceEventError::ReferenceHandleMismatch)
            }
            PmProductSource::PolymarketMarket { .. }
            | PmProductSource::PolymarketAccount { .. } => Err(OkxReferenceEventError::WrongSource),
        }
    }

    #[must_use]
    pub const fn source(self) -> PmProductSource {
        self.source
    }

    #[must_use]
    pub const fn reference(self) -> OkxReferenceHandle {
        self.reference
    }

    #[must_use]
    pub const fn price(self) -> OkxReferencePrice {
        self.price
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum OkxReferenceEventError {
    #[error("reference event requires an OKX reference source")]
    WrongSource,
    #[error("OKX source reference handle does not match the event")]
    ReferenceHandleMismatch,
}

macro_rules! source_bound_event {
    ($event:ty) => {
        impl PmSourceBound for $event {
            fn source(&self) -> PmProductSource {
                <$event>::source(*self)
            }
        }
    };
}

source_bound_event!(PmMarketEvent);
impl PmSourceBound for PmBookEvent {
    fn source(&self) -> PmProductSource {
        self.source()
    }
}
source_bound_event!(PmOrderEvent);
source_bound_event!(PmFillEvent);
source_bound_event!(PmBalanceEvent);
source_bound_event!(PmAllowanceEvent);
source_bound_event!(PmPositionEvent);
source_bound_event!(OkxReferenceEvent);

fn validate_market_source(
    source: PmProductSource,
    instrument: PmInstrumentHandle,
) -> Result<(), PmEventError> {
    match source {
        PmProductSource::PolymarketMarket { token, .. } if token == instrument.token() => Ok(()),
        PmProductSource::PolymarketMarket { .. } => Err(PmEventError::MarketSourceTokenMismatch),
        PmProductSource::OkxReference { .. } | PmProductSource::PolymarketAccount { .. } => {
            Err(PmEventError::WrongMarketSource)
        }
    }
}

fn validate_account_source(
    source: PmProductSource,
    account: PmAccountHandle,
) -> Result<(), PmEventError> {
    match source {
        PmProductSource::PolymarketAccount {
            account: source_account,
            ..
        } if source_account == account => Ok(()),
        PmProductSource::PolymarketAccount { .. } => Err(PmEventError::AccountSourceMismatch),
        PmProductSource::OkxReference { .. } | PmProductSource::PolymarketMarket { .. } => {
            Err(PmEventError::WrongAccountSource)
        }
    }
}

fn validate_revision(revision: SnapshotRevision) -> Result<(), PmEventError> {
    if revision.value() == 0 {
        Err(PmEventError::ZeroRevision)
    } else {
        Ok(())
    }
}
