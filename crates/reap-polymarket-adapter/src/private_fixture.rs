use reap_pm_core::{
    ConnectionEpoch, EventEnvelope, EvmAddress, IngressSequence, PmAccountHandle, PmAccountScope,
    PmBookQuantity, PmConnectionId, PmFillEvent, PmFillExecution, PmFillFee, PmFillId, PmFillKey,
    PmFillRole, PmFillSettlementStatus, PmInstrumentHandle, PmInstrumentId, PmMarketId,
    PmOrderEvent, PmOrderIdentity, PmOrderProgress, PmOrderSide, PmOrderStatus, PmPrice,
    PmProductSource, PmQuantity, PmSourceBound, PmTokenId, PmVenueOrderId, PmVenueOrderKey, U256,
    exact_order_amounts,
};
use reap_polymarket_wire::{
    PmFixtureOpenOrder, PmFixtureUserEvent, PmFixtureUserFrame, PmFixtureUserOrder,
    PmFixtureUserTrade, PmPrivateFixtureError, parse_private_user_fixture,
};
use thiserror::Error;

use crate::fixture_delivery::checked_delivery;
use crate::fixture_scope::PmFixtureOwnerId;
use crate::fixture_scope::validate_account_source;
use crate::{
    PmFixtureAggregateDelivery, PmFixtureCompletionOccurrence, PmFixtureDeliveryError,
    PmFixtureDeliveryScope, PmFixtureInstrumentScope, PmFixturePrivateRoleGrant,
    PmFixtureScopeError, PmFixtureServicedAggregate,
};

pub const MAX_PM_PRIVATE_NORMALIZED_OBSERVATIONS: usize = 4_096;
pub type PmFixturePrivateDelivery = PmFixtureAggregateDelivery<PmFixturePrivateBatch>;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PmFixtureFeeEvidence {
    Known {
        asset: reap_pm_core::PmAssetId,
        delta: reap_pm_core::PmSignedUnits,
    },
    Unknown,
    Incomplete,
}

impl PmFixtureFeeEvidence {
    const fn into_core(self) -> PmFillFee {
        match self {
            Self::Known { asset, delta } => PmFillFee::Known { asset, delta },
            Self::Unknown => PmFillFee::Unknown,
            Self::Incomplete => PmFillFee::Incomplete,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PmUnresolvedTradeReason {
    MissingExactOrderLinkage,
    MultipleOrderReferenceKinds,
    MissingDirectOrderRole,
    MissingLocalMakerOrderProof,
    ExternalMakerOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmFixtureUnresolvedTrade {
    source: PmProductSource,
    account: PmAccountHandle,
    instrument: PmInstrumentHandle,
    fill_id: PmFillId,
    order: Option<PmVenueOrderKey>,
    candidate_order: Option<PmVenueOrderId>,
    reason: PmUnresolvedTradeReason,
    settlement: PmFillSettlementStatus,
    fee: PmFixtureFeeEvidence,
}

impl PmFixtureUnresolvedTrade {
    #[must_use]
    pub const fn account(self) -> PmAccountHandle {
        self.account
    }

    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn fill_id(self) -> PmFillId {
        self.fill_id
    }

    #[must_use]
    pub const fn order(self) -> Option<PmVenueOrderKey> {
        self.order
    }

    #[must_use]
    pub const fn candidate_order(self) -> Option<PmVenueOrderId> {
        self.candidate_order
    }

    #[must_use]
    pub const fn reason(self) -> PmUnresolvedTradeReason {
        self.reason
    }

    #[must_use]
    pub const fn settlement(self) -> PmFillSettlementStatus {
        self.settlement
    }

    #[must_use]
    pub const fn fee(self) -> PmFixtureFeeEvidence {
        self.fee
    }
}

impl PmSourceBound for PmFixtureUnresolvedTrade {
    fn source(&self) -> PmProductSource {
        self.source
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PmPrivateLifecycleObservation {
    Order(PmOrderEvent),
    Fill(PmFillEvent),
    UnresolvedTrade(PmFixtureUnresolvedTrade),
}

#[derive(Debug, PartialEq, Eq)]
pub struct PmFixturePrivateBatch {
    account_scope: PmAccountScope,
    source: PmProductSource,
    instrument: PmFixtureInstrumentScope,
    observations: Box<[PmPrivateLifecycleObservation]>,
}

impl PmFixturePrivateBatch {
    #[must_use]
    pub const fn account_scope(&self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn instrument_scope(&self) -> PmFixtureInstrumentScope {
        self.instrument
    }

    #[must_use]
    pub fn observations(&self) -> &[PmPrivateLifecycleObservation] {
        &self.observations
    }
}

impl PmSourceBound for PmFixturePrivateBatch {
    fn source(&self) -> PmProductSource {
        self.source
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPrivateNormalizationError {
    #[error("private fixture wire parsing failed: {0}")]
    Wire(#[from] PmPrivateFixtureError),
    #[error("private fixture market identity is invalid")]
    InvalidMarket,
    #[error("private fixture outcome-token identity is invalid")]
    InvalidToken,
    #[error("private fixture names another configured instrument")]
    InstrumentMismatch,
    #[error("private fixture maker/funder identity is invalid")]
    InvalidFunder,
    #[error("private fixture maker/funder differs from the exact account scope")]
    FunderMismatch,
    #[error("private fixture venue-order identity is invalid")]
    InvalidVenueOrder,
    #[error("private fixture fill identity is invalid")]
    InvalidFill,
    #[error("private fixture order side is unknown")]
    UnknownSide,
    #[error("private fixture order status is unknown")]
    UnknownOrderStatus,
    #[error("private fixture order event kind is outside the proven fixture contract")]
    UnknownOrderEventKind,
    #[error("private fixture order event kind conflicts with its exact progress/status")]
    OrderEventKindStatusMismatch,
    #[error("private fixture trade status is unknown")]
    UnknownTradeStatus,
    #[error("private fixture trader role is unknown")]
    UnknownTradeRole,
    #[error("private fixture trader role conflicts with its order-reference kind")]
    TradeRoleMismatch,
    #[error("private fixture order price is invalid")]
    InvalidPrice,
    #[error("private fixture price is off the configured market tick")]
    PriceOffTick,
    #[error("private fixture original or fill quantity is invalid")]
    InvalidQuantity,
    #[error("private fixture original order quantity is below minimum or off the CLOB V2 lot")]
    InvalidOrderQuantityContract,
    #[error("private fixture price and quantity do not produce integral protocol amounts")]
    NonIntegralProtocolAmounts,
    #[error("private fixture matched quantity is invalid")]
    InvalidMatchedQuantity,
    #[error("private fixture open-order response contains a terminal order")]
    OpenOrderIsTerminal,
    #[error("private fixture contains duplicate exact order references")]
    DuplicateOrderReference,
    #[error("private fixture normalization exceeds its fixed observation bound")]
    TooManyObservations,
    #[error("normalized private fixture violates the PM core event contract")]
    EventContract,
    #[error("private fixture stream has no active connection epoch")]
    NoActiveEpoch,
    #[error("private fixture reconnect epoch must be nonzero")]
    ZeroConnectionEpoch,
    #[error("private fixture reconnect epoch did not advance")]
    ConnectionEpochDidNotAdvance,
    #[error("private fixture delivery belongs to an old or unknown connection epoch")]
    ConnectionEpochMismatch,
    #[error("private fixture local ingress sequence did not advance")]
    IngressSequenceDidNotAdvance,
    #[error("private fixture user stream cannot invent venue sequence")]
    UnexpectedVenueSequence,
    #[error("private fixture user stream cannot carry snapshot revision")]
    UnexpectedSnapshotRevision,
    #[error("private fixture delivery construction failed: {0}")]
    Delivery(#[from] PmFixtureDeliveryError),
}

/// Fixture-only PM order/fill lifecycle observation capability.
pub trait PmPrivateLifecycleRole: sealed::Sealed {
    type OrderObservation;
    type FillObservation;
    type UnresolvedObservation;

    fn account_scope(&self) -> PmAccountScope;
    fn account(&self) -> PmAccountHandle;
    fn instrument_scope(&self) -> PmFixtureInstrumentScope;
    fn source(&self) -> PmProductSource;
    fn connection(&self) -> PmConnectionId;
    fn active_epoch(&self) -> Option<ConnectionEpoch>;
}

#[derive(Debug, PartialEq, Eq)]
pub struct PmFixturePrivateLifecycle {
    owner_id: PmFixtureOwnerId,
    account_scope: PmAccountScope,
    instrument: PmFixtureInstrumentScope,
    source: PmProductSource,
    connection: PmConnectionId,
    active_epoch: Option<ConnectionEpoch>,
    last_ingress_sequence: IngressSequence,
}

impl PmFixturePrivateLifecycle {
    pub fn new(
        grant: PmFixturePrivateRoleGrant,
        account_scope: PmAccountScope,
        instrument: PmFixtureInstrumentScope,
        source: PmProductSource,
        connection: PmConnectionId,
    ) -> Result<Self, PmPrivateLifecycleRoleError> {
        Self::new_bound(
            grant.into_owner_id(),
            account_scope,
            instrument,
            source,
            connection,
        )
    }

    pub(crate) fn new_bound(
        owner_id: PmFixtureOwnerId,
        account_scope: PmAccountScope,
        instrument: PmFixtureInstrumentScope,
        source: PmProductSource,
        connection: PmConnectionId,
    ) -> Result<Self, PmPrivateLifecycleRoleError> {
        validate_role_source(account_scope, source)?;
        Ok(Self {
            owner_id,
            account_scope,
            instrument,
            source,
            connection,
            active_epoch: None,
            last_ingress_sequence: IngressSequence::new(0),
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
    pub const fn instrument_scope(&self) -> PmFixtureInstrumentScope {
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
    pub const fn active_epoch(&self) -> Option<ConnectionEpoch> {
        self.active_epoch
    }

    pub fn reconnect(
        &mut self,
        connection_epoch: ConnectionEpoch,
    ) -> Result<(), PmPrivateNormalizationError> {
        if connection_epoch.value() == 0 {
            return Err(PmPrivateNormalizationError::ZeroConnectionEpoch);
        }
        if self
            .active_epoch
            .is_some_and(|active| connection_epoch <= active)
        {
            return Err(PmPrivateNormalizationError::ConnectionEpochDidNotAdvance);
        }
        self.active_epoch = Some(connection_epoch);
        self.last_ingress_sequence = IngressSequence::new(0);
        Ok(())
    }

    pub fn receive_user_fixture(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
        raw: &[u8],
        fee: PmFixtureFeeEvidence,
    ) -> Result<PmFixturePrivateDelivery, PmPrivateNormalizationError> {
        let ordering = occurrence.ordering();
        let active_epoch = self
            .active_epoch
            .ok_or(PmPrivateNormalizationError::NoActiveEpoch)?;
        if ordering.connection_epoch() != active_epoch {
            return Err(PmPrivateNormalizationError::ConnectionEpochMismatch);
        }
        if ordering.local_ingress_sequence() <= self.last_ingress_sequence {
            return Err(PmPrivateNormalizationError::IngressSequenceDidNotAdvance);
        }
        if ordering.venue_sequence().is_some() {
            return Err(PmPrivateNormalizationError::UnexpectedVenueSequence);
        }
        if ordering.snapshot_revision().is_some() {
            return Err(PmPrivateNormalizationError::UnexpectedSnapshotRevision);
        }
        let batch = self.normalize_user_payload(raw, fee)?;
        let delivery = checked_delivery(
            self.owner_id,
            self.account_scope,
            self.instrument,
            self.source,
            self.connection,
            occurrence,
            batch,
        )?;
        self.last_ingress_sequence = ordering.local_ingress_sequence();
        Ok(delivery)
    }

    /// Opens a serviced private delivery only for the exact role instance
    /// that produced it, while retaining its structural scope at reduction.
    pub fn reduce_private_delivery<R>(
        &self,
        delivery: PmFixtureServicedAggregate<PmFixturePrivateBatch>,
        reduce: impl FnOnce(PmFixtureDeliveryScope, EventEnvelope<PmFixturePrivateBatch>) -> R,
    ) -> Result<R, Box<PmFixtureServicedAggregate<PmFixturePrivateBatch>>> {
        delivery.reduce_with_owner(self.owner_id, reduce)
    }

    pub(crate) fn normalize_user_payload(
        &self,
        raw: &[u8],
        fee: PmFixtureFeeEvidence,
    ) -> Result<PmFixturePrivateBatch, PmPrivateNormalizationError> {
        let frame = parse_private_user_fixture(raw)?;
        self.normalize_user_frame(&frame, fee)
    }

    pub(crate) fn normalize_user_frame(
        &self,
        frame: &PmFixtureUserFrame,
        fee: PmFixtureFeeEvidence,
    ) -> Result<PmFixturePrivateBatch, PmPrivateNormalizationError> {
        let mut observations = Vec::with_capacity(frame.events().len());
        for event in frame.events() {
            match event {
                PmFixtureUserEvent::Order(order) => {
                    push_observation(
                        &mut observations,
                        PmPrivateLifecycleObservation::Order(normalize_user_order(self, order)?),
                    )?;
                }
                PmFixtureUserEvent::Trade(trade) => {
                    for observation in normalize_trade(self, trade, fee)? {
                        push_observation(&mut observations, observation)?;
                    }
                }
            }
        }
        Ok(PmFixturePrivateBatch {
            account_scope: self.account_scope,
            source: self.source,
            instrument: self.instrument,
            observations: observations.into_boxed_slice(),
        })
    }
}

impl sealed::Sealed for PmFixturePrivateLifecycle {}

impl PmPrivateLifecycleRole for PmFixturePrivateLifecycle {
    type OrderObservation = PmOrderEvent;
    type FillObservation = PmFillEvent;
    type UnresolvedObservation = PmFixtureUnresolvedTrade;

    fn account_scope(&self) -> PmAccountScope {
        self.account_scope
    }

    fn account(&self) -> PmAccountHandle {
        self.account_scope.handle()
    }

    fn instrument_scope(&self) -> PmFixtureInstrumentScope {
        self.instrument
    }

    fn source(&self) -> PmProductSource {
        self.source
    }

    fn connection(&self) -> PmConnectionId {
        self.connection
    }

    fn active_epoch(&self) -> Option<ConnectionEpoch> {
        self.active_epoch
    }
}

pub(crate) fn normalize_open_order(
    role: &PmFixturePrivateLifecycle,
    order: &PmFixtureOpenOrder,
) -> Result<PmOrderEvent, PmPrivateNormalizationError> {
    normalize_order_fields(
        role,
        OrderFields {
            id: order.id(),
            market: order.market(),
            asset_id: order.asset_id(),
            side: order.side(),
            original_size: order.original_size(),
            size_matched: order.size_matched(),
            price: order.price(),
            status: order.status(),
            maker_address: order.maker_address(),
        },
        true,
    )
}

pub(crate) fn normalize_order_detail(
    role: &PmFixturePrivateLifecycle,
    order: &PmFixtureOpenOrder,
) -> Result<PmOrderEvent, PmPrivateNormalizationError> {
    normalize_order_fields(
        role,
        OrderFields {
            id: order.id(),
            market: order.market(),
            asset_id: order.asset_id(),
            side: order.side(),
            original_size: order.original_size(),
            size_matched: order.size_matched(),
            price: order.price(),
            status: order.status(),
            maker_address: order.maker_address(),
        },
        false,
    )
}

fn normalize_user_order(
    role: &PmFixturePrivateLifecycle,
    order: &PmFixtureUserOrder,
) -> Result<PmOrderEvent, PmPrivateNormalizationError> {
    let kind = parse_user_order_kind(order.event_kind())?;
    let event = normalize_order_fields(
        role,
        OrderFields {
            id: order.id(),
            market: order.market(),
            asset_id: order.asset_id(),
            side: order.side(),
            original_size: order.original_size(),
            size_matched: order.size_matched(),
            price: order.price(),
            status: order.status(),
            maker_address: order.maker_address(),
        },
        false,
    )?;
    validate_user_order_kind(kind, event)?;
    Ok(event)
}

fn normalize_order_fields(
    role: &PmFixturePrivateLifecycle,
    fields: OrderFields<'_>,
    require_open: bool,
) -> Result<PmOrderEvent, PmPrivateNormalizationError> {
    validate_instrument(role.instrument, fields.market, fields.asset_id)?;
    validate_funder(role.account_scope, fields.maker_address)?;
    let side = parse_side(fields.side)?;
    let price = PmPrice::parse_decimal(fields.price)
        .map_err(|_| PmPrivateNormalizationError::InvalidPrice)?;
    let original = PmQuantity::parse_decimal(fields.original_size)
        .map_err(|_| PmPrivateNormalizationError::InvalidQuantity)?;
    price
        .validate_tick(role.instrument.tick())
        .map_err(|_| PmPrivateNormalizationError::PriceOffTick)?;
    original
        .validate_order(role.instrument.minimum_order_size())
        .map_err(|_| PmPrivateNormalizationError::InvalidOrderQuantityContract)?;
    exact_order_amounts(side, price, original)
        .map_err(|_| PmPrivateNormalizationError::NonIntegralProtocolAmounts)?;
    let cumulative = parse_nonnegative_quantity(fields.size_matched)?;
    let status = parse_order_status(fields.status, cumulative)?;
    if require_open && status.is_terminal() {
        return Err(PmPrivateNormalizationError::OpenOrderIsTerminal);
    }
    let progress = PmOrderProgress::new(original, cumulative, status)
        .map_err(|_| PmPrivateNormalizationError::EventContract)?;
    let venue_order = parse_venue_order(role.account(), fields.id)?;
    let identity = PmOrderIdentity::new(None, Some(venue_order))
        .map_err(|_| PmPrivateNormalizationError::EventContract)?;
    PmOrderEvent::new(
        role.source,
        role.instrument.handle(),
        identity,
        side,
        price,
        progress,
    )
    .map_err(|_| PmPrivateNormalizationError::EventContract)
}

fn normalize_trade(
    role: &PmFixturePrivateLifecycle,
    trade: &PmFixtureUserTrade,
    fee: PmFixtureFeeEvidence,
) -> Result<Vec<PmPrivateLifecycleObservation>, PmPrivateNormalizationError> {
    validate_instrument(role.instrument, trade.market(), trade.asset_id())?;
    EvmAddress::parse(trade.maker_address())
        .map_err(|_| PmPrivateNormalizationError::InvalidFunder)?;
    let settlement = parse_trade_status(trade.status())?;
    let fill_id =
        PmFillId::new(trade.id()).map_err(|_| PmPrivateNormalizationError::InvalidFill)?;

    match trade.trader_side() {
        Some("TAKER") => normalize_taker_trade(role, trade, fill_id, settlement, fee),
        Some("MAKER") => normalize_maker_trade(role, trade, fill_id, settlement, fee),
        Some(_) => Err(PmPrivateNormalizationError::UnknownTradeRole),
        None => normalize_roleless_trade(role, trade, fill_id, settlement, fee),
    }
}

fn normalize_roleless_trade(
    role: &PmFixturePrivateLifecycle,
    trade: &PmFixtureUserTrade,
    fill_id: PmFillId,
    settlement: PmFillSettlementStatus,
    fee: PmFixtureFeeEvidence,
) -> Result<Vec<PmPrivateLifecycleObservation>, PmPrivateNormalizationError> {
    let direct = trade.order_id();
    let taker = trade.taker_order_id();
    let maker = trade
        .maker_orders()
        .is_some_and(|orders| !orders.is_empty());
    if let (Some(order), None, false) = (direct, taker, maker) {
        let venue_order = parse_venue_order(role.account(), order)?;
        Ok(vec![unresolved_trade(
            role,
            fill_id,
            Some(venue_order),
            PmUnresolvedTradeReason::MissingDirectOrderRole,
            settlement,
            fee,
        )])
    } else if direct.is_none() && taker.is_none() && !maker {
        Ok(vec![unresolved_trade(
            role,
            fill_id,
            None,
            PmUnresolvedTradeReason::MissingExactOrderLinkage,
            settlement,
            fee,
        )])
    } else {
        Ok(vec![unresolved_trade(
            role,
            fill_id,
            None,
            PmUnresolvedTradeReason::MultipleOrderReferenceKinds,
            settlement,
            fee,
        )])
    }
}

#[allow(clippy::too_many_arguments)]
fn normalize_linked_trade(
    role: &PmFixturePrivateLifecycle,
    trade: &PmFixtureUserTrade,
    fill_id: PmFillId,
    venue_order: PmVenueOrderKey,
    fill_role: PmFillRole,
    settlement: PmFillSettlementStatus,
    fee: PmFixtureFeeEvidence,
) -> Result<Vec<PmPrivateLifecycleObservation>, PmPrivateNormalizationError> {
    let fill = normalize_fill(
        role,
        fill_id,
        venue_order,
        trade.side(),
        fill_role,
        settlement,
        trade.price(),
        trade.size(),
        fee,
    )?;
    Ok(vec![PmPrivateLifecycleObservation::Fill(fill)])
}

fn normalize_taker_trade(
    role: &PmFixturePrivateLifecycle,
    trade: &PmFixtureUserTrade,
    fill_id: PmFillId,
    settlement: PmFillSettlementStatus,
    fee: PmFixtureFeeEvidence,
) -> Result<Vec<PmPrivateLifecycleObservation>, PmPrivateNormalizationError> {
    let local_reference = match (trade.taker_order_id(), trade.order_id()) {
        (Some(order), None) | (None, Some(order)) => order,
        (None, None) => {
            return Ok(vec![unresolved_trade(
                role,
                fill_id,
                None,
                PmUnresolvedTradeReason::MissingExactOrderLinkage,
                settlement,
                fee,
            )]);
        }
        (Some(_), Some(_)) => {
            return Ok(vec![unresolved_trade(
                role,
                fill_id,
                None,
                PmUnresolvedTradeReason::MultipleOrderReferenceKinds,
                settlement,
                fee,
            )]);
        }
    };
    let venue_order = parse_venue_order(role.account(), local_reference)?;
    normalize_linked_trade(
        role,
        trade,
        fill_id,
        venue_order,
        PmFillRole::Taker,
        settlement,
        fee,
    )
}

fn normalize_maker_trade(
    role: &PmFixturePrivateLifecycle,
    trade: &PmFixtureUserTrade,
    fill_id: PmFillId,
    settlement: PmFillSettlementStatus,
    fee: PmFixtureFeeEvidence,
) -> Result<Vec<PmPrivateLifecycleObservation>, PmPrivateNormalizationError> {
    if let Some(maker_orders) = trade.maker_orders().filter(|orders| !orders.is_empty()) {
        let mut observations = Vec::with_capacity(maker_orders.len());
        let mut seen = Vec::with_capacity(maker_orders.len());
        for maker in maker_orders {
            validate_instrument(role.instrument, trade.market(), maker.asset_id())?;
            let candidate = PmVenueOrderId::new(maker.order_id())
                .map_err(|_| PmPrivateNormalizationError::InvalidVenueOrder)?;
            if seen.contains(&candidate) {
                return Err(PmPrivateNormalizationError::DuplicateOrderReference);
            }
            seen.push(candidate);
            let observation = match maker.maker_address() {
                Some(address) => {
                    let address = EvmAddress::parse(address)
                        .map_err(|_| PmPrivateNormalizationError::InvalidFunder)?;
                    if address == role.account_scope.funder().address() {
                        let venue_order = PmVenueOrderKey::new(role.account(), candidate);
                        PmPrivateLifecycleObservation::Fill(normalize_fill(
                            role,
                            fill_id,
                            venue_order,
                            maker.side(),
                            PmFillRole::Maker,
                            settlement,
                            maker.price(),
                            maker.matched_amount(),
                            fee,
                        )?)
                    } else {
                        unresolved_maker_leg(
                            role,
                            fill_id,
                            candidate,
                            PmUnresolvedTradeReason::ExternalMakerOrder,
                            settlement,
                            fee,
                        )
                    }
                }
                None => unresolved_maker_leg(
                    role,
                    fill_id,
                    candidate,
                    PmUnresolvedTradeReason::MissingLocalMakerOrderProof,
                    settlement,
                    fee,
                ),
            };
            observations.push(observation);
        }
        return Ok(observations);
    }
    match (trade.order_id(), trade.taker_order_id()) {
        (Some(order), None) => {
            validate_funder(role.account_scope, trade.maker_address())?;
            let venue_order = parse_venue_order(role.account(), order)?;
            normalize_linked_trade(
                role,
                trade,
                fill_id,
                venue_order,
                PmFillRole::Maker,
                settlement,
                fee,
            )
        }
        (None, None) => Ok(vec![unresolved_trade(
            role,
            fill_id,
            None,
            PmUnresolvedTradeReason::MissingExactOrderLinkage,
            settlement,
            fee,
        )]),
        (None, Some(_)) | (Some(_), Some(_)) => Err(PmPrivateNormalizationError::TradeRoleMismatch),
    }
}

#[allow(clippy::too_many_arguments)]
fn normalize_fill(
    role: &PmFixturePrivateLifecycle,
    fill_id: PmFillId,
    venue_order: PmVenueOrderKey,
    side: &str,
    fill_role: PmFillRole,
    settlement: PmFillSettlementStatus,
    price: &str,
    quantity: &str,
    fee: PmFixtureFeeEvidence,
) -> Result<PmFillEvent, PmPrivateNormalizationError> {
    let side = parse_side(side)?;
    let price =
        PmPrice::parse_decimal(price).map_err(|_| PmPrivateNormalizationError::InvalidPrice)?;
    let quantity = PmQuantity::parse_decimal(quantity)
        .map_err(|_| PmPrivateNormalizationError::InvalidQuantity)?;
    price
        .validate_tick(role.instrument.tick())
        .map_err(|_| PmPrivateNormalizationError::PriceOffTick)?;
    exact_order_amounts(side, price, quantity)
        .map_err(|_| PmPrivateNormalizationError::NonIntegralProtocolAmounts)?;
    let execution = PmFillExecution::new(
        side,
        fill_role,
        settlement,
        price,
        quantity,
        fee.into_core(),
    );
    let identity = PmOrderIdentity::new(None, Some(venue_order))
        .map_err(|_| PmPrivateNormalizationError::EventContract)?;
    PmFillEvent::new(
        role.source,
        role.instrument.handle(),
        PmFillKey::new(venue_order, fill_id),
        identity,
        execution,
    )
    .map_err(|_| PmPrivateNormalizationError::EventContract)
}

fn unresolved_trade(
    role: &PmFixturePrivateLifecycle,
    fill_id: PmFillId,
    order: Option<PmVenueOrderKey>,
    reason: PmUnresolvedTradeReason,
    settlement: PmFillSettlementStatus,
    fee: PmFixtureFeeEvidence,
) -> PmPrivateLifecycleObservation {
    PmPrivateLifecycleObservation::UnresolvedTrade(PmFixtureUnresolvedTrade {
        source: role.source,
        account: role.account(),
        instrument: role.instrument.handle(),
        fill_id,
        order,
        candidate_order: None,
        reason,
        settlement,
        fee,
    })
}

fn unresolved_maker_leg(
    role: &PmFixturePrivateLifecycle,
    fill_id: PmFillId,
    candidate_order: PmVenueOrderId,
    reason: PmUnresolvedTradeReason,
    settlement: PmFillSettlementStatus,
    fee: PmFixtureFeeEvidence,
) -> PmPrivateLifecycleObservation {
    PmPrivateLifecycleObservation::UnresolvedTrade(PmFixtureUnresolvedTrade {
        source: role.source,
        account: role.account(),
        instrument: role.instrument.handle(),
        fill_id,
        order: None,
        candidate_order: Some(candidate_order),
        reason,
        settlement,
        fee,
    })
}

fn validate_instrument(
    expected: PmFixtureInstrumentScope,
    market: &str,
    token: &str,
) -> Result<PmInstrumentHandle, PmPrivateNormalizationError> {
    let market =
        PmMarketId::parse(market).map_err(|_| PmPrivateNormalizationError::InvalidMarket)?;
    let token = token
        .parse::<U256>()
        .ok()
        .and_then(|units| PmTokenId::new(units).ok())
        .ok_or(PmPrivateNormalizationError::InvalidToken)?;
    if PmInstrumentId::new(market, token) != expected.id() {
        return Err(PmPrivateNormalizationError::InstrumentMismatch);
    }
    Ok(expected.handle())
}

fn validate_funder(
    account_scope: PmAccountScope,
    maker_address: &str,
) -> Result<(), PmPrivateNormalizationError> {
    let maker =
        EvmAddress::parse(maker_address).map_err(|_| PmPrivateNormalizationError::InvalidFunder)?;
    if maker != account_scope.funder().address() {
        return Err(PmPrivateNormalizationError::FunderMismatch);
    }
    Ok(())
}

fn parse_side(side: &str) -> Result<PmOrderSide, PmPrivateNormalizationError> {
    match side {
        "BUY" => Ok(PmOrderSide::Buy),
        "SELL" => Ok(PmOrderSide::Sell),
        _ => Err(PmPrivateNormalizationError::UnknownSide),
    }
}

fn parse_order_status(
    status: &str,
    cumulative: U256,
) -> Result<PmOrderStatus, PmPrivateNormalizationError> {
    match status {
        "LIVE" | "ORDER_STATUS_LIVE" if cumulative.is_zero() => Ok(PmOrderStatus::Open),
        "LIVE" | "ORDER_STATUS_LIVE" => Ok(PmOrderStatus::PartiallyFilled),
        "MATCHED" | "ORDER_STATUS_MATCHED" => Ok(PmOrderStatus::Filled),
        "CANCELED" | "ORDER_STATUS_CANCELED" | "CANCELLED" | "ORDER_STATUS_CANCELLED" => {
            Ok(PmOrderStatus::Cancelled)
        }
        "EXPIRED" | "ORDER_STATUS_EXPIRED" => Ok(PmOrderStatus::Expired),
        "INVALID" | "REJECTED" | "ORDER_STATUS_INVALID" | "ORDER_STATUS_REJECTED" => {
            Ok(PmOrderStatus::Rejected)
        }
        _ => Err(PmPrivateNormalizationError::UnknownOrderStatus),
    }
}

fn parse_trade_status(status: &str) -> Result<PmFillSettlementStatus, PmPrivateNormalizationError> {
    match status {
        "MATCHED" => Ok(PmFillSettlementStatus::Matched),
        "MINED" => Ok(PmFillSettlementStatus::Mined),
        "CONFIRMED" => Ok(PmFillSettlementStatus::Confirmed),
        "RETRYING" => Ok(PmFillSettlementStatus::Retrying),
        "FAILED" => Ok(PmFillSettlementStatus::Failed),
        _ => Err(PmPrivateNormalizationError::UnknownTradeStatus),
    }
}

#[derive(Clone, Copy)]
enum UserOrderKind {
    Placement,
    Update,
    Cancellation,
}

fn parse_user_order_kind(kind: &str) -> Result<UserOrderKind, PmPrivateNormalizationError> {
    match kind {
        "PLACEMENT" => Ok(UserOrderKind::Placement),
        "UPDATE" => Ok(UserOrderKind::Update),
        "CANCELLATION" => Ok(UserOrderKind::Cancellation),
        _ => Err(PmPrivateNormalizationError::UnknownOrderEventKind),
    }
}

fn validate_user_order_kind(
    kind: UserOrderKind,
    event: PmOrderEvent,
) -> Result<(), PmPrivateNormalizationError> {
    let progress = event.progress();
    let valid = match kind {
        UserOrderKind::Placement => {
            progress.status() == PmOrderStatus::Open && progress.cumulative_filled().is_zero()
        }
        UserOrderKind::Update => {
            matches!(
                progress.status(),
                PmOrderStatus::PartiallyFilled | PmOrderStatus::Filled
            ) && !progress.cumulative_filled().is_zero()
        }
        UserOrderKind::Cancellation => progress.status() == PmOrderStatus::Cancelled,
    };
    if valid {
        Ok(())
    } else {
        Err(PmPrivateNormalizationError::OrderEventKindStatusMismatch)
    }
}

fn parse_nonnegative_quantity(value: &str) -> Result<U256, PmPrivateNormalizationError> {
    match PmBookQuantity::parse_decimal(value)
        .map_err(|_| PmPrivateNormalizationError::InvalidMatchedQuantity)?
    {
        PmBookQuantity::Delete => Ok(U256::ZERO),
        PmBookQuantity::Quantity(quantity) => Ok(quantity.protocol_units()),
    }
}

fn parse_venue_order(
    account: PmAccountHandle,
    value: &str,
) -> Result<PmVenueOrderKey, PmPrivateNormalizationError> {
    let id =
        PmVenueOrderId::new(value).map_err(|_| PmPrivateNormalizationError::InvalidVenueOrder)?;
    Ok(PmVenueOrderKey::new(account, id))
}

fn push_observation(
    observations: &mut Vec<PmPrivateLifecycleObservation>,
    observation: PmPrivateLifecycleObservation,
) -> Result<(), PmPrivateNormalizationError> {
    if observations.len() == MAX_PM_PRIVATE_NORMALIZED_OBSERVATIONS {
        return Err(PmPrivateNormalizationError::TooManyObservations);
    }
    observations.push(observation);
    Ok(())
}

fn validate_role_source(
    account_scope: PmAccountScope,
    source: PmProductSource,
) -> Result<(), PmPrivateLifecycleRoleError> {
    match validate_account_source(account_scope, source) {
        Ok(()) => Ok(()),
        Err(PmFixtureScopeError::WrongSource) => Err(PmPrivateLifecycleRoleError::WrongSource),
        Err(PmFixtureScopeError::SourceAccountMismatch) => {
            Err(PmPrivateLifecycleRoleError::SourceAccountMismatch)
        }
    }
}

struct OrderFields<'a> {
    id: &'a str,
    market: &'a str,
    asset_id: &'a str,
    side: &'a str,
    original_size: &'a str,
    size_matched: &'a str,
    price: &'a str,
    status: &'a str,
    maker_address: &'a str,
}
