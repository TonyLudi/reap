use reap_pm_core::{PmAccountScope, PmInstrumentHandle, PmInstrumentId};
use reap_pm_live_contracts::ConstructedRoleBinding;
use reap_polymarket_adapter::{
    PmCancelOwnedPurpose, PmFakeCancelResult, PmFakeCancelScript, PmFakePlaceResult,
    PmFakePlaceScript, PmFixtureInstrumentScope, PmFixtureOwnedExecution, PmGtcPostOnlyProfile,
};

use crate::coordinator::authority::{
    PmAuthorityError, PmAuthorityRevisions, PreparedPmCancel, PreparedPmQuote, ReservedPmCancel,
    ReservedPmQuote, consume_prepared_cancel, consume_prepared_quote, prepare_pm_cancel,
    prepare_pm_quote,
};
use crate::journal::{PmCancelIntentDurablyAcknowledged, PmQuoteIntentDurablyAcknowledged};

/// Narrow Phase 2 ownership bundle for the fixture execution role.
#[derive(Debug)]
pub(crate) struct PmFakeEffectRole {
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
            execution: PmFixtureOwnedExecution::new(account_scope, instrument),
            instrument_id,
        }
    }

    pub(crate) const fn bindings(&self) -> [ConstructedRoleBinding; 2] {
        ConstructedRoleBinding::owned_execution(
            self.execution.account_scope(),
            self.execution.instrument(),
            self.instrument_id,
        )
    }

    pub(crate) const fn account_scope(&self) -> PmAccountScope {
        self.execution.account_scope()
    }

    pub(crate) const fn instrument(&self) -> PmInstrumentHandle {
        self.execution.instrument()
    }

    pub(crate) const fn instrument_id(&self) -> PmInstrumentId {
        self.instrument_id
    }

    pub(crate) const fn place_profile(&self) -> PmGtcPostOnlyProfile {
        self.execution.place_profile()
    }

    pub(crate) const fn cancel_purpose(&self) -> PmCancelOwnedPurpose {
        self.execution.cancel_purpose()
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
            &self.execution,
            self.instrument_id,
            reserved,
            current_scope,
            current_revisions,
            monotonic_now_ns,
            acknowledged,
        )
    }

    pub(crate) fn execute_quote(
        &self,
        prepared: PreparedPmQuote,
        script: PmFakePlaceScript,
    ) -> Result<PmFakePlaceResult, PmAuthorityError> {
        let command = consume_prepared_quote(
            prepared,
            self.account_scope(),
            self.instrument(),
            self.instrument_id,
        )?;
        Ok(self.execution.execute_place(command, script)?)
    }

    pub(crate) fn prepare_cancel(
        &self,
        reserved: ReservedPmCancel,
        current_scope: PmFixtureInstrumentScope,
        monotonic_now_ns: u64,
        acknowledged: PmCancelIntentDurablyAcknowledged,
    ) -> Result<PreparedPmCancel, PmAuthorityError> {
        prepare_pm_cancel(
            &self.execution,
            self.instrument_id,
            reserved,
            current_scope,
            monotonic_now_ns,
            acknowledged,
        )
    }

    pub(crate) fn execute_cancel(
        &self,
        prepared: PreparedPmCancel,
        script: PmFakeCancelScript,
    ) -> Result<PmFakeCancelResult, PmAuthorityError> {
        let command = consume_prepared_cancel(
            prepared,
            self.account_scope(),
            self.instrument(),
            self.instrument_id,
        )?;
        Ok(self.execution.execute_cancel(command, script)?)
    }
}
