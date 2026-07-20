use reap_pm_core::{PmAccountScope, PmInstrumentHandle};
use reap_pm_live_contracts::ConstructedRoleBinding;
use reap_polymarket_adapter::PmFixtureOwnedExecution;

/// Narrow Phase 2 ownership bundle for the fixture execution role.
#[derive(Debug)]
pub(crate) struct PmFakeEffectRole {
    execution: PmFixtureOwnedExecution,
}

impl PmFakeEffectRole {
    pub(crate) const fn new(account_scope: PmAccountScope, instrument: PmInstrumentHandle) -> Self {
        Self {
            execution: PmFixtureOwnedExecution::new(account_scope, instrument),
        }
    }

    pub(crate) const fn bindings(&self) -> [ConstructedRoleBinding; 2] {
        ConstructedRoleBinding::owned_execution(
            self.execution.account_scope(),
            self.execution.instrument(),
        )
    }

    pub(crate) const fn account_scope(&self) -> PmAccountScope {
        self.execution.account_scope()
    }

    pub(crate) const fn instrument(&self) -> PmInstrumentHandle {
        self.execution.instrument()
    }
}
