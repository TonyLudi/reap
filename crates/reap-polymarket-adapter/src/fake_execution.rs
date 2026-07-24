mod command;
mod outcome;

pub use command::{PmFakeCancelCommand, PmFakeOrderType, PmFakePlaceCommand};
pub use outcome::{
    MAX_PM_FAKE_ACK_FILL_LEGS, PmFakeAckImmediateFillLeg, PmFakeCancelOutcome,
    PmFakeCancelRejectReason, PmFakeCancelResult, PmFakeCancelScript, PmFakeImmediateFill,
    PmFakePlaceAck, PmFakePlaceOutcome, PmFakePlaceRejectReason, PmFakePlaceResult,
    PmFakePlaceScript,
};
use reap_pm_core::{PmAccountHandle, PmAccountScope, PmInstrumentHandle, PmNumericError};
use reap_polymarket_wire::PmUnsignedOrderError;
use thiserror::Error;

mod sealed {
    pub trait Sealed {}
}

/// In-process fake owned-execution capability.
///
/// It is scoped to one exact account and instrument, and accepts only the
/// fixed Goal F command shapes.
pub trait PmOwnedExecutionRole: sealed::Sealed {
    type PlaceProfile;
    type CancelPurpose;

    fn account_scope(&self) -> PmAccountScope;
    fn account(&self) -> PmAccountHandle;
    fn instrument(&self) -> PmInstrumentHandle;
    fn place_profile(&self) -> Self::PlaceProfile;
    fn cancel_purpose(&self) -> Self::CancelPurpose;
}

#[derive(Debug, PartialEq, Eq)]
pub struct PmFixtureOwnedExecution {
    account_scope: PmAccountScope,
    instrument: PmInstrumentHandle,
    place_profile: PmGtcPostOnlyProfile,
    cancel_purpose: PmCancelOwnedPurpose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmGtcPostOnlyProfile {
    private: (),
}

impl PmGtcPostOnlyProfile {
    #[must_use]
    pub(crate) const fn goal_f() -> Self {
        Self { private: () }
    }

    #[must_use]
    pub const fn order_type(self) -> PmFakeOrderType {
        PmFakeOrderType::Gtc
    }

    #[must_use]
    pub const fn post_only(self) -> bool {
        true
    }

    #[must_use]
    pub const fn defer_exec(self) -> bool {
        false
    }

    #[must_use]
    pub const fn expiration(self) -> u64 {
        0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmCancelOwnedPurpose {
    private: (),
}

impl PmCancelOwnedPurpose {
    #[must_use]
    pub(crate) const fn goal_f() -> Self {
        Self { private: () }
    }
}

impl PmFixtureOwnedExecution {
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
    pub const fn account(&self) -> PmAccountHandle {
        self.account_scope.handle()
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
}

impl sealed::Sealed for PmFixtureOwnedExecution {}

impl PmOwnedExecutionRole for PmFixtureOwnedExecution {
    type PlaceProfile = PmGtcPostOnlyProfile;
    type CancelPurpose = PmCancelOwnedPurpose;

    fn account_scope(&self) -> PmAccountScope {
        self.account_scope
    }

    fn account(&self) -> PmAccountHandle {
        self.account_scope.handle()
    }

    fn instrument(&self) -> PmInstrumentHandle {
        self.instrument
    }

    fn place_profile(&self) -> Self::PlaceProfile {
        self.place_profile
    }

    fn cancel_purpose(&self) -> Self::CancelPurpose {
        self.cancel_purpose
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmFakeExecutionError {
    #[error("fake command account does not match the execution role")]
    AccountMismatch,
    #[error("fake command instrument does not match the execution role")]
    InstrumentMismatch,
    #[error("fake command account chain does not match market metadata")]
    ChainMismatch,
    #[error("fixed fake execution profile requires one EOA funder identity")]
    EoaIdentityMismatch,
    #[error("market lifecycle is not quote-ready")]
    MarketNotReady,
    #[error("fake acknowledgement venue order belongs to another account")]
    VenueOrderAccountMismatch,
    #[error("fake acknowledgement immediate-fill leg count exceeds its fixed bound")]
    TooManyImmediateFillLegs,
    #[error("fake acknowledgement repeats an immediate-fill identity")]
    DuplicateImmediateFill,
    #[error("fake acknowledgement immediate fill violates the order limit")]
    ImmediateFillOutsideLimit,
    #[error("fake acknowledgement immediate fills exceed the original order quantity")]
    ImmediateFillExceedsOrder,
    #[error(transparent)]
    Numeric(#[from] PmNumericError),
    #[error(transparent)]
    UnsignedOrder(#[from] PmUnsignedOrderError),
}
