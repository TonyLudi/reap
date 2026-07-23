use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use reap_pm_core::{
    EvmAddress, OkxInstrumentId, OkxReferenceHandle, OkxReferenceInstrument, PmAssetId, PmChainId,
    PmConditionId, PmGoalFTradingDomain, PmInstrumentHandle, PmInstrumentId, PmMarketHandle,
    PmMarketId, PmMarketLifecycle, PmMarketMetadata, PmMetadataError, PmOutcomeLabel,
    PmOutcomeMetadata, PmPublicObservationGrant, PmQuantity, PmReferenceMapping, PmSpenderDomain,
    PmSpenderRequirement, PmTick, PmTokenHandle, PmTokenId, U256,
};

const CONDITION: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MARKET: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const COLLATERAL: &str = "0x1111111111111111111111111111111111111111";
const SPENDER: &str = "0x2222222222222222222222222222222222222222";
const OUTCOME_CONTRACT: &str = "0x3333333333333333333333333333333333333333";
const GOAL_F_PUSD: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
const GOAL_F_CTF: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
const GOAL_F_STANDARD_EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";
const GOAL_F_NEGATIVE_RISK_EXCHANGE: &str = "0xe2222d279d744050d28e00520010520000310F59";

fn requirement(domain: PmSpenderDomain) -> PmSpenderRequirement {
    PmSpenderRequirement::new(
        PmChainId::new(137).unwrap(),
        EvmAddress::parse(SPENDER).unwrap(),
        domain,
        PmAssetId::collateral(EvmAddress::parse(COLLATERAL).unwrap()),
    )
}

fn metadata_with_spenders(
    negative_risk: bool,
    spenders: [Option<PmSpenderRequirement>; 8],
    spender_count: u8,
) -> Result<PmMarketMetadata, PmMetadataError> {
    PmMarketMetadata::new(
        PmConditionId::parse(CONDITION).unwrap(),
        PmMarketId::parse(MARKET).unwrap(),
        PmOutcomeMetadata::new(
            PmTokenId::new(U256::from_u64(123)).unwrap(),
            PmOutcomeLabel::new("YES").unwrap(),
        ),
        PmMarketLifecycle::new(true, false, false, true, true),
        PmTick::parse_decimal("0.0001").unwrap(),
        PmQuantity::parse_decimal("0.01").unwrap(),
        negative_risk,
        PmChainId::new(137).unwrap(),
        EvmAddress::parse(SPENDER).unwrap(),
        spenders,
        spender_count,
    )
}

fn metadata(negative_risk: bool) -> PmMarketMetadata {
    let mut spenders = [None; 8];
    spenders[0] = Some(requirement(if negative_risk {
        PmSpenderDomain::NegativeRisk
    } else {
        PmSpenderDomain::Standard
    }));
    metadata_with_spenders(negative_risk, spenders, 1).unwrap()
}

fn goal_f_metadata(negative_risk: bool) -> PmMarketMetadata {
    let chain = PmChainId::new(137).unwrap();
    let exchange = EvmAddress::parse(if negative_risk {
        GOAL_F_NEGATIVE_RISK_EXCHANGE
    } else {
        GOAL_F_STANDARD_EXCHANGE
    })
    .unwrap();
    let domain = if negative_risk {
        PmSpenderDomain::NegativeRisk
    } else {
        PmSpenderDomain::Standard
    };
    let token = PmTokenId::new(U256::from_u64(123)).unwrap();
    let mut spenders = [None; 8];
    spenders[0] = Some(PmSpenderRequirement::new(
        chain,
        exchange,
        domain,
        PmAssetId::collateral(EvmAddress::parse(GOAL_F_PUSD).unwrap()),
    ));
    spenders[1] = Some(PmSpenderRequirement::new(
        chain,
        exchange,
        domain,
        PmAssetId::outcome(EvmAddress::parse(GOAL_F_CTF).unwrap(), token),
    ));
    PmMarketMetadata::new(
        PmConditionId::parse(CONDITION).unwrap(),
        PmMarketId::parse(MARKET).unwrap(),
        PmOutcomeMetadata::new(token, PmOutcomeLabel::new("YES").unwrap()),
        PmMarketLifecycle::new(true, false, false, true, true),
        PmTick::parse_decimal("0.0001").unwrap(),
        PmQuantity::parse_decimal("0.01").unwrap(),
        negative_risk,
        chain,
        exchange,
        spenders,
        2,
    )
    .unwrap()
}

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn metadata_retains_structural_membership_and_exact_market_facts() {
    let metadata = metadata(false);
    assert_eq!(
        metadata.condition(),
        PmConditionId::parse(CONDITION).unwrap()
    );
    assert_eq!(metadata.market(), PmMarketId::parse(MARKET).unwrap());
    assert_eq!(metadata.outcome().token().units(), U256::from_u64(123));
    assert_eq!(metadata.outcome().label().as_str(), "YES");
    assert_eq!(metadata.tick().units(), 100);
    assert_eq!(
        metadata.minimum_order_size(),
        PmQuantity::parse_decimal("0.01").unwrap()
    );
    assert_eq!(metadata.lot_units(), 10_000);
    assert!(!metadata.negative_risk());
    assert_eq!(metadata.required_spenders().collect::<Vec<_>>().len(), 1);
}

#[test]
fn goal_f_trading_domain_is_exact_and_never_selected_from_an_arbitrary_spender() {
    for negative_risk in [false, true] {
        let metadata = goal_f_metadata(negative_risk);
        let contract = PmGoalFTradingDomain::from_metadata(metadata).unwrap();
        assert_eq!(
            contract.instrument(),
            PmInstrumentId::new(metadata.market(), metadata.outcome().token())
        );
        assert_eq!(contract.chain(), metadata.chain());
        assert_eq!(contract.exchange(), metadata.exchange());
        assert_eq!(
            contract.collateral(),
            PmAssetId::collateral(EvmAddress::parse(GOAL_F_PUSD).unwrap())
        );
        assert_eq!(
            contract.outcome(),
            PmAssetId::outcome(
                EvmAddress::parse(GOAL_F_CTF).unwrap(),
                metadata.outcome().token()
            )
        );
        assert_eq!(
            contract.required_spenders(),
            metadata.required_spenders().collect::<Vec<_>>().as_slice()
        );
    }

    assert_eq!(
        PmGoalFTradingDomain::from_metadata(metadata(false)),
        Err(PmMetadataError::UnsupportedGoalFExchange)
    );
}

#[test]
fn metadata_represents_lifecycle_without_turning_it_into_readiness_policy() {
    let state = PmMarketLifecycle::new(false, true, true, false, false);
    assert!(!state.active());
    assert!(state.closed());
    assert!(state.archived());
    assert!(!state.accepting_orders());
    assert!(!state.order_book_enabled());
}

#[test]
fn metadata_rejects_off_lot_minimum_and_inconsistent_spenders() {
    let mut spenders = [None; 8];
    spenders[0] = Some(requirement(PmSpenderDomain::Standard));
    let result = PmMarketMetadata::new(
        PmConditionId::parse(CONDITION).unwrap(),
        PmMarketId::parse(MARKET).unwrap(),
        PmOutcomeMetadata::new(
            PmTokenId::new(U256::from_u64(123)).unwrap(),
            PmOutcomeLabel::new("YES").unwrap(),
        ),
        PmMarketLifecycle::new(true, false, false, true, true),
        PmTick::parse_decimal("0.0001").unwrap(),
        PmQuantity::parse_decimal("0.011").unwrap(),
        false,
        PmChainId::new(137).unwrap(),
        EvmAddress::parse(SPENDER).unwrap(),
        spenders,
        1,
    );
    assert_eq!(result, Err(PmMetadataError::MinimumOffLot));

    let mut duplicates = [None; 8];
    duplicates[0] = Some(requirement(PmSpenderDomain::Standard));
    duplicates[1] = duplicates[0];
    assert_eq!(
        PmMarketMetadata::new(
            PmConditionId::parse(CONDITION).unwrap(),
            PmMarketId::parse(MARKET).unwrap(),
            PmOutcomeMetadata::new(
                PmTokenId::new(U256::from_u64(123)).unwrap(),
                PmOutcomeLabel::new("YES").unwrap(),
            ),
            PmMarketLifecycle::new(true, false, false, true, true),
            PmTick::parse_decimal("0.0001").unwrap(),
            PmQuantity::parse_decimal("0.01").unwrap(),
            false,
            PmChainId::new(137).unwrap(),
            EvmAddress::parse(SPENDER).unwrap(),
            duplicates,
            2,
        ),
        Err(PmMetadataError::DuplicateSpender)
    );

    let mut wrong_domain = [None; 8];
    wrong_domain[0] = Some(requirement(PmSpenderDomain::NegativeRisk));
    assert_eq!(
        PmMarketMetadata::new(
            PmConditionId::parse(CONDITION).unwrap(),
            PmMarketId::parse(MARKET).unwrap(),
            PmOutcomeMetadata::new(
                PmTokenId::new(U256::from_u64(123)).unwrap(),
                PmOutcomeLabel::new("YES").unwrap(),
            ),
            PmMarketLifecycle::new(true, false, false, true, true),
            PmTick::parse_decimal("0.0001").unwrap(),
            PmQuantity::parse_decimal("0.01").unwrap(),
            false,
            PmChainId::new(137).unwrap(),
            EvmAddress::parse(SPENDER).unwrap(),
            wrong_domain,
            1,
        ),
        Err(PmMetadataError::SpenderDomainMismatch)
    );
}

#[test]
fn metadata_deserialization_rechecks_exact_minimum_and_spender_invariants() {
    let valid = metadata(false);
    let encoded = serde_json::to_string(&valid).unwrap();
    assert_eq!(
        serde_json::from_str::<PmMarketMetadata>(&encoded).unwrap(),
        valid
    );

    let mut off_lot = serde_json::to_value(valid).unwrap();
    off_lot["minimum_order_size"] = serde_json::json!("11000");
    assert!(serde_json::from_value::<PmMarketMetadata>(off_lot).is_err());

    let mut wrong_domain = serde_json::to_value(valid).unwrap();
    wrong_domain["required_spenders"][0]["domain"] = serde_json::json!("negative_risk");
    assert!(serde_json::from_value::<PmMarketMetadata>(wrong_domain).is_err());

    let mut zero_chain = serde_json::to_value(valid).unwrap();
    zero_chain["chain"] = serde_json::json!(0);
    assert!(serde_json::from_value::<PmMarketMetadata>(zero_chain).is_err());
}

#[test]
fn spender_order_is_canonical_and_outcome_assets_match_the_metadata_token() {
    let collateral = requirement(PmSpenderDomain::Standard);
    let outcome = PmSpenderRequirement::new(
        PmChainId::new(137).unwrap(),
        EvmAddress::parse(SPENDER).unwrap(),
        PmSpenderDomain::Standard,
        PmAssetId::outcome(
            EvmAddress::parse(OUTCOME_CONTRACT).unwrap(),
            PmTokenId::new(U256::from_u64(123)).unwrap(),
        ),
    );

    let mut forward = [None; 8];
    forward[0] = Some(collateral);
    forward[1] = Some(outcome);
    let mut reversed = [None; 8];
    reversed[0] = Some(outcome);
    reversed[1] = Some(collateral);

    let from_forward = metadata_with_spenders(false, forward, 2).unwrap();
    let from_reversed = metadata_with_spenders(false, reversed, 2).unwrap();
    assert_eq!(from_forward, from_reversed);
    assert_eq!(hash_of(&from_forward), hash_of(&from_reversed));
    assert_eq!(
        serde_json::to_vec(&from_forward).unwrap(),
        serde_json::to_vec(&from_reversed).unwrap()
    );
    assert_eq!(
        from_forward.required_spenders().collect::<Vec<_>>(),
        from_reversed.required_spenders().collect::<Vec<_>>()
    );

    let wrong_outcome = PmSpenderRequirement::new(
        PmChainId::new(137).unwrap(),
        EvmAddress::parse(SPENDER).unwrap(),
        PmSpenderDomain::Standard,
        PmAssetId::outcome(
            EvmAddress::parse(OUTCOME_CONTRACT).unwrap(),
            PmTokenId::new(U256::from_u64(999)).unwrap(),
        ),
    );
    let mut wrong = [None; 8];
    wrong[0] = Some(wrong_outcome);
    assert_eq!(
        metadata_with_spenders(false, wrong, 1),
        Err(PmMetadataError::SpenderOutcomeTokenMismatch)
    );

    let mut wrong_json = serde_json::to_value(from_forward).unwrap();
    wrong_json["required_spenders"][1]["asset"] =
        serde_json::to_value(wrong_outcome.asset()).unwrap();
    assert!(serde_json::from_value::<PmMarketMetadata>(wrong_json).is_err());
}

#[test]
fn reference_mapping_is_nonempty_bounded_sorted_and_duplicate_free() {
    let target = PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(4),
        PmTokenHandle::from_ordinal(9),
    );
    let mut references = [None; 16];
    references[0] = Some(OkxReferenceHandle::from_ordinal(3));
    references[1] = Some(OkxReferenceHandle::from_ordinal(1));
    references[2] = Some(OkxReferenceHandle::from_ordinal(2));
    let mapping = PmReferenceMapping::new(target, references, 3).unwrap();
    assert_eq!(mapping.target(), target);
    assert_eq!(
        mapping.references().collect::<Vec<_>>(),
        vec![
            OkxReferenceHandle::from_ordinal(1),
            OkxReferenceHandle::from_ordinal(2),
            OkxReferenceHandle::from_ordinal(3),
        ]
    );

    assert!(PmReferenceMapping::new(target, [None; 16], 0).is_err());
    assert!(PmReferenceMapping::new(target, [None; 16], 17).is_err());

    let mut duplicates = [None; 16];
    duplicates[0] = Some(OkxReferenceHandle::from_ordinal(1));
    duplicates[1] = Some(OkxReferenceHandle::from_ordinal(1));
    assert!(PmReferenceMapping::new(target, duplicates, 2).is_err());
}

#[test]
fn reference_mapping_deserialization_sorts_and_rejects_ambiguous_arrays() {
    let target = PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(4),
        PmTokenHandle::from_ordinal(9),
    );
    let mut unsorted = serde_json::json!({
        "target": {"market": 4, "token": 9},
        "references": [3, 1, 2, null, null, null, null, null,
                       null, null, null, null, null, null, null, null],
        "reference_count": 3
    });
    let decoded: PmReferenceMapping = serde_json::from_value(unsorted.clone()).unwrap();
    assert_eq!(decoded.target(), target);
    assert_eq!(
        decoded.references().collect::<Vec<_>>(),
        vec![
            OkxReferenceHandle::from_ordinal(1),
            OkxReferenceHandle::from_ordinal(2),
            OkxReferenceHandle::from_ordinal(3),
        ]
    );

    unsorted["references"][1] = serde_json::json!(3);
    assert!(serde_json::from_value::<PmReferenceMapping>(unsorted).is_err());

    let noncanonical_tail = serde_json::json!({
        "target": {"market": 4, "token": 9},
        "references": [1, 2, null, null, null, null, null, null,
                       null, null, null, null, null, null, null, null],
        "reference_count": 1
    });
    assert!(serde_json::from_value::<PmReferenceMapping>(noncanonical_tail).is_err());
}

#[test]
fn goal_f_public_identity_tables_assign_zero_ordinals_and_one_fingerprint() {
    let raw_pm = PmInstrumentId::new(
        PmMarketId::parse(MARKET).unwrap(),
        PmTokenId::new(U256::from_u64(123)).unwrap(),
    );
    let raw_okx = OkxReferenceInstrument::index(OkxInstrumentId::new("BTC-USDT").unwrap());
    let first = PmPublicObservationGrant::derive_goal_f(raw_okx, raw_pm);
    let second = PmPublicObservationGrant::derive_goal_f(raw_okx, raw_pm);

    assert_eq!(first.okx_reference().ordinal(), 0);
    assert_eq!(first.instrument().market().ordinal(), 0);
    assert_eq!(first.instrument().token().ordinal(), 0);
    assert_eq!(first.okx_instrument(), raw_okx);
    assert_eq!(first.polymarket_instrument(), raw_pm);
    assert_eq!(
        first.configuration_fingerprint(),
        second.configuration_fingerprint()
    );
    assert_eq!(
        first.configuration_fingerprint().bytes(),
        [
            0x97, 0xde, 0xc7, 0x0d, 0xd8, 0x4d, 0x86, 0x79, 0x88, 0x68, 0xd3, 0xbf, 0x9c, 0x93,
            0x9c, 0xc1, 0x5a, 0x76, 0xcd, 0x0d, 0x0a, 0x66, 0xe1, 0xc2, 0x02, 0x88, 0x87, 0x7f,
            0xde, 0xb8, 0x27, 0xb0,
        ]
    );

    let changed = PmPublicObservationGrant::derive_goal_f(
        OkxReferenceInstrument::index(OkxInstrumentId::new("ETH-USDT").unwrap()),
        raw_pm,
    );
    assert_ne!(
        first.configuration_fingerprint(),
        changed.configuration_fingerprint()
    );
}
