#![allow(dead_code)]

use reap_pm_core::{
    ConnectionEpoch, EventOrdering, EvmAddress, IngressSequence, MAX_REQUIRED_SPENDERS,
    PmAccountHandle, PmAccountScope, PmAssetId, PmChainId, PmConditionId, PmConnectionId,
    PmEnvironmentId, PmFunderId, PmGoalFTradingDomain, PmInstrumentHandle, PmInstrumentId,
    PmMarketHandle, PmMarketId, PmMarketLifecycle, PmMarketMetadata, PmOutcomeLabel,
    PmOutcomeMetadata, PmProductSource, PmQuantity, PmSignerId, PmSnapshotEvidence, PmSourceHandle,
    PmSpenderDomain, PmSpenderRequirement, PmTick, PmTokenHandle, PmTokenId, ReceivedEventClock,
    SnapshotRevision, U256,
};
use reap_polymarket_adapter::{
    PmFixtureAccountPositionSnapshot, PmFixtureAccountRoleGrant, PmFixtureCompletionOccurrence,
    PmFixtureInstrumentScope, PmFixturePrivateLifecycle, PmFixturePrivateRoleGrant,
    PmFixtureReadOwnerGrant, PmFixtureReconciliation, PmFixtureReconciliationRoleGrant,
};

pub const CONDITION: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const MARKET: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
pub const FUNDER: &str = "0xabababababababababababababababababababab";
pub const MIXED_CASE_FUNDER: &str = "0xABABABABABABABABABABABABABABABABABABABAB";
pub const PUSD: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
pub const CONDITIONAL_TOKENS: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
pub const STANDARD_EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";

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

pub fn instrument_id() -> PmInstrumentId {
    PmInstrumentId::new(PmMarketId::parse(MARKET).unwrap(), token())
}

pub fn instrument_scope() -> PmFixtureInstrumentScope {
    PmFixtureInstrumentScope::from_metadata(instrument(), market_metadata()).unwrap()
}

pub fn account_scope() -> PmAccountScope {
    let eoa = address(FUNDER);
    PmAccountScope::new(
        PmEnvironmentId::new("phase4-fixture").unwrap(),
        PmChainId::new(137).unwrap(),
        PmSignerId::new(eoa),
        PmFunderId::new(eoa),
        PmAccountHandle::from_ordinal(7),
    )
}

pub fn account_source() -> PmProductSource {
    PmProductSource::polymarket_account(PmSourceHandle::from_ordinal(4), account_scope().handle())
}

pub fn connection() -> PmConnectionId {
    PmConnectionId::new("phase4-fixture-connection").unwrap()
}

pub fn snapshot(revision: u64) -> PmSnapshotEvidence {
    PmSnapshotEvidence::new(SnapshotRevision::new(revision)).unwrap()
}

pub fn completion(
    epoch: u64,
    sequence: u64,
    snapshot_revision: Option<u64>,
) -> PmFixtureCompletionOccurrence {
    PmFixtureCompletionOccurrence::new(
        ReceivedEventClock::new(None, 10_000 + sequence, 20_000 + sequence).unwrap(),
        EventOrdering::new(
            ConnectionEpoch::new(epoch),
            snapshot_revision.map(SnapshotRevision::new),
            None,
            None,
            IngressSequence::new(sequence),
        )
        .unwrap(),
    )
}

pub fn market_metadata() -> PmMarketMetadata {
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
        PmMarketLifecycle::new(true, false, false, true, true),
        PmTick::parse_decimal("0.01").unwrap(),
        PmQuantity::parse_decimal("5").unwrap(),
        false,
        chain,
        exchange,
        spenders,
        2,
    )
    .unwrap()
}

pub fn trading_domain() -> PmGoalFTradingDomain {
    PmGoalFTradingDomain::from_metadata(market_metadata()).unwrap()
}

pub fn grants() -> (
    PmFixturePrivateRoleGrant,
    PmFixtureReconciliationRoleGrant,
    PmFixtureAccountRoleGrant,
) {
    PmFixtureReadOwnerGrant::allocate().split()
}

pub fn private_with(grant: PmFixturePrivateRoleGrant) -> PmFixturePrivateLifecycle {
    PmFixturePrivateLifecycle::new(
        grant,
        account_scope(),
        instrument_scope(),
        account_source(),
        connection(),
    )
    .unwrap()
}

pub fn reconciliation_with(grant: PmFixtureReconciliationRoleGrant) -> PmFixtureReconciliation {
    PmFixtureReconciliation::new(
        grant,
        account_scope(),
        instrument_scope(),
        account_source(),
        connection(),
    )
    .unwrap()
}

pub fn account_with(grant: PmFixtureAccountRoleGrant) -> PmFixtureAccountPositionSnapshot {
    PmFixtureAccountPositionSnapshot::new(
        grant,
        account_scope(),
        instrument_scope(),
        account_source(),
        connection(),
    )
    .unwrap()
}

pub fn private_role() -> PmFixturePrivateLifecycle {
    let (grant, _, _) = grants();
    private_with(grant)
}

pub fn reconciliation_role() -> PmFixtureReconciliation {
    let (_, grant, _) = grants();
    reconciliation_with(grant)
}

pub fn account_role() -> PmFixtureAccountPositionSnapshot {
    let (_, _, grant) = grants();
    account_with(grant)
}
