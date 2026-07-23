use reap_pm_core::{
    EvmAddress, MAX_REQUIRED_SPENDERS, OkxInstrumentId, OkxReferenceHandle, OkxReferenceInstrument,
    PmAccountHandle, PmAccountScope, PmAssetId, PmChainId, PmConditionId, PmConnectionId,
    PmEnvironmentId, PmFunderId, PmInstrumentHandle, PmMarketHandle, PmMarketId, PmMarketLifecycle,
    PmMarketMetadata, PmOutcomeLabel, PmOutcomeMetadata, PmProductSource, PmQuantity,
    PmReferenceMapping, PmSignerId, PmSourceHandle, PmSpenderDomain, PmSpenderId,
    PmSpenderRequirement, PmTick, PmTokenHandle, PmTokenId, U256,
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
        PmMarketHandle::from_ordinal(0),
        PmTokenHandle::from_ordinal(0),
    );
    let reference = OkxReferenceHandle::from_ordinal(0);
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
    let mut required_spenders = [None; MAX_REQUIRED_SPENDERS];
    required_spenders[0] = Some(spender.requirement());
    let expected_metadata = PmMarketMetadata::new(
        PmConditionId::parse("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .unwrap(),
        PmMarketId::parse("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
            .unwrap(),
        PmOutcomeMetadata::new(
            PmTokenId::new(U256::from_u64(11)).unwrap(),
            PmOutcomeLabel::new("Yes").unwrap(),
        ),
        PmMarketLifecycle::new(true, false, false, true, true),
        PmTick::parse_decimal("0.01").unwrap(),
        PmQuantity::parse_decimal("1").unwrap(),
        false,
        PmChainId::new(137).unwrap(),
        EvmAddress::from_bytes([2; 20]).unwrap(),
        required_spenders,
        1,
    )
    .unwrap();
    let public = PmPublicConnectivityConfig::new(
        mapping,
        OkxReferenceInstrument::index(OkxInstrumentId::new("BTC-USDT").unwrap()),
        expected_metadata,
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
