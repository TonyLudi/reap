use reap_pm_core::{
    PmAccountScope, PmClientOrderKey, PmInstrumentHandle, PmInstrumentId, PmOrderSalt, PmOrderSide,
    PmPrice, PmQuantity, PmVenueOrderKey,
};
use reap_polymarket_wire::PmUnsignedClobV2Order;

use crate::fixture_scope::PmFixtureInstrumentScope;

use super::{
    PmCancelOwnedPurpose, PmFakeExecutionError, PmFixtureOwnedExecution, PmGtcPostOnlyProfile,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmFakeOrderType {
    Gtc,
}

/// Checked data for the only fake place profile.
///
/// Construction remains on the scoped execution role. This command is
/// move-only so an effect call consumes the exact value it was given.
#[derive(Debug, PartialEq, Eq)]
pub struct PmFakePlaceCommand {
    account_scope: PmAccountScope,
    instrument_scope: PmFixtureInstrumentScope,
    client_order: PmClientOrderKey,
    side: PmOrderSide,
    price: PmPrice,
    quantity: PmQuantity,
    unsigned_order: PmUnsignedClobV2Order,
    profile: PmGtcPostOnlyProfile,
}

impl PmFakePlaceCommand {
    #[must_use]
    pub const fn account_scope(&self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn instrument(&self) -> PmInstrumentHandle {
        self.instrument_scope.handle()
    }

    #[must_use]
    pub const fn instrument_id(&self) -> PmInstrumentId {
        self.instrument_scope.id()
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
    pub const fn unsigned_order(&self) -> PmUnsignedClobV2Order {
        self.unsigned_order
    }

    #[must_use]
    pub const fn profile(&self) -> PmGtcPostOnlyProfile {
        self.profile
    }

    pub(super) const fn instrument_scope(&self) -> PmFixtureInstrumentScope {
        self.instrument_scope
    }
}

/// Checked data for cancelling one exact locally identified order.
#[derive(Debug, PartialEq, Eq)]
pub struct PmFakeCancelCommand {
    account_scope: PmAccountScope,
    instrument_scope: PmFixtureInstrumentScope,
    client_order: PmClientOrderKey,
    venue_order: PmVenueOrderKey,
    purpose: PmCancelOwnedPurpose,
}

impl PmFakeCancelCommand {
    #[must_use]
    pub const fn account_scope(&self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn instrument(&self) -> PmInstrumentHandle {
        self.instrument_scope.handle()
    }

    #[must_use]
    pub const fn instrument_id(&self) -> PmInstrumentId {
        self.instrument_scope.id()
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
    pub const fn purpose(&self) -> PmCancelOwnedPurpose {
        self.purpose
    }
}

impl PmFixtureOwnedExecution {
    #[allow(clippy::too_many_arguments)]
    pub fn place_command(
        &self,
        instrument_scope: PmFixtureInstrumentScope,
        client_order: PmClientOrderKey,
        salt: PmOrderSalt,
        side: PmOrderSide,
        price: PmPrice,
        quantity: PmQuantity,
        timestamp_ms: u64,
    ) -> Result<PmFakePlaceCommand, PmFakeExecutionError> {
        validate_structural_scope(self, instrument_scope)?;
        validate_quote_ready(instrument_scope)?;
        if client_order.account() != self.account() {
            return Err(PmFakeExecutionError::AccountMismatch);
        }
        let account_scope = self.account_scope();
        let signer = account_scope.signer().address();
        let funder = account_scope.funder().address();
        if signer != funder {
            return Err(PmFakeExecutionError::EoaIdentityMismatch);
        }
        let metadata = instrument_scope.metadata();
        let unsigned_order = PmUnsignedClobV2Order::new_goal_f(
            salt,
            funder,
            signer,
            instrument_scope.id().token(),
            side,
            price,
            quantity,
            metadata.tick(),
            metadata.minimum_order_size(),
            timestamp_ms,
        )?;

        Ok(PmFakePlaceCommand {
            account_scope,
            instrument_scope,
            client_order,
            side,
            price,
            quantity,
            unsigned_order,
            profile: self.place_profile(),
        })
    }

    pub fn cancel_command(
        &self,
        instrument_scope: PmFixtureInstrumentScope,
        client_order: PmClientOrderKey,
        venue_order: PmVenueOrderKey,
    ) -> Result<PmFakeCancelCommand, PmFakeExecutionError> {
        validate_structural_scope(self, instrument_scope)?;
        if client_order.account() != self.account() || venue_order.account() != self.account() {
            return Err(PmFakeExecutionError::AccountMismatch);
        }
        Ok(PmFakeCancelCommand {
            account_scope: self.account_scope(),
            instrument_scope,
            client_order,
            venue_order,
            purpose: self.cancel_purpose(),
        })
    }
}

fn validate_structural_scope(
    role: &PmFixtureOwnedExecution,
    instrument_scope: PmFixtureInstrumentScope,
) -> Result<(), PmFakeExecutionError> {
    if instrument_scope.handle() != role.instrument() {
        return Err(PmFakeExecutionError::InstrumentMismatch);
    }
    if instrument_scope.metadata().chain() != role.account_scope().chain() {
        return Err(PmFakeExecutionError::ChainMismatch);
    }
    Ok(())
}

fn validate_quote_ready(
    instrument_scope: PmFixtureInstrumentScope,
) -> Result<(), PmFakeExecutionError> {
    let lifecycle = instrument_scope.metadata().lifecycle();
    if !lifecycle.active()
        || lifecycle.closed()
        || lifecycle.archived()
        || !lifecycle.accepting_orders()
        || !lifecycle.order_book_enabled()
    {
        return Err(PmFakeExecutionError::MarketNotReady);
    }
    Ok(())
}
