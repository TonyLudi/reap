use reap_pm_core::{
    EvmAddress, OkxReferenceHandle, PmAccountHandle, PmAccountScope, PmAssetId, PmChainId,
    PmConnectionId, PmEnvironmentId, PmFunderId, PmInstrumentHandle, PmMarketHandle,
    PmProductSource, PmReferenceMapping, PmSignerId, PmSourceHandle, PmSpenderDomain, PmSpenderId,
    PmSpenderRequirement, PmTokenHandle,
};
use reap_pm_live::{PmProduct, PmPublicCapture, PmReadOnlyMonitor};
use reap_pm_live_contracts::{
    PmAccountConnectivityConfig, PmConnectionRoute, PmConnectivityConfig, PmFakeExecutionProfile,
    PmPublicConnectivityConfig, PmRoleKind,
};
use reap_pm_strategy::{PmModelInputRequirements, PmQuoteModelRequirements};

#[derive(Debug)]
struct FixtureModel(PmModelInputRequirements);

impl PmQuoteModelRequirements for FixtureModel {
    fn input_requirements(&self) -> PmModelInputRequirements {
        self.0
    }
}

fn fixture() -> (PmConnectivityConfig, PmModelInputRequirements) {
    let instrument = PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(1),
        PmTokenHandle::from_ordinal(2),
    );
    let reference = OkxReferenceHandle::from_ordinal(3);
    let eoa = EvmAddress::from_bytes([4; 20]).unwrap();
    let account_scope = PmAccountScope::new(
        PmEnvironmentId::new("fixture").unwrap(),
        PmChainId::new(137).unwrap(),
        PmSignerId::new(eoa),
        PmFunderId::new(eoa),
        PmAccountHandle::from_ordinal(4),
    );
    let spender = PmSpenderId::new(
        account_scope.handle(),
        PmSpenderRequirement::new(
            PmChainId::new(137).unwrap(),
            EvmAddress::from_bytes([2; 20]).unwrap(),
            PmSpenderDomain::Standard,
            PmAssetId::collateral(EvmAddress::from_bytes([1; 20]).unwrap()),
        ),
    );
    let mut references = [None; 16];
    references[0] = Some(reference);
    let mapping = PmReferenceMapping::new(instrument, references, 1).unwrap();
    let public = PmPublicConnectivityConfig::new(
        mapping,
        PmConnectionRoute::new(
            PmProductSource::okx_reference(PmSourceHandle::from_ordinal(1), reference),
            PmConnectionId::new("okx-public").unwrap(),
        ),
        PmConnectionRoute::new(
            PmProductSource::polymarket_market(PmSourceHandle::from_ordinal(2), instrument.token()),
            PmConnectionId::new("pm-public").unwrap(),
        ),
    )
    .unwrap();
    let account = PmAccountConnectivityConfig::new(
        instrument,
        account_scope,
        PmConnectionRoute::new(
            PmProductSource::polymarket_account(
                PmSourceHandle::from_ordinal(3),
                account_scope.handle(),
            ),
            PmConnectionId::new("pm-account").unwrap(),
        ),
        vec![spender],
    )
    .unwrap();
    (
        PmConnectivityConfig::new(public, account).unwrap(),
        PmModelInputRequirements::new(reference, instrument),
    )
}

#[test]
fn roots_construct_exact_connectivity_roles_and_product_schedule_binding() {
    let (config, requirements) = fixture();
    let public = PmPublicCapture::new(config.public().clone()).unwrap();
    let monitor = PmReadOnlyMonitor::new(config.account().clone()).unwrap();
    let product = PmProduct::new(
        config,
        FixtureModel(requirements),
        PmFakeExecutionProfile::goal_f(),
    )
    .unwrap();

    assert_eq!(
        public.reached_roles(),
        &[
            PmRoleKind::OkxPublicObservation,
            PmRoleKind::PmPublicObservation
        ]
    );
    assert_eq!(
        monitor.reached_roles(),
        &[
            PmRoleKind::PmPrivateLifecycle,
            PmRoleKind::PmOrderReconciliation,
            PmRoleKind::PmAccountPositionSnapshot,
        ]
    );
    assert_eq!(product.reached_roles().len(), 6);
    assert_eq!(public.binding_count(), 5);
    assert_eq!(monitor.binding_count(), 9);
    assert_eq!(product.binding_count(), 17);
}
