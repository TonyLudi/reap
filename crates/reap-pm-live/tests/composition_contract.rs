use reap_pm_core::{
    EvmAddress, MAX_REQUIRED_SPENDERS, OkxInstrumentId, OkxReferenceHandle, OkxReferenceInstrument,
    PmAccountHandle, PmAccountScope, PmAssetId, PmChainId, PmConditionId, PmConnectionId,
    PmEnvironmentId, PmFunderId, PmInstrumentHandle, PmMarketHandle, PmMarketId, PmMarketLifecycle,
    PmMarketMetadata, PmOutcomeLabel, PmOutcomeMetadata, PmProductSource, PmQuantity,
    PmReferenceMapping, PmSignerId, PmSourceHandle, PmSpenderDomain, PmSpenderRequirement, PmTick,
    PmTokenHandle, PmTokenId, U256,
};
use reap_pm_live::{PmProduct, PmPublicCapture, PmReadOnlyMonitor};
use reap_pm_live_contracts::{
    PmAccountConnectivityConfig, PmConnectionRoute, PmConnectivityConfig, PmFakeExecutionProfile,
    PmPublicConnectivityConfig, PmRoleKind,
};
use reap_pm_state::{
    PmCardinalityRiskLimits, PmExposureRiskLimits, PmFreshnessRiskLimits, PmOrderRiskLimits,
    PmRiskLimits,
};
use reap_pm_strategy::{PmModelInputRequirements, PmQuoteModelRequirements};

const GOAL_F_PUSD: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
const GOAL_F_CTF: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
const GOAL_F_STANDARD_EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";

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
    let exchange = EvmAddress::parse(GOAL_F_STANDARD_EXCHANGE).unwrap();
    let token = PmTokenId::new(U256::from_u64(11)).unwrap();
    let collateral_requirement = PmSpenderRequirement::new(
        PmChainId::new(137).unwrap(),
        exchange,
        PmSpenderDomain::Standard,
        PmAssetId::collateral(EvmAddress::parse(GOAL_F_PUSD).unwrap()),
    );
    let outcome_requirement = PmSpenderRequirement::new(
        PmChainId::new(137).unwrap(),
        exchange,
        PmSpenderDomain::Standard,
        PmAssetId::outcome(EvmAddress::parse(GOAL_F_CTF).unwrap(), token),
    );
    let mut references = [None; 16];
    references[0] = Some(reference);
    let mapping = PmReferenceMapping::new(instrument, references, 1).unwrap();
    let mut required_spenders = [None; MAX_REQUIRED_SPENDERS];
    required_spenders[0] = Some(collateral_requirement);
    required_spenders[1] = Some(outcome_requirement);
    let expected_metadata = PmMarketMetadata::new(
        PmConditionId::parse("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .unwrap(),
        PmMarketId::parse("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
            .unwrap(),
        PmOutcomeMetadata::new(token, PmOutcomeLabel::new("Yes").unwrap()),
        PmMarketLifecycle::new(true, false, false, true, true),
        PmTick::parse_decimal("0.01").unwrap(),
        PmQuantity::parse_decimal("1").unwrap(),
        false,
        PmChainId::new(137).unwrap(),
        exchange,
        required_spenders,
        2,
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
    let account = PmAccountConnectivityConfig::derive_goal_f(
        &public,
        account_scope,
        PmConnectionRoute::new(
            PmProductSource::polymarket_account(
                PmSourceHandle::from_ordinal(3),
                account_scope.handle(),
            ),
            PmConnectionId::new("pm-account").unwrap(),
        ),
    )
    .unwrap();
    (
        PmConnectivityConfig::new(public, account).unwrap(),
        PmModelInputRequirements::new(reference, instrument),
    )
}

fn risk_limits() -> PmRiskLimits {
    PmRiskLimits::new(
        PmOrderRiskLimits::new(
            PmQuantity::parse_decimal("100").unwrap(),
            U256::from_u64(100_000_000),
        )
        .unwrap(),
        PmExposureRiskLimits::new(
            U256::from_u64(1_000_000_000),
            U256::from_u64(1_000_000_000),
            U256::from_u64(1_000_000_000),
            U256::from_u64(1_000_000_000),
        )
        .unwrap(),
        PmCardinalityRiskLimits::new(128, 128, 128).unwrap(),
        PmFreshnessRiskLimits::new(
            1_000_000_000,
            1_000_000_000,
            1_000_000_000,
            1_000_000_000,
            1_000_000_000,
            1_000_000_000,
        )
        .unwrap(),
    )
}

#[test]
fn roots_construct_exact_connectivity_roles_and_product_schedule_binding() {
    let (config, requirements) = fixture();
    let public = PmPublicCapture::new(config.public().clone()).unwrap();
    let monitor = PmReadOnlyMonitor::new(config.account().clone(), risk_limits()).unwrap();
    let product = PmProduct::new(
        config,
        FixtureModel(requirements),
        PmFakeExecutionProfile::goal_f(),
        risk_limits(),
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
    assert_eq!(monitor.binding_count(), 10);
    assert_eq!(product.binding_count(), 18);
}
