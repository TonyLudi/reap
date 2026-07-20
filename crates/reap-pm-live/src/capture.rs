use reap_pm_core::{OkxReferenceHandle, PmConnectionId, PmProductSource};
use reap_pm_live_contracts::{
    ConstructedRoleBinding, PmConnectionRoute, PmPublicConnectivityConfig,
};
use reap_polymarket_adapter::{PmPublicRole, PmPublicRoleError};

#[derive(Debug)]
struct OkxPublicReferenceRole {
    reference: OkxReferenceHandle,
    source: PmProductSource,
    connection: PmConnectionId,
}

impl OkxPublicReferenceRole {
    const fn new(
        reference: OkxReferenceHandle,
        source: PmProductSource,
        connection: PmConnectionId,
    ) -> Self {
        Self {
            reference,
            source,
            connection,
        }
    }
}

/// Narrow Phase 2 construction bundle for the two public capture roles.
#[derive(Debug)]
pub(crate) struct PmCaptureRoles {
    okx: OkxPublicReferenceRole,
    polymarket: PmPublicRole,
}

impl PmCaptureRoles {
    pub(crate) fn new(config: &PmPublicConnectivityConfig) -> Result<Self, PmPublicRoleError> {
        let okx_route = config.okx_route();
        let polymarket_route = config.polymarket_route();
        Ok(Self {
            okx: OkxPublicReferenceRole::new(
                config.okx_reference(),
                okx_route.source(),
                okx_route.connection(),
            ),
            polymarket: PmPublicRole::new(
                config.instrument(),
                polymarket_route.source(),
                polymarket_route.connection(),
            )?,
        })
    }

    pub(crate) fn bindings(&self) -> Vec<ConstructedRoleBinding> {
        let mut bindings = Vec::with_capacity(5);
        bindings.push(ConstructedRoleBinding::okx_public(
            self.okx.reference,
            PmConnectionRoute::new(self.okx.source, self.okx.connection),
        ));
        bindings.extend(ConstructedRoleBinding::pm_public(
            self.polymarket.instrument(),
            PmConnectionRoute::new(self.polymarket.source(), self.polymarket.connection()),
        ));
        bindings
    }

    pub(crate) const fn reference(&self) -> OkxReferenceHandle {
        self.okx.reference
    }

    pub(crate) const fn instrument(&self) -> reap_pm_core::PmInstrumentHandle {
        self.polymarket.instrument()
    }
}
