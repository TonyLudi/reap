use reap_pm_live_contracts::{
    ConstructedRoleBinding, PmConnectivityConfig, PmConnectivityPlan, PmFakeExecutionProfile,
    PmRoleKind,
};
use reap_pm_state::PmRiskLimits;
use reap_pm_strategy::PmQuoteModelRequirements;

use super::PmCompositionError;
use crate::capture_roles::PmCaptureBlueprint;
use crate::fake_effect::PmFakeEffectRole;
use crate::private_monitor::PmPrivateMonitorRuntime;
use crate::schedule::PmQuoteScheduleRole;

mod run;

pub use run::{PmProductRun, PmProductRunError, PmProductStartError};

/// Secret-free sibling PM product composition.
///
/// Construction joins only the explicitly planned public observation,
/// fixture/read-only private state, pure model, fake execution, and
/// coordinator-owned schedule roles. [`PmProduct::start`] consumes all of
/// them into one production owner.
#[derive(Debug)]
pub struct PmProduct<M> {
    pub(super) model: M,
    pub(super) plan: PmConnectivityPlan,
    pub(super) bindings: Vec<ConstructedRoleBinding>,
    pub(super) capture: PmCaptureBlueprint,
    pub(super) private: PmPrivateMonitorRuntime,
    pub(super) fake_effect: PmFakeEffectRole,
    pub(super) schedule: PmQuoteScheduleRole,
}

impl<M: PmQuoteModelRequirements> PmProduct<M> {
    pub fn new(
        config: PmConnectivityConfig,
        model: M,
        profile: PmFakeExecutionProfile,
        risk_limits: PmRiskLimits,
    ) -> Result<Self, PmCompositionError> {
        let requirements = model.input_requirements();
        let plan = PmConnectivityPlan::product(config, requirements, profile)?;
        let public = plan
            .public_config()
            .expect("product plan carries public config");
        let account_config = plan
            .account_config()
            .expect("product plan carries account config");
        let capture = PmCaptureBlueprint::new(public)?;
        let private = PmPrivateMonitorRuntime::new(account_config, risk_limits)?;
        let fake_effect = PmFakeEffectRole::new(
            account_config.account_scope(),
            account_config.instrument(),
            account_config.instrument_id(),
        );
        let schedule = PmQuoteScheduleRole::new(public.instrument());
        let mut bindings = capture.bindings(public);
        bindings.extend(private.bindings(account_config)?);
        bindings.extend(fake_effect.bindings());
        bindings.push(schedule.binding());
        plan.validate_bindings(&bindings)?;
        Ok(Self {
            model,
            plan,
            bindings,
            capture,
            private,
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
        debug_assert_eq!(requirements.reference(), self.capture.reference(public));
        debug_assert_eq!(self.capture.instrument(), public.instrument());
        debug_assert_eq!(self.private.account_scope(), account.account_scope());
        debug_assert_eq!(self.private.instrument(), account.instrument());
        debug_assert_eq!(self.fake_effect.account_scope(), account.account_scope());
        debug_assert_eq!(self.fake_effect.instrument(), account.instrument());
        debug_assert_eq!(self.schedule.instrument(), public.instrument());
        self.bindings.len()
    }
}
