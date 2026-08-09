use reap_pm_core::{PmAccountScope, PmInstrumentHandle, PmInstrumentId};
use reap_pm_live_contracts::{ConstructedRoleBinding, PmAccountSignatureProfile};
use reap_polymarket_adapter::{
    PmCancelOwnedPurpose, PmFakeCancelResult, PmFakeCancelScript, PmFakePlaceResult,
    PmFakePlaceScript, PmFixedMutationPreparation, PmFixtureInstrumentScope,
    PmFixtureOwnedExecution, PmGtcPostOnlyProfile,
};

use crate::coordinator::authority::{
    PmAuthorityError, PmAuthorityRevisions, PreparedPmCancel, PreparedPmQuote, ReservedPmCancel,
    ReservedPmQuote, prepare_pm_cancel, prepare_pm_quote,
};
use crate::coordinator::{PmPreparedCancelDispatch, PmPreparedPlaceDispatch};
use crate::journal::{PmCancelIntentDurablyAcknowledged, PmQuoteIntentDurablyAcknowledged};

/// Narrow Phase 2 ownership bundle for the fixture execution role.
#[derive(Debug)]
pub(crate) struct PmFakeEffectRole {
    preparation: PmMutationPreparationRole,
    executor: PmFixtureEffectExecutor,
}

/// Backend-neutral fixed-profile preparation. It cannot execute fixture
/// scripts and is the only half owned by an authenticated coordinator.
#[derive(Debug)]
pub(crate) struct PmMutationPreparationRole {
    preparation: PmFixedMutationPreparation,
    instrument_id: PmInstrumentId,
    account_signature_profile: PmAccountSignatureProfile,
}

/// Fixture-only result synthesis retained solely by `PmProductRun`.
#[derive(Debug)]
pub(crate) struct PmFixtureEffectExecutor {
    execution: PmFixtureOwnedExecution,
    instrument_id: PmInstrumentId,
}

impl PmFakeEffectRole {
    pub(crate) const fn new(
        account_scope: PmAccountScope,
        instrument: PmInstrumentHandle,
        instrument_id: PmInstrumentId,
    ) -> Self {
        Self {
            preparation: PmMutationPreparationRole::new(account_scope, instrument, instrument_id),
            executor: PmFixtureEffectExecutor::new(account_scope, instrument, instrument_id),
        }
    }

    pub(crate) const fn for_account_signature_profile(
        account_signature_profile: PmAccountSignatureProfile,
        account_scope: PmAccountScope,
        instrument: PmInstrumentHandle,
        instrument_id: PmInstrumentId,
    ) -> Self {
        Self {
            preparation: PmMutationPreparationRole::for_account_signature_profile(
                account_signature_profile,
                account_scope,
                instrument,
                instrument_id,
            ),
            // This executor remains fixture-only and carries no signer or
            // transport. Authenticated composition destroys it before start.
            executor: PmFixtureEffectExecutor::new(account_scope, instrument, instrument_id),
        }
    }

    pub(crate) fn split(self) -> (PmMutationPreparationRole, PmFixtureEffectExecutor) {
        (self.preparation, self.executor)
    }

    pub(crate) const fn account_scope(&self) -> PmAccountScope {
        self.preparation.account_scope()
    }

    pub(crate) const fn instrument(&self) -> PmInstrumentHandle {
        self.preparation.instrument()
    }

    pub(crate) const fn bindings(&self) -> [ConstructedRoleBinding; 2] {
        ConstructedRoleBinding::owned_execution(
            self.preparation.account_scope(),
            self.preparation.instrument(),
            self.preparation.instrument_id(),
        )
    }
}

impl PmMutationPreparationRole {
    pub(crate) const fn new(
        account_scope: PmAccountScope,
        instrument: PmInstrumentHandle,
        instrument_id: PmInstrumentId,
    ) -> Self {
        Self {
            preparation: PmFixedMutationPreparation::new(account_scope, instrument),
            instrument_id,
            account_signature_profile: PmAccountSignatureProfile::EoaType0,
        }
    }

    pub(crate) const fn new_pm_t2_proxy(
        account_scope: PmAccountScope,
        instrument: PmInstrumentHandle,
        instrument_id: PmInstrumentId,
    ) -> Self {
        Self {
            preparation: PmFixedMutationPreparation::new_pm_t2_proxy(account_scope, instrument),
            instrument_id,
            account_signature_profile: PmAccountSignatureProfile::ProxyType1,
        }
    }

    pub(crate) const fn for_account_signature_profile(
        account_signature_profile: PmAccountSignatureProfile,
        account_scope: PmAccountScope,
        instrument: PmInstrumentHandle,
        instrument_id: PmInstrumentId,
    ) -> Self {
        match account_signature_profile {
            PmAccountSignatureProfile::EoaType0 => {
                Self::new(account_scope, instrument, instrument_id)
            }
            PmAccountSignatureProfile::ProxyType1 => {
                Self::new_pm_t2_proxy(account_scope, instrument, instrument_id)
            }
        }
    }

    pub(crate) const fn account_scope(&self) -> PmAccountScope {
        self.preparation.account_scope()
    }

    pub(crate) const fn instrument(&self) -> PmInstrumentHandle {
        self.preparation.instrument()
    }

    pub(crate) const fn instrument_id(&self) -> PmInstrumentId {
        self.instrument_id
    }

    pub(crate) const fn account_signature_profile(&self) -> PmAccountSignatureProfile {
        self.account_signature_profile
    }

    pub(crate) const fn place_profile(&self) -> PmGtcPostOnlyProfile {
        self.preparation.place_profile()
    }

    pub(crate) const fn cancel_purpose(&self) -> PmCancelOwnedPurpose {
        self.preparation.cancel_purpose()
    }

    pub(crate) fn prepare_quote(
        &self,
        reserved: ReservedPmQuote,
        current_scope: PmFixtureInstrumentScope,
        current_revisions: PmAuthorityRevisions,
        monotonic_now_ns: u64,
        acknowledged: PmQuoteIntentDurablyAcknowledged,
    ) -> Result<PreparedPmQuote, PmAuthorityError> {
        prepare_pm_quote(
            &self.preparation,
            self.instrument_id,
            reserved,
            current_scope,
            current_revisions,
            monotonic_now_ns,
            acknowledged,
        )
    }

    pub(crate) fn prepare_cancel(
        &self,
        reserved: ReservedPmCancel,
        current_scope: PmFixtureInstrumentScope,
        monotonic_now_ns: u64,
        acknowledged: PmCancelIntentDurablyAcknowledged,
    ) -> Result<PreparedPmCancel, PmAuthorityError> {
        prepare_pm_cancel(
            &self.preparation,
            self.instrument_id,
            reserved,
            current_scope,
            monotonic_now_ns,
            acknowledged,
        )
    }
}

impl PmFixtureEffectExecutor {
    pub(crate) const fn new(
        account_scope: PmAccountScope,
        instrument: PmInstrumentHandle,
        instrument_id: PmInstrumentId,
    ) -> Self {
        Self {
            execution: PmFixtureOwnedExecution::new(account_scope, instrument),
            instrument_id,
        }
    }

    pub(crate) const fn account_scope(&self) -> PmAccountScope {
        self.execution.account_scope()
    }

    pub(crate) const fn instrument(&self) -> PmInstrumentHandle {
        self.execution.instrument()
    }

    /// Fake backend implementation of the fixed place dispatch.
    pub(crate) fn execute_place_fixture(
        &self,
        dispatch: PmPreparedPlaceDispatch,
        script: PmFakePlaceScript,
    ) -> Result<PmFakePlaceResult, PmAuthorityError> {
        if dispatch.account_scope() != self.account_scope()
            || dispatch.instrument() != self.instrument()
            || dispatch.instrument_id() != self.instrument_id
        {
            return Err(PmAuthorityError::ScopeMismatch);
        }
        Ok(self
            .execution
            .execute_place(dispatch.into_request(), script)?)
    }

    /// Fake backend implementation of the exact-owned cancel dispatch.
    pub(crate) fn execute_cancel_fixture(
        &self,
        dispatch: PmPreparedCancelDispatch,
        script: PmFakeCancelScript,
    ) -> Result<PmFakeCancelResult, PmAuthorityError> {
        if dispatch.account_scope() != self.account_scope()
            || dispatch.instrument() != self.instrument()
            || dispatch.instrument_id() != self.instrument_id
        {
            return Err(PmAuthorityError::ScopeMismatch);
        }
        Ok(self
            .execution
            .execute_cancel(dispatch.into_request(), script)?)
    }
}
