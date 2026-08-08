use reap_polymarket_live_adapter::{
    PmExactOwnedCancelLoopbackRole, PmFixedPlaceLoopbackRole, PmLoopbackMutationConnectivityOwner,
    PmPublicConnectivityOwner,
};
use thiserror::Error;

use super::super::PmProduct;
use crate::{
    capture_roles::PmCaptureBlueprint, fake_effect::PmMutationPreparationRole,
    private_monitor::PmPrivateMonitorRuntime, schedule::PmQuoteScheduleRole,
};
use reap_pm_live_contracts::{ConstructedRoleBinding, PmConnectivityPlan};

/// Unstarted, statically authenticated loopback composition.
///
/// Every authority required by the eventual run is moved into this value.
/// In particular, there is no fixture executor, optional mutation backend,
/// raw client, or independently held credential/signer handle. Startup will
/// consume this root only after metadata, both journals, and the exact
/// configuration pairing gates have succeeded.
pub(super) struct PmAuthenticatedLoopbackProduct<M> {
    common: PmAuthenticatedLoopbackProductParts<M>,
    public_connectivity: PmPublicConnectivityOwner,
    private_connectivity: PmLoopbackMutationConnectivityOwner,
    place_transport: PmFixedPlaceLoopbackRole,
    cancel_transport: PmExactOwnedCancelLoopbackRole,
}

/// Backend-neutral product owners extracted before the authenticated root is
/// constructed. The fixture executor is split and destroyed at the boundary;
/// it is never a field of this value or any authenticated run.
pub(super) struct PmAuthenticatedLoopbackProductParts<M> {
    pub(super) model: M,
    pub(super) plan: PmConnectivityPlan,
    pub(super) bindings: Vec<ConstructedRoleBinding>,
    pub(super) capture: PmCaptureBlueprint,
    pub(super) private: Box<PmPrivateMonitorRuntime>,
    pub(super) preparation: PmMutationPreparationRole,
    pub(super) schedule: PmQuoteScheduleRole,
}

impl<M> PmAuthenticatedLoopbackProduct<M> {
    #[allow(
        clippy::too_many_arguments,
        reason = "the cold root keeps every independently owned network and mutation authority explicit"
    )]
    pub(super) fn new(
        product: PmProduct<M>,
        public_connectivity: PmPublicConnectivityOwner,
        private_connectivity: PmLoopbackMutationConnectivityOwner,
        place_transport: PmFixedPlaceLoopbackRole,
        cancel_transport: PmExactOwnedCancelLoopbackRole,
    ) -> Result<Self, PmAuthenticatedLoopbackCompositionError> {
        let PmProduct {
            model,
            plan,
            bindings,
            capture,
            private,
            fake_effect,
            schedule,
        } = product;
        let (preparation, _) = fake_effect.split();
        let expected = plan
            .public_config()
            .expect("product plan carries public config")
            .expected_metadata();
        let public_scope = public_connectivity.configured_scope();
        if public_scope.condition() != expected.condition()
            || public_scope.market() != expected.market()
            || public_scope.token() != expected.outcome().token()
        {
            return Err(PmAuthenticatedLoopbackCompositionError::PublicScopeMismatch);
        }
        Ok(Self {
            common: PmAuthenticatedLoopbackProductParts {
                model,
                plan,
                bindings,
                capture,
                private,
                preparation,
                schedule,
            },
            public_connectivity,
            private_connectivity,
            place_transport,
            cancel_transport,
        })
    }

    pub(super) fn into_parts(
        self,
    ) -> (
        PmAuthenticatedLoopbackProductParts<M>,
        PmPublicConnectivityOwner,
        PmLoopbackMutationConnectivityOwner,
        PmFixedPlaceLoopbackRole,
        PmExactOwnedCancelLoopbackRole,
    ) {
        (
            self.common,
            self.public_connectivity,
            self.private_connectivity,
            self.place_transport,
            self.cancel_transport,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(super) enum PmAuthenticatedLoopbackCompositionError {
    #[error("authenticated loopback public roles do not match the exact product wire scope")]
    PublicScopeMismatch,
}
