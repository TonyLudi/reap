use reap_pm_core::{PmAccountHandle, PmAccountScope, PmInstrumentHandle};

mod sealed {
    pub trait Sealed {}
}

/// In-process fake owned-execution capability marker.
///
/// Phase 2 intentionally provides no place/cancel method and no prepared
/// command. Phase 5 adds the take-once prepared-value transition without
/// widening this role to arbitrary commands.
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
