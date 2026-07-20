use reap_pm_core::PmInstrumentHandle;
use reap_pm_live_contracts::ConstructedRoleBinding;

/// Internal owner of the model-declared quote-evaluation timer requirement.
#[derive(Debug)]
pub(crate) struct PmQuoteScheduleRole {
    instrument: PmInstrumentHandle,
}

impl PmQuoteScheduleRole {
    pub(crate) const fn new(instrument: PmInstrumentHandle) -> Self {
        Self { instrument }
    }

    pub(crate) const fn binding(&self) -> ConstructedRoleBinding {
        ConstructedRoleBinding::quote_schedule(self.instrument)
    }

    pub(crate) const fn instrument(&self) -> PmInstrumentHandle {
        self.instrument
    }
}
