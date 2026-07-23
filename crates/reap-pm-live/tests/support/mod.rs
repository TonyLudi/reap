#![allow(dead_code)]

use std::time::Duration;

use reap_okx_public_source::OkxPublicSession;
use reap_pm_core::{
    ConnectionEpoch, EvmAddress, MAX_OKX_REFERENCES_PER_MAPPING, MAX_REQUIRED_SPENDERS,
    OkxInstrumentId, OkxReferenceHandle, OkxReferenceInstrument, PmAssetId, PmChainId,
    PmConditionId, PmConnectionId, PmEnvironmentId, PmFunderId, PmInstrumentHandle, PmMarketHandle,
    PmMarketId, PmMarketLifecycle, PmMarketMetadata, PmOutcomeLabel, PmOutcomeMetadata,
    PmProductSource, PmQuantity, PmReferenceMapping, PmSignerId, PmSourceHandle, PmSpenderDomain,
    PmSpenderRequirement, PmTick, PmTokenHandle, PmTokenId, SnapshotRevision, U256,
};
use reap_pm_live::{
    PmCaptureHeader, PmCaptureProvenance, PmCaptureReconnectPolicy, PmCaptureScope,
    PmCaptureSessionPolicy,
};
use reap_pm_live_contracts::{
    PmAccountConnectivityConfig, PmConnectionRoute, PmPublicConnectivityConfig,
};
use reap_pm_state::{
    PmBookFreshness, PmBookReducer, PmBookTransition, PmCardinalityRiskLimits, PmDomainFingerprint,
    PmExposureRiskLimits, PmFreshnessRiskLimits, PmMetadataContract, PmMetadataFingerprint,
    PmMetadataObservation, PmOrderRiskLimits, PmRiskLimits,
};
use reap_polymarket_adapter::{
    PmAuthoritativeMetadata, PmMetadataRevisionInput, PmPublicHeartbeatConfig, PmPublicRole,
    PmPublicSession,
};

pub const CONDITION: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const MARKET: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
pub const TOKEN: u64 = 123;
pub const PM_CONNECTION: &str = "pm-public-0";
pub const OKX_CONNECTION: &str = "okx-reference-0";
pub const PM_ACCOUNT_CONNECTION: &str = "pm-account-0";
pub const PM_FUNDER: &str = "0xabababababababababababababababababababab";

const PUSD: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
const CONDITIONAL_TOKENS: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
const STANDARD_EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";

pub fn instrument() -> PmInstrumentHandle {
    PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(0),
        PmTokenHandle::from_ordinal(0),
    )
}

pub fn token() -> PmTokenId {
    PmTokenId::new(U256::from_u64(TOKEN)).unwrap()
}

pub fn pm_source() -> PmProductSource {
    PmProductSource::polymarket_market(PmSourceHandle::from_ordinal(0), instrument().token())
}

pub fn okx_source() -> PmProductSource {
    PmProductSource::okx_reference(
        PmSourceHandle::from_ordinal(0),
        OkxReferenceHandle::from_ordinal(0),
    )
}

pub fn market_metadata() -> PmMarketMetadata {
    let chain = PmChainId::new(137).unwrap();
    let exchange = address(STANDARD_EXCHANGE);
    let mut spenders = [None; MAX_REQUIRED_SPENDERS];
    spenders[0] = Some(PmSpenderRequirement::new(
        chain,
        exchange,
        PmSpenderDomain::Standard,
        PmAssetId::collateral(address(PUSD)),
    ));
    spenders[1] = Some(PmSpenderRequirement::new(
        chain,
        exchange,
        PmSpenderDomain::Standard,
        PmAssetId::outcome(address(CONDITIONAL_TOKENS), token()),
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

pub fn authoritative() -> PmAuthoritativeMetadata {
    let lifecycle = format!(
        r#"{{"condition_id":"{CONDITION}","market_id":"{MARKET}","active":true,"closed":false,"archived":false,"accepting_orders":true,"enable_order_book":true}}"#
    );
    let clob = format!(
        r#"{{"condition_id":"{CONDITION}","market_id":"{MARKET}","minimum_tick_size":"0.01","minimum_order_size":"5","neg_risk":false,"tokens":[{{"token_id":"{TOKEN}","outcome":"Yes"}},{{"token_id":"456","outcome":"No"}}]}}"#
    );
    PmAuthoritativeMetadata::join_raw(
        instrument(),
        pm_source(),
        market_metadata(),
        lifecycle.as_bytes(),
        clob.as_bytes(),
        PmMetadataRevisionInput::new(SnapshotRevision::new(7), 50).unwrap(),
    )
    .unwrap()
}

pub fn public_config() -> PmPublicConnectivityConfig {
    let mut references = [None; MAX_OKX_REFERENCES_PER_MAPPING];
    references[0] = Some(OkxReferenceHandle::from_ordinal(0));
    let mapping = PmReferenceMapping::new(instrument(), references, 1).unwrap();
    PmPublicConnectivityConfig::new(
        mapping,
        OkxReferenceInstrument::index(OkxInstrumentId::new("BTC-USDT").unwrap()),
        market_metadata(),
        PmConnectionRoute::new(okx_source(), PmConnectionId::new(OKX_CONNECTION).unwrap()),
        PmConnectionRoute::new(pm_source(), PmConnectionId::new(PM_CONNECTION).unwrap()),
    )
    .unwrap()
}

pub fn account_scope() -> reap_pm_core::PmAccountScope {
    let eoa = address(PM_FUNDER);
    reap_pm_core::PmAccountScope::new(
        PmEnvironmentId::new("phase4-fixture").unwrap(),
        PmChainId::new(137).unwrap(),
        PmSignerId::new(eoa),
        PmFunderId::new(eoa),
        reap_pm_core::PmAccountHandle::from_ordinal(7),
    )
}

pub fn account_source() -> PmProductSource {
    PmProductSource::polymarket_account(PmSourceHandle::from_ordinal(4), account_scope().handle())
}

pub fn account_config() -> PmAccountConnectivityConfig {
    PmAccountConnectivityConfig::derive_goal_f(
        &public_config(),
        account_scope(),
        PmConnectionRoute::new(
            account_source(),
            PmConnectionId::new(PM_ACCOUNT_CONNECTION).unwrap(),
        ),
    )
    .unwrap()
}

pub fn private_risk_limits() -> PmRiskLimits {
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

pub fn session_policy() -> PmCaptureSessionPolicy {
    let reconnect =
        PmCaptureReconnectPolicy::new(Duration::from_nanos(10), Duration::from_nanos(40), 2)
            .unwrap();
    PmCaptureSessionPolicy::new(
        reap_pm_core::ConnectionEpoch::new(11),
        None,
        reconnect,
        PmPublicHeartbeatConfig::new(10, 5).unwrap(),
        PmBookFreshness::new(10_000, 1_000).unwrap(),
        21,
        reconnect,
    )
    .unwrap()
}

pub fn provenance() -> PmCaptureProvenance {
    PmCaptureProvenance::new(
        "8222273a9c72033b760e1d2fec813bc77144556d",
        "bbb5bc143a914ba8c96d84342321b3dba30ec0fc",
        "8e671f14c4b1e8137b1dc1b0bd7d39c79d9c8f961a8483daa32151df99cbdf81",
        "aca0221387a45e0ab0eec76adfb3dce8e7d3c0cbcb32187167dd5c556c459eeb",
    )
    .unwrap()
}

pub fn capture_header() -> PmCaptureHeader {
    let authority = authoritative();
    let scope = PmCaptureScope::new(&public_config(), &authority).unwrap();
    PmCaptureHeader::new(scope, session_policy(), provenance()).unwrap()
}

pub fn book_reducer(epoch: u64) -> PmBookReducer {
    let authority = authoritative();
    let fingerprint = PmMetadataFingerprint::new(authority.metadata_fingerprint()).unwrap();
    let domain = PmDomainFingerprint::new(authority.domain_fingerprint()).unwrap();
    let contract = PmMetadataContract::goal_f_clob_v2(market_metadata(), domain);
    let mut reducer = PmBookReducer::new(
        instrument(),
        fingerprint,
        contract,
        PmBookFreshness::new(10_000, 1_000).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        reducer
            .apply_metadata(
                PmMetadataObservation::new(
                    instrument(),
                    SnapshotRevision::new(7),
                    fingerprint,
                    contract,
                    50,
                )
                .unwrap(),
            )
            .unwrap(),
        PmBookTransition::MetadataAccepted { .. }
    ));
    reducer.begin_epoch(ConnectionEpoch::new(epoch)).unwrap();
    reducer
}

pub fn capture_session() -> PmPublicSession {
    let config = public_config();
    let role = PmPublicRole::from_expected_metadata(
        config.observation_grant(),
        config.expected_metadata(),
        config.polymarket_route().source(),
        config.polymarket_route().connection(),
    )
    .unwrap();
    let policy = session_policy();
    PmPublicSession::new(
        role,
        authoritative(),
        policy.pm_initial_epoch(),
        policy.pm_last_snapshot_revision(),
        policy.pm_reconnect().as_transport(),
        policy.pm_heartbeat().unwrap(),
    )
    .unwrap()
}

pub fn okx_session() -> OkxPublicSession {
    let config = public_config();
    let policy = session_policy();
    OkxPublicSession::new_configured_capture(
        config.okx_reference_instrument().instrument_id().as_str(),
        config.okx_route().connection().as_str(),
        policy.okx_initial_epoch(),
        policy.okx_reconnect().as_transport(),
    )
    .unwrap()
}

pub fn max_ignored_trade_frame() -> Vec<u8> {
    let event = r#"{"event_type":"last_trade_price"}"#;
    format!(
        "[{}]",
        std::iter::repeat_n(event, 64).collect::<Vec<_>>().join(",")
    )
    .into_bytes()
}

pub fn snapshot_one() -> String {
    format!(
        r#"{{"event_type":"book","market":"{MARKET}","asset_id":"123","timestamp":"123456789","hash":"6ac95ffad569774202496c914c0753fc43279c4c","bids":[{{"price":"0.30","size":"100"}},{{"price":"0.40","size":"50"}}],"asks":[{{"price":"0.60","size":"75"}},{{"price":"0.70","size":"100"}}],"min_order_size":"5","tick_size":"0.01","neg_risk":false,"last_trade_price":"0.50"}}"#
    )
}

pub fn delta() -> String {
    format!(
        r#"{{"event_type":"price_change","market":"{MARKET}","timestamp":"123456790","price_changes":[{{"asset_id":"123","price":"0.40","size":"0","side":"BUY","hash":"tx-delete","best_bid":"0.30","best_ask":"0.60"}},{{"asset_id":"123","price":"0.50","size":"12.5","side":"BUY","hash":"tx-add","best_bid":"0.50","best_ask":"0.60"}}]}}"#
    )
}

pub fn delta_two() -> String {
    format!(
        r#"{{"event_type":"price_change","market":"{MARKET}","timestamp":"123456792","price_changes":[{{"asset_id":"123","price":"0.70","size":"0","side":"SELL","hash":"tx-delete-ask","best_bid":"0.50","best_ask":"0.60"}},{{"asset_id":"123","price":"0.55","size":"20","side":"SELL","hash":"tx-add-ask","best_bid":"0.50","best_ask":"0.55"}}]}}"#
    )
}

pub fn bbo() -> String {
    format!(
        r#"{{"event_type":"best_bid_ask","market":"{MARKET}","asset_id":"123","timestamp":"123456791","best_bid":"0.50","best_ask":"0.60","bid_size":"12.5","ask_size":"75"}}"#
    )
}

pub fn snapshot_two() -> String {
    format!(
        r#"{{"event_type":"book","market":"{MARKET}","asset_id":"123","timestamp":"123456799","hash":"9a761de00e50161d51408c4555b5e0fb6f29c69d","bids":[{{"price":"0.31","size":"80"}},{{"price":"0.41","size":"40"}}],"asks":[{{"price":"0.61","size":"70"}},{{"price":"0.71","size":"90"}}],"min_order_size":"5","tick_size":"0.01","neg_risk":false,"last_trade_price":"0.51"}}"#
    )
}

pub fn tick_change() -> String {
    format!(
        r#"{{"event_type":"tick_size_change","market":"{MARKET}","asset_id":"123","timestamp":"123456792","old_tick_size":"0.01","new_tick_size":"0.001"}}"#
    )
}

pub fn okx_ack() -> &'static str {
    r#"{"event":"subscribe","arg":{"channel":"index-tickers","instId":"BTC-USDT"}}"#
}

pub fn okx_reference() -> &'static str {
    r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"00050000.125000","ts":"1700000000123"}]}"#
}

fn address(value: &str) -> EvmAddress {
    EvmAddress::parse(value).unwrap()
}
