use reap_pm_core::{
    PmAccountScope, PmClientOrderKey, PmFillExecution, PmFillFee, PmFillId, PmFillKey, PmFillRole,
    PmFillSettlementStatus, PmInstrumentHandle, PmInstrumentId, PmOrderSide, PmPrice, PmQuantity,
    PmVenueOrderKey, U256,
};

use super::{
    PmFakeCancelCommand, PmFakeExecutionError, PmFakePlaceCommand, PmFixtureOwnedExecution,
};

pub const MAX_PM_FAKE_ACK_FILL_LEGS: usize = 64;

/// One exact immediate fill supplied by a deterministic fake script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmFakeImmediateFill {
    id: PmFillId,
    price: PmPrice,
    quantity: PmQuantity,
    fee: PmFillFee,
}

impl PmFakeImmediateFill {
    #[must_use]
    pub const fn new(id: PmFillId, price: PmPrice, quantity: PmQuantity, fee: PmFillFee) -> Self {
        Self {
            id,
            price,
            quantity,
            fee,
        }
    }

    #[must_use]
    pub const fn id(self) -> PmFillId {
        self.id
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmFakePlaceRejectReason {
    FixtureRejected,
    PostOnlyWouldTake,
}

#[derive(Debug, PartialEq, Eq)]
pub struct PmFakePlaceScript {
    kind: PmFakePlaceScriptKind,
}

#[derive(Debug, PartialEq, Eq)]
enum PmFakePlaceScriptKind {
    Acknowledged {
        venue_order: PmVenueOrderKey,
        immediate_fills: Box<[PmFakeImmediateFill]>,
    },
    Rejected(PmFakePlaceRejectReason),
    AcknowledgementUnknown,
}

impl PmFakePlaceScript {
    pub fn acknowledged(
        venue_order: PmVenueOrderKey,
        immediate_fills: Box<[PmFakeImmediateFill]>,
    ) -> Result<Self, PmFakeExecutionError> {
        if immediate_fills.len() > MAX_PM_FAKE_ACK_FILL_LEGS {
            return Err(PmFakeExecutionError::TooManyImmediateFillLegs);
        }
        Ok(Self {
            kind: PmFakePlaceScriptKind::Acknowledged {
                venue_order,
                immediate_fills,
            },
        })
    }

    #[must_use]
    pub const fn rejected(reason: PmFakePlaceRejectReason) -> Self {
        Self {
            kind: PmFakePlaceScriptKind::Rejected(reason),
        }
    }

    #[must_use]
    pub const fn acknowledgement_unknown() -> Self {
        Self {
            kind: PmFakePlaceScriptKind::AcknowledgementUnknown,
        }
    }
}

/// Immediate fill normalized against the acknowledged venue-order leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmFakeAckImmediateFillLeg {
    key: PmFillKey,
    execution: PmFillExecution,
}

impl PmFakeAckImmediateFillLeg {
    #[must_use]
    pub const fn key(self) -> PmFillKey {
        self.key
    }

    #[must_use]
    pub const fn execution(self) -> PmFillExecution {
        self.execution
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PmFakePlaceAck {
    venue_order: PmVenueOrderKey,
    immediate_fills: Box<[PmFakeAckImmediateFillLeg]>,
}

impl PmFakePlaceAck {
    #[must_use]
    pub const fn venue_order(&self) -> PmVenueOrderKey {
        self.venue_order
    }

    #[must_use]
    pub fn immediate_fills(&self) -> &[PmFakeAckImmediateFillLeg] {
        &self.immediate_fills
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PmFakePlaceOutcome {
    Acknowledged(PmFakePlaceAck),
    Rejected(PmFakePlaceRejectReason),
    AcknowledgementUnknown,
}

#[derive(Debug, PartialEq, Eq)]
pub struct PmFakePlaceResult {
    account_scope: PmAccountScope,
    instrument: PmInstrumentHandle,
    instrument_id: PmInstrumentId,
    client_order: PmClientOrderKey,
    side: PmOrderSide,
    price: PmPrice,
    quantity: PmQuantity,
    outcome: PmFakePlaceOutcome,
}

impl PmFakePlaceResult {
    #[must_use]
    pub const fn account_scope(&self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn instrument(&self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn instrument_id(&self) -> PmInstrumentId {
        self.instrument_id
    }

    #[must_use]
    pub const fn client_order(&self) -> PmClientOrderKey {
        self.client_order
    }

    #[must_use]
    pub const fn side(&self) -> PmOrderSide {
        self.side
    }

    #[must_use]
    pub const fn price(&self) -> PmPrice {
        self.price
    }

    #[must_use]
    pub const fn quantity(&self) -> PmQuantity {
        self.quantity
    }

    #[must_use]
    pub const fn outcome(&self) -> &PmFakePlaceOutcome {
        &self.outcome
    }
}

impl PmFixtureOwnedExecution {
    pub fn execute_place(
        &self,
        command: PmFakePlaceCommand,
        script: PmFakePlaceScript,
    ) -> Result<PmFakePlaceResult, PmFakeExecutionError> {
        validate_place_command(self, &command)?;
        let outcome = match script.kind {
            PmFakePlaceScriptKind::Acknowledged {
                venue_order,
                immediate_fills,
            } => {
                PmFakePlaceOutcome::Acknowledged(build_ack(&command, venue_order, immediate_fills)?)
            }
            PmFakePlaceScriptKind::Rejected(reason) => PmFakePlaceOutcome::Rejected(reason),
            PmFakePlaceScriptKind::AcknowledgementUnknown => {
                PmFakePlaceOutcome::AcknowledgementUnknown
            }
        };

        Ok(PmFakePlaceResult {
            account_scope: command.account_scope(),
            instrument: command.instrument(),
            instrument_id: command.instrument_id(),
            client_order: command.client_order(),
            side: command.side(),
            price: command.price(),
            quantity: command.quantity(),
            outcome,
        })
    }
}

fn validate_place_command(
    role: &PmFixtureOwnedExecution,
    command: &PmFakePlaceCommand,
) -> Result<(), PmFakeExecutionError> {
    if command.account_scope() != role.account_scope()
        || command.client_order().account() != role.account()
    {
        return Err(PmFakeExecutionError::AccountMismatch);
    }
    if command.instrument() != role.instrument() {
        return Err(PmFakeExecutionError::InstrumentMismatch);
    }
    Ok(())
}

// The fake script owns a compact, length-capped slice. Taking that box by
// value preserves the one-shot script boundary without a second input copy.
#[allow(clippy::boxed_local)]
fn build_ack(
    command: &PmFakePlaceCommand,
    venue_order: PmVenueOrderKey,
    immediate_fills: Box<[PmFakeImmediateFill]>,
) -> Result<PmFakePlaceAck, PmFakeExecutionError> {
    if venue_order.account() != command.account_scope().handle() {
        return Err(PmFakeExecutionError::VenueOrderAccountMismatch);
    }
    let scope = command.instrument_scope();
    let mut total = U256::ZERO;
    let mut normalized = Vec::with_capacity(immediate_fills.len());
    for (index, fill) in immediate_fills.iter().copied().enumerate() {
        if immediate_fills[..index]
            .iter()
            .any(|prior| prior.id() == fill.id())
        {
            return Err(PmFakeExecutionError::DuplicateImmediateFill);
        }
        fill.price().validate_tick(scope.tick())?;
        let inside_limit = match command.side() {
            PmOrderSide::Buy => fill.price() <= command.price(),
            PmOrderSide::Sell => fill.price() >= command.price(),
        };
        if !inside_limit {
            return Err(PmFakeExecutionError::ImmediateFillOutsideLimit);
        }
        total = total.checked_add(fill.quantity().protocol_units())?;
        if total > command.quantity().protocol_units() {
            return Err(PmFakeExecutionError::ImmediateFillExceedsOrder);
        }
        normalized.push(PmFakeAckImmediateFillLeg {
            key: PmFillKey::new(venue_order, fill.id()),
            execution: PmFillExecution::new(
                command.side(),
                PmFillRole::Maker,
                PmFillSettlementStatus::Matched,
                fill.price(),
                fill.quantity(),
                fill.fee(),
            ),
        });
    }
    Ok(PmFakePlaceAck {
        venue_order,
        immediate_fills: normalized.into_boxed_slice(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmFakeCancelRejectReason {
    FixtureRejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmFakeCancelScript {
    outcome: PmFakeCancelOutcome,
}

impl PmFakeCancelScript {
    #[must_use]
    pub const fn accepted() -> Self {
        Self {
            outcome: PmFakeCancelOutcome::Accepted,
        }
    }

    #[must_use]
    pub const fn rejected(reason: PmFakeCancelRejectReason) -> Self {
        Self {
            outcome: PmFakeCancelOutcome::Rejected(reason),
        }
    }

    #[must_use]
    pub const fn already_filled() -> Self {
        Self {
            outcome: PmFakeCancelOutcome::AlreadyFilled,
        }
    }

    #[must_use]
    pub const fn acknowledgement_unknown() -> Self {
        Self {
            outcome: PmFakeCancelOutcome::AcknowledgementUnknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmFakeCancelOutcome {
    Accepted,
    Rejected(PmFakeCancelRejectReason),
    AlreadyFilled,
    AcknowledgementUnknown,
}

#[derive(Debug, PartialEq, Eq)]
pub struct PmFakeCancelResult {
    account_scope: PmAccountScope,
    instrument: PmInstrumentHandle,
    instrument_id: PmInstrumentId,
    client_order: PmClientOrderKey,
    venue_order: PmVenueOrderKey,
    outcome: PmFakeCancelOutcome,
}

impl PmFakeCancelResult {
    #[must_use]
    pub const fn account_scope(&self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn instrument(&self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn instrument_id(&self) -> PmInstrumentId {
        self.instrument_id
    }

    #[must_use]
    pub const fn client_order(&self) -> PmClientOrderKey {
        self.client_order
    }

    #[must_use]
    pub const fn venue_order(&self) -> PmVenueOrderKey {
        self.venue_order
    }

    #[must_use]
    pub const fn outcome(&self) -> PmFakeCancelOutcome {
        self.outcome
    }
}

impl PmFixtureOwnedExecution {
    pub fn execute_cancel(
        &self,
        command: PmFakeCancelCommand,
        script: PmFakeCancelScript,
    ) -> Result<PmFakeCancelResult, PmFakeExecutionError> {
        if command.account_scope() != self.account_scope()
            || command.client_order().account() != self.account()
            || command.venue_order().account() != self.account()
        {
            return Err(PmFakeExecutionError::AccountMismatch);
        }
        if command.instrument() != self.instrument() {
            return Err(PmFakeExecutionError::InstrumentMismatch);
        }
        Ok(PmFakeCancelResult {
            account_scope: command.account_scope(),
            instrument: command.instrument(),
            instrument_id: command.instrument_id(),
            client_order: command.client_order(),
            venue_order: command.venue_order(),
            outcome: script.outcome,
        })
    }
}
