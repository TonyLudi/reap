use reap_pm_core::{
    PmAccountScope, PmClientOrderKey, PmGoalFTradingDomain, PmInstrumentHandle, PmInstrumentId,
    PmOrderSalt, PmOrderSide, PmPrice, PmQuantity, PmVenueOrderKey,
};
use reap_polymarket_wire::PmUnsignedClobV2Order;

use crate::fixture_scope::PmFixtureInstrumentScope;

use super::{
    PmCancelOwnedPurpose, PmFakeExecutionError, PmFixtureOwnedExecution, PmGtcPostOnlyProfile,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmFixedOrderType {
    Gtc,
}

/// Checked request data for the only supported place profile.
///
/// Construction remains on the scoped execution role. The request is
/// backend-neutral and move-only so one dispatch consumes the exact value
/// admitted after durability.
#[derive(Debug, PartialEq, Eq)]
pub struct PmGtcPostOnlyPlaceRequest {
    account_scope: PmAccountScope,
    instrument_scope: PmFixtureInstrumentScope,
    client_order: PmClientOrderKey,
    side: PmOrderSide,
    price: PmPrice,
    quantity: PmQuantity,
    unsigned_order: PmUnsignedClobV2Order,
    profile: PmGtcPostOnlyProfile,
}

impl PmGtcPostOnlyPlaceRequest {
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

    /// Returns the exact non-secret chain/exchange/domain metadata used when
    /// this request was prepared.
    ///
    /// A live fixed-profile edge derives its EIP-712 domain from this checked
    /// metadata; callers cannot select a different domain per order.
    #[must_use]
    pub const fn trading_domain(&self) -> PmGoalFTradingDomain {
        self.instrument_scope.trading_domain()
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

/// Checked request data for cancelling one exact locally identified order.
///
/// The request is backend-neutral and move-only. It carries no signer,
/// credential, transport, or generic cancellation capability.
#[derive(Debug, PartialEq, Eq)]
pub struct PmExactOwnedCancelRequest {
    account_scope: PmAccountScope,
    instrument_scope: PmFixtureInstrumentScope,
    client_order: PmClientOrderKey,
    venue_order: PmVenueOrderKey,
    purpose: PmCancelOwnedPurpose,
}

/// Scope-bound, backend-neutral request preparation.
///
/// This role can construct only the fixed Goal-F place/cancel request shapes;
/// it has no fixture scripts, result synthesis, signer, credentials, or
/// transport capability.
#[derive(Debug, PartialEq, Eq)]
pub struct PmFixedMutationPreparation {
    account_scope: PmAccountScope,
    instrument: PmInstrumentHandle,
    place_profile: PmGtcPostOnlyProfile,
    cancel_purpose: PmCancelOwnedPurpose,
}

impl PmFixedMutationPreparation {
    #[must_use]
    pub const fn new(account_scope: PmAccountScope, instrument: PmInstrumentHandle) -> Self {
        Self {
            account_scope,
            instrument,
            place_profile: PmGtcPostOnlyProfile::goal_f(),
            cancel_purpose: PmCancelOwnedPurpose::goal_f(),
        }
    }

    #[must_use]
    pub const fn account_scope(&self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn instrument(&self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn place_profile(&self) -> PmGtcPostOnlyProfile {
        self.place_profile
    }

    #[must_use]
    pub const fn cancel_purpose(&self) -> PmCancelOwnedPurpose {
        self.cancel_purpose
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare_place(
        &self,
        instrument_scope: PmFixtureInstrumentScope,
        client_order: PmClientOrderKey,
        salt: PmOrderSalt,
        side: PmOrderSide,
        price: PmPrice,
        quantity: PmQuantity,
        timestamp_ms: u64,
    ) -> Result<PmGtcPostOnlyPlaceRequest, PmFakeExecutionError> {
        validate_preparation_scope(self, instrument_scope)?;
        validate_quote_ready(instrument_scope)?;
        if client_order.account() != self.account_scope.handle() {
            return Err(PmFakeExecutionError::AccountMismatch);
        }
        let signer = self.account_scope.signer().address();
        let funder = self.account_scope.funder().address();
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
        Ok(PmGtcPostOnlyPlaceRequest {
            account_scope: self.account_scope,
            instrument_scope,
            client_order,
            side,
            price,
            quantity,
            unsigned_order,
            profile: self.place_profile,
        })
    }

    pub fn prepare_cancel(
        &self,
        instrument_scope: PmFixtureInstrumentScope,
        client_order: PmClientOrderKey,
        venue_order: PmVenueOrderKey,
    ) -> Result<PmExactOwnedCancelRequest, PmFakeExecutionError> {
        validate_preparation_scope(self, instrument_scope)?;
        if client_order.account() != self.account_scope.handle()
            || venue_order.account() != self.account_scope.handle()
        {
            return Err(PmFakeExecutionError::AccountMismatch);
        }
        Ok(PmExactOwnedCancelRequest {
            account_scope: self.account_scope,
            instrument_scope,
            client_order,
            venue_order,
            purpose: self.cancel_purpose,
        })
    }
}

impl PmExactOwnedCancelRequest {
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
    ) -> Result<PmGtcPostOnlyPlaceRequest, PmFakeExecutionError> {
        PmFixedMutationPreparation::new(self.account_scope(), self.instrument()).prepare_place(
            instrument_scope,
            client_order,
            salt,
            side,
            price,
            quantity,
            timestamp_ms,
        )
    }

    pub fn cancel_command(
        &self,
        instrument_scope: PmFixtureInstrumentScope,
        client_order: PmClientOrderKey,
        venue_order: PmVenueOrderKey,
    ) -> Result<PmExactOwnedCancelRequest, PmFakeExecutionError> {
        PmFixedMutationPreparation::new(self.account_scope(), self.instrument()).prepare_cancel(
            instrument_scope,
            client_order,
            venue_order,
        )
    }
}

/// Compatibility name for the completed Goal F fixture API.
pub type PmFakeOrderType = PmFixedOrderType;
/// Compatibility name for the completed Goal F fixture API.
pub type PmFakePlaceCommand = PmGtcPostOnlyPlaceRequest;
/// Compatibility name for the completed Goal F fixture API.
pub type PmFakeCancelCommand = PmExactOwnedCancelRequest;

fn validate_preparation_scope(
    role: &PmFixedMutationPreparation,
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
