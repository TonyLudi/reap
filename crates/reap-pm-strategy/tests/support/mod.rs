#![allow(dead_code)]

use reap_pm_core::{
    EvmAddress, MAX_REQUIRED_SPENDERS, PmAssetId, PmChainId, PmConditionId, PmInstrumentHandle,
    PmMarketHandle, PmMarketId, PmMarketLifecycle, PmMarketMetadata, PmOutcomeLabel,
    PmOutcomeMetadata, PmQuantity, PmSpenderDomain, PmSpenderRequirement, PmTick, PmTokenHandle,
    PmTokenId, U256,
};

const CONDITION: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MARKET: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const PUSD: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
const CONDITIONAL_TOKENS: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
const STANDARD_EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";

pub fn address(value: &str) -> EvmAddress {
    EvmAddress::parse(value).unwrap()
}

pub fn token() -> PmTokenId {
    PmTokenId::new(U256::from_u64(123)).unwrap()
}

pub fn instrument() -> PmInstrumentHandle {
    PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(1),
        PmTokenHandle::from_ordinal(2),
    )
}

pub fn metadata_with(lifecycle: PmMarketLifecycle, tick: &str, minimum: &str) -> PmMarketMetadata {
    let chain = PmChainId::new(137).unwrap();
    let exchange = address(STANDARD_EXCHANGE);
    let collateral = PmAssetId::collateral(address(PUSD));
    let outcome = PmAssetId::outcome(address(CONDITIONAL_TOKENS), token());
    let mut spenders = [None; MAX_REQUIRED_SPENDERS];
    spenders[0] = Some(PmSpenderRequirement::new(
        chain,
        exchange,
        PmSpenderDomain::Standard,
        collateral,
    ));
    spenders[1] = Some(PmSpenderRequirement::new(
        chain,
        exchange,
        PmSpenderDomain::Standard,
        outcome,
    ));
    PmMarketMetadata::new(
        PmConditionId::parse(CONDITION).unwrap(),
        PmMarketId::parse(MARKET).unwrap(),
        PmOutcomeMetadata::new(token(), PmOutcomeLabel::new("Yes").unwrap()),
        lifecycle,
        PmTick::parse_decimal(tick).unwrap(),
        PmQuantity::parse_decimal(minimum).unwrap(),
        false,
        chain,
        exchange,
        spenders,
        2,
    )
    .unwrap()
}

pub fn metadata() -> PmMarketMetadata {
    metadata_with(
        PmMarketLifecycle::new(true, false, false, true, true),
        "0.01",
        "5",
    )
}
