use reap_pm_live_contracts::{
    ConstructedRoleBinding, PmAccountConnectivityConfig, PmConnectionRoute, PmConnectivityConfig,
    PmConnectivityPlan, PmFakeExecutionProfile, PmPlanError, PmPublicConnectivityConfig,
    PmRoleKind,
};
use reap_pm_strategy::PmQuoteModelRequirements;
use reap_polymarket_adapter::{
    PmAccountPositionRoleError, PmFixtureAccountPositionSnapshot, PmFixturePrivateLifecycle,
    PmFixtureReconciliation, PmPrivateLifecycleRoleError, PmPublicRoleError,
    PmReconciliationContractError,
};
use thiserror::Error;

use crate::capture::PmCaptureRoles;
use crate::fake_effect::PmFakeEffectRole;
use crate::schedule::PmQuoteScheduleRole;

#[derive(Debug, Error)]
pub enum PmCompositionError {
    #[error(transparent)]
    Plan(#[from] PmPlanError),
    #[error(transparent)]
    AccountRole(#[from] PmAccountPositionRoleError),
    #[error(transparent)]
    PublicRole(#[from] PmPublicRoleError),
    #[error(transparent)]
    PrivateRole(#[from] PmPrivateLifecycleRoleError),
    #[error(transparent)]
    ReconciliationRole(#[from] PmReconciliationContractError),
}

#[derive(Debug)]
pub struct PmPublicCapture {
    plan: PmConnectivityPlan,
    bindings: Vec<ConstructedRoleBinding>,
    capture: PmCaptureRoles,
}

impl PmPublicCapture {
    pub fn new(config: PmPublicConnectivityConfig) -> Result<Self, PmCompositionError> {
        let plan = PmConnectivityPlan::public_capture(config)?;
        Self::from_plan(plan)
    }

    fn from_plan(plan: PmConnectivityPlan) -> Result<Self, PmCompositionError> {
        let config = plan
            .public_config()
            .expect("public plan carries public config");
        let capture = PmCaptureRoles::new(config)?;
        let bindings = capture.bindings();
        plan.validate_bindings(&bindings)?;
        Ok(Self {
            plan,
            bindings,
            capture,
        })
    }

    #[must_use]
    pub fn reached_roles(&self) -> &[PmRoleKind] {
        self.plan.reached_roles()
    }

    #[must_use]
    pub fn binding_count(&self) -> usize {
        let config = self
            .plan
            .public_config()
            .expect("public plan carries public config");
        debug_assert_eq!(self.capture.reference(), config.okx_reference());
        debug_assert_eq!(self.capture.instrument(), config.instrument());
        self.bindings.len()
    }
}

#[derive(Debug)]
pub struct PmReadOnlyMonitor {
    plan: PmConnectivityPlan,
    bindings: Vec<ConstructedRoleBinding>,
    private: PmFixturePrivateLifecycle,
    reconciliation: PmFixtureReconciliation,
    account: PmFixtureAccountPositionSnapshot,
}

impl PmReadOnlyMonitor {
    pub fn new(config: PmAccountConnectivityConfig) -> Result<Self, PmCompositionError> {
        let plan = PmConnectivityPlan::read_only_monitor(config)?;
        Self::from_plan(plan)
    }

    fn from_plan(plan: PmConnectivityPlan) -> Result<Self, PmCompositionError> {
        let config = plan
            .account_config()
            .expect("monitor plan carries account config");
        let route = config.account_route();
        let private = PmFixturePrivateLifecycle::new(
            config.account_scope(),
            route.source(),
            route.connection(),
        )?;
        let reconciliation = PmFixtureReconciliation::new(
            config.account_scope(),
            route.source(),
            route.connection(),
        )?;
        let account = PmFixtureAccountPositionSnapshot::new(
            config.account_scope(),
            config.instrument(),
            route.source(),
            route.connection(),
            config.required_spenders().to_vec(),
        )?;
        let bindings = monitor_bindings(&private, &reconciliation, &account)?;
        plan.validate_bindings(&bindings)?;
        Ok(Self {
            plan,
            bindings,
            private,
            reconciliation,
            account,
        })
    }

    #[must_use]
    pub fn reached_roles(&self) -> &[PmRoleKind] {
        self.plan.reached_roles()
    }

    #[must_use]
    pub fn binding_count(&self) -> usize {
        let config = self
            .plan
            .account_config()
            .expect("monitor plan carries account config");
        debug_assert_eq!(self.private.account_scope(), config.account_scope());
        debug_assert_eq!(self.reconciliation.account_scope(), config.account_scope());
        debug_assert_eq!(self.account.account_scope(), config.account_scope());
        debug_assert_eq!(self.account.instrument(), config.instrument());
        self.bindings.len()
    }
}

#[derive(Debug)]
pub struct PmProduct<M> {
    model: M,
    plan: PmConnectivityPlan,
    bindings: Vec<ConstructedRoleBinding>,
    capture: PmCaptureRoles,
    private: PmFixturePrivateLifecycle,
    reconciliation: PmFixtureReconciliation,
    account: PmFixtureAccountPositionSnapshot,
    fake_effect: PmFakeEffectRole,
    schedule: PmQuoteScheduleRole,
}

impl<M: PmQuoteModelRequirements> PmProduct<M> {
    pub fn new(
        config: PmConnectivityConfig,
        model: M,
        profile: PmFakeExecutionProfile,
    ) -> Result<Self, PmCompositionError> {
        let requirements = model.input_requirements();
        let plan = PmConnectivityPlan::product(config, requirements, profile)?;
        let public = plan
            .public_config()
            .expect("product plan carries public config");
        let account_config = plan
            .account_config()
            .expect("product plan carries account config");
        let capture = PmCaptureRoles::new(public)?;
        let route = account_config.account_route();
        let private = PmFixturePrivateLifecycle::new(
            account_config.account_scope(),
            route.source(),
            route.connection(),
        )?;
        let reconciliation = PmFixtureReconciliation::new(
            account_config.account_scope(),
            route.source(),
            route.connection(),
        )?;
        let account = PmFixtureAccountPositionSnapshot::new(
            account_config.account_scope(),
            account_config.instrument(),
            route.source(),
            route.connection(),
            account_config.required_spenders().to_vec(),
        )?;
        let fake_effect =
            PmFakeEffectRole::new(account_config.account_scope(), account_config.instrument());
        let schedule = PmQuoteScheduleRole::new(public.instrument());
        let mut bindings = capture.bindings();
        bindings.extend(monitor_bindings(&private, &reconciliation, &account)?);
        bindings.extend(fake_effect.bindings());
        bindings.push(schedule.binding());
        plan.validate_bindings(&bindings)?;
        Ok(Self {
            model,
            plan,
            bindings,
            capture,
            private,
            reconciliation,
            account,
            fake_effect,
            schedule,
        })
    }

    #[must_use]
    pub fn reached_roles(&self) -> &[PmRoleKind] {
        self.plan.reached_roles()
    }

    #[must_use]
    pub fn binding_count(&self) -> usize {
        let requirements = self.model.input_requirements();
        let public = self
            .plan
            .public_config()
            .expect("product plan carries public config");
        let account = self
            .plan
            .account_config()
            .expect("product plan carries account config");
        debug_assert_eq!(requirements.reference(), self.capture.reference());
        debug_assert_eq!(self.capture.instrument(), public.instrument());
        debug_assert_eq!(self.private.account_scope(), account.account_scope());
        debug_assert_eq!(self.reconciliation.account_scope(), account.account_scope());
        debug_assert_eq!(self.account.account_scope(), account.account_scope());
        debug_assert_eq!(self.account.instrument(), account.instrument());
        debug_assert_eq!(self.fake_effect.account_scope(), account.account_scope());
        debug_assert_eq!(self.fake_effect.instrument(), account.instrument());
        debug_assert_eq!(self.schedule.instrument(), public.instrument());
        self.bindings.len()
    }
}

fn monitor_bindings(
    private: &PmFixturePrivateLifecycle,
    reconciliation: &PmFixtureReconciliation,
    account: &PmFixtureAccountPositionSnapshot,
) -> Result<Vec<ConstructedRoleBinding>, PmPlanError> {
    let mut bindings = Vec::with_capacity(16);
    bindings.extend(ConstructedRoleBinding::private_lifecycle(
        private.account_scope(),
        PmConnectionRoute::new(private.source(), private.connection()),
    ));
    bindings.extend(ConstructedRoleBinding::reconciliation(
        reconciliation.account_scope(),
        PmConnectionRoute::new(reconciliation.source(), reconciliation.connection()),
    ));
    bindings.extend(ConstructedRoleBinding::account_snapshot(
        account.account_scope(),
        account.instrument(),
        account.required_spenders(),
        PmConnectionRoute::new(account.source(), account.connection()),
    )?);
    Ok(bindings)
}
