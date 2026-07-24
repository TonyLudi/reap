use std::time::Duration;

#[cfg(test)]
use std::path::PathBuf;

use reap_okx_public_source::OkxPublicSession;
use reap_pm_core::{
    ConnectionEpoch, EventOrdering, EvmAddress, IngressSequence, MAX_OKX_REFERENCES_PER_MAPPING,
    MAX_REQUIRED_SPENDERS, OkxInstrumentId, OkxReferenceHandle, OkxReferenceInstrument,
    PmAccountHandle, PmAccountScope, PmAllowanceValue, PmAssetId, PmChainId, PmConditionId,
    PmConnectionId, PmEnvironmentId, PmErc1155OperatorApproval, PmFunderId, PmInstrumentHandle,
    PmMarketHandle, PmMarketId, PmMarketLifecycle, PmMarketMetadata, PmOutcomeLabel,
    PmOutcomeMetadata, PmProductSource, PmQuantity, PmReferenceMapping, PmSignerId,
    PmSnapshotEvidence, PmSourceHandle, PmSpenderDomain, PmSpenderRequirement, PmTick,
    PmTokenHandle, PmTokenId, ReceivedEventClock, SnapshotRevision, U256,
};
#[cfg(test)]
use reap_pm_core::{PmFillQueryCursor, PmPositionAvailability, PmSignedUnits};
#[cfg(test)]
use reap_pm_live_contracts::PmFakeExecutionProfile;
use reap_pm_live_contracts::{
    PmAccountConnectivityConfig, PmConnectionRoute, PmConnectivityConfig,
    PmPublicConnectivityConfig,
};
use reap_pm_state::{
    PmBookFreshness, PmCardinalityRiskLimits, PmExposureRiskLimits, PmFreshnessRiskLimits,
    PmOrderRiskLimits, PmRiskLimits,
};
use reap_pm_strategy::{
    PmModelInputRequirements, PmQuoteModel, PmQuoteModelInput, PmQuoteModelOutput,
    PmQuoteModelRequirements, PmQuoteSides,
};
use reap_polymarket_adapter::{
    PmAuthoritativeMetadata, PmFixtureAllowanceRow, PmFixtureCompletionOccurrence,
    PmMetadataRevisionInput, PmPublicHeartbeatConfig, PmPublicRole, PmPublicSession,
};
#[cfg(test)]
use reap_polymarket_adapter::{
    PmFixtureBalanceRow, PmFixtureFeeEvidence, PmFixtureInstrumentScope, PmFixturePositionRow,
};

#[cfg(test)]
use crate::{
    PmCaptureProvenance, PmOpenOrdersFixtureInput, PmProduct, PmProductPublicIngressOutcome,
    PmProductRun, PmProductStartError, PmReconciliationFixtureInput,
};
use crate::{
    PmCaptureReconnectPolicy, PmCaptureSessionPolicy, PmCoordinatorPolicy, PmFixtureQueryOccurrence,
};

pub(crate) const CONDITION: &str =
    "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub(crate) const MARKET: &str =
    "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
pub(crate) const TOKEN: u64 = 123;
pub(crate) const PM_FUNDER: &str = "0xabababababababababababababababababababab";
const PUSD: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
const CONDITIONAL_TOKENS: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
const STANDARD_EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";

#[derive(Debug, Clone, Copy)]
pub(crate) struct Phase6Model {
    requirements: PmModelInputRequirements,
    quantity: PmQuantity,
    alternate_replacement_price: bool,
}

impl PmQuoteModelRequirements for Phase6Model {
    fn input_requirements(&self) -> PmModelInputRequirements {
        self.requirements
    }
}

impl PmQuoteModel for Phase6Model {
    fn evaluate(&self, input: PmQuoteModelInput) -> PmQuoteModelOutput {
        let replacement_band = (input.monotonic_observed_ns() / 10) % 2;
        let probability = if self.alternate_replacement_price && replacement_band == 1 {
            0.41
        } else {
            0.40
        };
        PmQuoteModelOutput::new(probability, self.quantity, PmQuoteSides::Buy)
            .expect("fixed model output")
    }
}

pub(crate) fn instrument() -> PmInstrumentHandle {
    PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(0),
        PmTokenHandle::from_ordinal(0),
    )
}

pub(crate) fn reference() -> OkxReferenceHandle {
    OkxReferenceHandle::from_ordinal(0)
}

pub(crate) fn token() -> PmTokenId {
    PmTokenId::new(U256::from_u64(TOKEN)).expect("fixed token")
}

pub(crate) fn account_scope() -> PmAccountScope {
    let eoa = EvmAddress::parse(PM_FUNDER).expect("fixed funder");
    PmAccountScope::new(
        PmEnvironmentId::new("phase6-evidence").expect("fixed environment"),
        PmChainId::new(137).expect("fixed chain"),
        PmSignerId::new(eoa),
        PmFunderId::new(eoa),
        PmAccountHandle::from_ordinal(7),
    )
}

pub(crate) fn market_metadata() -> PmMarketMetadata {
    let chain = PmChainId::new(137).expect("fixed chain");
    let exchange = EvmAddress::parse(STANDARD_EXCHANGE).expect("fixed exchange");
    let mut spenders = [None; MAX_REQUIRED_SPENDERS];
    spenders[0] = Some(PmSpenderRequirement::new(
        chain,
        exchange,
        PmSpenderDomain::Standard,
        PmAssetId::collateral(EvmAddress::parse(PUSD).expect("fixed collateral")),
    ));
    spenders[1] = Some(PmSpenderRequirement::new(
        chain,
        exchange,
        PmSpenderDomain::Standard,
        PmAssetId::outcome(
            EvmAddress::parse(CONDITIONAL_TOKENS).expect("fixed conditional token"),
            token(),
        ),
    ));
    PmMarketMetadata::new(
        PmConditionId::parse(CONDITION).expect("fixed condition"),
        PmMarketId::parse(MARKET).expect("fixed market"),
        PmOutcomeMetadata::new(
            token(),
            PmOutcomeLabel::new("Yes").expect("fixed outcome label"),
        ),
        PmMarketLifecycle::new(true, false, false, true, true),
        PmTick::parse_decimal("0.01").expect("fixed tick"),
        PmQuantity::parse_decimal("5").expect("fixed minimum quantity"),
        false,
        chain,
        exchange,
        spenders,
        2,
    )
    .expect("valid fixed market metadata")
}

pub(crate) fn connectivity_config() -> PmConnectivityConfig {
    let instrument = instrument();
    let reference = reference();
    let mut references = [None; MAX_OKX_REFERENCES_PER_MAPPING];
    references[0] = Some(reference);
    let mapping =
        PmReferenceMapping::new(instrument, references, 1).expect("one fixed reference mapping");
    let public = PmPublicConnectivityConfig::new(
        mapping,
        OkxReferenceInstrument::index(
            OkxInstrumentId::new("BTC-USDT").expect("fixed reference instrument"),
        ),
        market_metadata(),
        PmConnectionRoute::new(
            PmProductSource::okx_reference(PmSourceHandle::from_ordinal(0), reference),
            PmConnectionId::new("phase6-okx").expect("fixed OKX connection"),
        ),
        PmConnectionRoute::new(
            PmProductSource::polymarket_market(PmSourceHandle::from_ordinal(1), instrument.token()),
            PmConnectionId::new("phase6-pm-public").expect("fixed PM public connection"),
        ),
    )
    .expect("valid fixed public config");
    let scope = account_scope();
    let account = PmAccountConnectivityConfig::derive_goal_f(
        &public,
        scope,
        PmConnectionRoute::new(
            PmProductSource::polymarket_account(PmSourceHandle::from_ordinal(2), scope.handle()),
            PmConnectionId::new("phase6-pm-account").expect("fixed account connection"),
        ),
    )
    .expect("valid fixed account config");
    PmConnectivityConfig::new(public, account).expect("valid fixed product config")
}

pub(crate) fn risk_limits() -> PmRiskLimits {
    PmRiskLimits::new(
        PmOrderRiskLimits::new(
            PmQuantity::parse_decimal("100").expect("fixed order limit"),
            U256::from_u64(100_000_000),
        )
        .expect("valid order limits"),
        PmExposureRiskLimits::new(
            U256::from_u64(20_000_000_000),
            U256::from_u64(10_000_000_000),
            U256::from_u64(10_000_000_000),
            U256::from_u64(10_000_000_000),
        )
        .expect("valid exposure limits"),
        PmCardinalityRiskLimits::new(8_192, 8_192, 8_192).expect("valid cardinality limits"),
        PmFreshnessRiskLimits::new(
            1_000_000_000,
            1_000_000_000,
            1_000_000_000,
            1_000_000_000,
            1_000_000_000,
            1_000_000_000,
        )
        .expect("valid freshness limits"),
    )
}

pub(crate) fn model() -> Phase6Model {
    Phase6Model {
        requirements: PmModelInputRequirements::new(reference(), instrument()),
        quantity: PmQuantity::parse_decimal("5").expect("fixed quantity"),
        alternate_replacement_price: true,
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReachedOverloadProfile {
    Standard,
    Storage1024,
    Persistence513,
}

#[cfg(test)]
impl ReachedOverloadProfile {
    fn model(self) -> Phase6Model {
        let quantity = match self {
            Self::Standard => "5",
            Self::Storage1024 => "10.24",
            Self::Persistence513 => "5.13",
        };
        Phase6Model {
            requirements: PmModelInputRequirements::new(reference(), instrument()),
            quantity: PmQuantity::parse_decimal(quantity).expect("fixed overload quantity"),
            alternate_replacement_price: false,
        }
    }
}

pub(crate) fn coordinator_policy() -> PmCoordinatorPolicy {
    PmCoordinatorPolicy::new(1_000_000_000, 1_000_000_000, 1_000_000)
        .expect("valid fixed coordinator policy")
}

pub(crate) fn authoritative() -> PmAuthoritativeMetadata {
    let lifecycle = format!(
        r#"{{"condition_id":"{CONDITION}","market_id":"{MARKET}","active":true,"closed":false,"archived":false,"accepting_orders":true,"enable_order_book":true}}"#
    );
    let clob = format!(
        r#"{{"condition_id":"{CONDITION}","market_id":"{MARKET}","minimum_tick_size":"0.01","minimum_order_size":"5","neg_risk":false,"tokens":[{{"token_id":"{TOKEN}","outcome":"Yes"}},{{"token_id":"456","outcome":"No"}}]}}"#
    );
    PmAuthoritativeMetadata::join_raw(
        instrument(),
        PmProductSource::polymarket_market(PmSourceHandle::from_ordinal(1), instrument().token()),
        market_metadata(),
        lifecycle.as_bytes(),
        clob.as_bytes(),
        PmMetadataRevisionInput::new(reap_pm_core::SnapshotRevision::new(1), 50)
            .expect("fixed metadata revision"),
    )
    .expect("valid fixed authority")
}

pub(crate) fn session_policy() -> PmCaptureSessionPolicy {
    let reconnect =
        PmCaptureReconnectPolicy::new(Duration::from_nanos(10), Duration::from_nanos(40), 2)
            .expect("fixed reconnect policy");
    PmCaptureSessionPolicy::new(
        reap_pm_core::ConnectionEpoch::new(1),
        None,
        reconnect,
        PmPublicHeartbeatConfig::new(10, 5).expect("fixed heartbeat"),
        PmBookFreshness::new(1_000_000_000, 1_000_000_000).expect("fixed book freshness"),
        1,
        reconnect,
    )
    .expect("valid fixed session policy")
}

#[cfg(test)]
pub(crate) fn provenance() -> PmCaptureProvenance {
    PmCaptureProvenance::new(
        "8222273a9c72033b760e1d2fec813bc77144556d",
        "bbb5bc143a914ba8c96d84342321b3dba30ec0fc",
        "8e671f14c4b1e8137b1dc1b0bd7d39c79d9c8f961a8483daa32151df99cbdf81",
        "aca0221387a45e0ab0eec76adfb3dce8e7d3c0cbcb32187167dd5c556c459eeb",
    )
    .expect("valid fixed provenance")
}

pub(crate) fn public_sessions() -> (PmPublicSession, OkxPublicSession) {
    let config = connectivity_config();
    let role = PmPublicRole::from_expected_metadata(
        config.public().observation_grant(),
        config.public().expected_metadata(),
        config.public().polymarket_route().source(),
        config.public().polymarket_route().connection(),
    )
    .expect("fixed PM public role");
    let policy = session_policy();
    let pm = PmPublicSession::new(
        role,
        authoritative(),
        policy.pm_initial_epoch(),
        policy.pm_last_snapshot_revision(),
        policy.pm_reconnect().as_transport(),
        policy.pm_heartbeat().expect("fixed heartbeat"),
    )
    .expect("fixed PM public session");
    let okx = OkxPublicSession::new_configured_capture(
        config
            .public()
            .okx_reference_instrument()
            .instrument_id()
            .as_str(),
        config.public().okx_route().connection().as_str(),
        policy.okx_initial_epoch(),
        policy.okx_reconnect().as_transport(),
    )
    .expect("fixed OKX public session");
    (pm, okx)
}

pub(crate) fn snapshot_frame() -> String {
    format!(
        r#"{{"event_type":"book","market":"{MARKET}","asset_id":"{TOKEN}","timestamp":"123456789","hash":"8cbca234acd8c8a70913b01de917fbf6160b73e0","bids":[{{"price":"0.30","size":"100"}}],"asks":[{{"price":"0.60","size":"75"}}],"min_order_size":"5","tick_size":"0.01","neg_risk":false,"last_trade_price":"0.40"}}"#
    )
}

pub(crate) fn top_frame() -> String {
    format!(
        r#"{{"event_type":"best_bid_ask","market":"{MARKET}","asset_id":"{TOKEN}","timestamp":"123456791","best_bid":"0.30","best_ask":"0.60","bid_size":"100","ask_size":"75"}}"#
    )
}

pub(crate) const fn okx_ack_frame() -> &'static str {
    r#"{"event":"subscribe","arg":{"channel":"index-tickers","instId":"BTC-USDT"}}"#
}

pub(crate) const fn okx_reference_frame() -> &'static str {
    r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"00050000.125000","ts":"1700000000123"}]}"#
}

#[cfg(test)]
pub(crate) async fn start_reached_overload_product(
    capture_path: PathBuf,
    journal_path: PathBuf,
) -> Result<PmProductRun<Phase6Model>, PmProductStartError> {
    start_reached_overload_product_for(ReachedOverloadProfile::Standard, capture_path, journal_path)
        .await
}

#[cfg(test)]
pub(crate) async fn start_reached_overload_product_for(
    profile: ReachedOverloadProfile,
    capture_path: PathBuf,
    journal_path: PathBuf,
) -> Result<PmProductRun<Phase6Model>, PmProductStartError> {
    let product = PmProduct::new(
        connectivity_config(),
        profile.model(),
        PmFakeExecutionProfile::goal_f(),
        risk_limits(),
    )
    .expect("fixed evidence product composes");
    product
        .start(
            capture_path,
            journal_path,
            authoritative(),
            session_policy(),
            provenance(),
            coordinator_policy(),
        )
        .await
        .map(|(run, _)| run)
}

#[cfg(test)]
pub(crate) async fn prepare_reached_overload_product(
    run: &mut PmProductRun<Phase6Model>,
) -> Result<(), String> {
    prepare_reached_public(run).await?;
    prepare_reached_private(run).await?;
    Ok(())
}

#[cfg(test)]
pub(crate) async fn complete_reached_overload_reconciliation(
    run: &mut PmProductRun<Phase6Model>,
    ordinal: u64,
    raw_fill_frames: &[&[u8]],
) -> Result<(), String> {
    let config = connectivity_config();
    let account = config.account();
    let domain = account.trading_domain();
    let balances = [
        PmFixtureBalanceRow::new(domain.collateral(), U256::from_u64(10_000_000_000)),
        PmFixtureBalanceRow::new(domain.outcome(), U256::from_u64(10_000_000_000)),
    ];
    let spenders = account.required_spenders();
    let allowances = [
        allowance_row(spenders[0], domain.collateral()),
        allowance_row(spenders[1], domain.collateral()),
    ];
    let instrument_scope =
        PmFixtureInstrumentScope::from_metadata(account.instrument(), account.expected_metadata())
            .map_err(|error| error.to_string())?;
    let positions = [PmFixturePositionRow::new(
        instrument_scope,
        U256::from_u64(10_000_000_000),
        PmPositionAvailability::Tradable,
    )];
    let request = 10_000_u64
        .checked_add(ordinal.saturating_mul(2))
        .ok_or_else(|| "fixed reconciliation request sequence overflowed".to_string())?;
    let revision = 2_u64
        .checked_add(ordinal)
        .ok_or_else(|| "fixed reconciliation revision overflowed".to_string())?;
    let monotonic = 2_000_000_000_u64
        .checked_add(ordinal.saturating_mul(1_000_000))
        .ok_or_else(|| "fixed reconciliation clock overflowed".to_string())?;
    let requested_byte = u8::try_from(ordinal)
        .map_err(|_| "fixed reconciliation ordinal exceeds cursor fixture".to_string())?;
    let resulting_byte = requested_byte
        .checked_add(1)
        .ok_or_else(|| "fixed reconciliation cursor overflowed".to_string())?;
    let reconciliation = PmReconciliationFixtureInput::new(
        query_occurrence(1, request, request + 1, revision, monotonic)?,
        &balances,
        &allowances,
        &positions,
        Some(PmFillQueryCursor::new(
            account.account_scope(),
            [requested_byte; 32],
        )),
        PmFillQueryCursor::new(account.account_scope(), [resulting_byte; 32]),
        raw_fill_frames,
        PmFixtureFeeEvidence::Unknown,
    );
    run.ingest_reconciliation_fixture(reconciliation)
        .map_err(|error| error.to_string())?;
    settle_product(run, monotonic + 2)?;
    drain_real_persistence(run, monotonic + 3).await
}

#[cfg(test)]
pub(crate) fn reconcile_reached_overload_fills_without_watermark_advance(
    run: &mut PmProductRun<Phase6Model>,
    raw_fill_frames: &[&[u8]],
) -> Result<(), String> {
    let config = connectivity_config();
    let account = config.account();
    let domain = account.trading_domain();
    let balances = [
        PmFixtureBalanceRow::new(domain.collateral(), U256::from_u64(10_000_000_000)),
        PmFixtureBalanceRow::new(domain.outcome(), U256::from_u64(10_000_000_000)),
    ];
    let spenders = account.required_spenders();
    let allowances = [
        allowance_row(spenders[0], domain.collateral()),
        allowance_row(spenders[1], domain.collateral()),
    ];
    let instrument_scope =
        PmFixtureInstrumentScope::from_metadata(account.instrument(), account.expected_metadata())
            .map_err(|error| error.to_string())?;
    let positions = [PmFixturePositionRow::new(
        instrument_scope,
        U256::from_u64(10_000_000_000),
        PmPositionAvailability::Tradable,
    )];
    let cursor = PmFillQueryCursor::new(account.account_scope(), [1; 32]);
    let reconciliation = PmReconciliationFixtureInput::new(
        query_occurrence(1, 200, 201, 3, 2_150)?,
        &balances,
        &allowances,
        &positions,
        Some(cursor),
        cursor,
        raw_fill_frames,
        PmFixtureFeeEvidence::Known {
            asset: domain.collateral(),
            delta: PmSignedUnits::ZERO,
        },
    );
    run.ingest_reconciliation_fixture(reconciliation)
        .map_err(|error| error.to_string())?;
    settle_product(run, 2_152)
}

#[cfg(test)]
async fn prepare_reached_public(run: &mut PmProductRun<Phase6Model>) -> Result<(), String> {
    let snapshot = snapshot_frame();
    {
        let mut ingress = run.public_ingress();
        ingress
            .record_pm_connection_started(60)
            .await
            .map_err(|error| error.to_string())?;
        ingress
            .record_okx_connection_started(70)
            .await
            .map_err(|error| error.to_string())?;
        ingress
            .record_pm_subscription_sent(90)
            .await
            .map_err(|error| error.to_string())?;
        ingress
            .record_okx_subscription_sent(100)
            .await
            .map_err(|error| error.to_string())?;
        expect_enqueued(
            ingress
                .issue_and_enqueue_pm_metadata(wall(101))
                .await
                .map_err(|error| format!("{error:?}"))?,
            "metadata",
        )?;
        expect_enqueued(
            ingress
                .capture_okx_public(wall(102), 102, okx_ack_frame().as_bytes())
                .await
                .map_err(|error| format!("{error:?}"))?,
            "OKX subscription acknowledgement",
        )?;
        let mut batch = ingress
            .capture_pm_public(wall(103), 103, snapshot.as_bytes())
            .await
            .map_err(|error| error.to_string())?;
        let flow = batch
            .take_snapshot_flow()
            .ok_or_else(|| "fixed PM snapshot omitted its flow token".to_string())?;
        let delivery = batch
            .into_books()
            .into_iter()
            .next()
            .ok_or_else(|| "fixed PM snapshot omitted its delivery".to_string())?;
        expect_enqueued(
            ingress
                .commit_then_enqueue_pm_snapshot(delivery, flow)
                .await
                .map_err(|error| format!("{error:?}"))?,
            "PM snapshot",
        )?;
        expect_enqueued(
            ingress
                .capture_okx_public(wall(104), 104, okx_reference_frame().as_bytes())
                .await
                .map_err(|error| format!("{error:?}"))?,
            "OKX reference",
        )?;
    }
    settle_product(run, 110)?;
    Ok(())
}

#[cfg(test)]
fn expect_enqueued<T, U>(
    outcome: PmProductPublicIngressOutcome<T, U>,
    operation: &str,
) -> Result<T, String> {
    match outcome {
        PmProductPublicIngressOutcome::Enqueued(value) => Ok(value),
        PmProductPublicIngressOutcome::ResyncRequired(_) => {
            Err(format!("fixed {operation} unexpectedly required resync"))
        }
    }
}

#[cfg(test)]
async fn prepare_reached_private(run: &mut PmProductRun<Phase6Model>) -> Result<(), String> {
    run.connect_private_fixture(completion(1, 1, None, 120))
        .map_err(|error| error.to_string())?;
    settle_product(run, 121)?;

    let empty: [&[u8]; 0] = [];
    let open_orders = PmOpenOrdersFixtureInput::new(query_occurrence(1, 2, 3, 1, 130)?, &empty);
    run.ingest_open_orders_fixture(open_orders)
        .map_err(|error| error.to_string())?;
    settle_product(run, 131)?;

    let config = connectivity_config();
    let account = config.account();
    let domain = account.trading_domain();
    let balances = [
        PmFixtureBalanceRow::new(domain.collateral(), U256::from_u64(10_000_000_000)),
        PmFixtureBalanceRow::new(domain.outcome(), U256::from_u64(10_000_000_000)),
    ];
    let spenders = account.required_spenders();
    let allowances = [
        allowance_row(spenders[0], domain.collateral()),
        allowance_row(spenders[1], domain.collateral()),
    ];
    let instrument_scope =
        PmFixtureInstrumentScope::from_metadata(account.instrument(), account.expected_metadata())
            .map_err(|error| error.to_string())?;
    let positions = [PmFixturePositionRow::new(
        instrument_scope,
        U256::from_u64(10_000_000_000),
        PmPositionAvailability::Tradable,
    )];
    let fills: [&[u8]; 0] = [];
    let reconciliation = PmReconciliationFixtureInput::new(
        query_occurrence(1, 4, 5, 2, 140)?,
        &balances,
        &allowances,
        &positions,
        None,
        PmFillQueryCursor::new(account.account_scope(), [1; 32]),
        &fills,
        PmFixtureFeeEvidence::Unknown,
    );
    run.ingest_reconciliation_fixture(reconciliation)
        .map_err(|error| error.to_string())?;
    settle_product(run, 141)?;
    drain_real_persistence(run, 150).await
}

#[cfg(test)]
async fn drain_real_persistence(
    run: &mut PmProductRun<Phase6Model>,
    mut monotonic_ns: u64,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while run.persistence_metrics().depth() != 0 {
        let occurrence = completion(1, monotonic_ns, None, monotonic_ns);
        if run
            .poll_persistence_fixture(occurrence, monotonic_ns)
            .map_err(|error| error.to_string())?
        {
            settle_product(run, monotonic_ns + 1)?;
            monotonic_ns += 2;
        } else {
            if tokio::time::Instant::now() >= deadline {
                return Err("fixed ready setup timed out waiting for durability".to_string());
            }
            // Writer readiness is host timing, not a new logical observation.
            // Retry the same occurrence so a slow host cannot advance fixture time.
            tokio::task::yield_now().await;
        }
    }
    Ok(())
}

#[cfg(test)]
fn settle_product(run: &mut PmProductRun<Phase6Model>, monotonic_ns: u64) -> Result<(), String> {
    for _ in 0..8 {
        let serviced = run
            .service_turn(monotonic_ns)
            .map_err(|error| error.to_string())?;
        while run.pop_effect().is_some() {}
        if serviced.total() == 0 {
            break;
        }
    }
    Ok(())
}

pub(crate) fn query_occurrence(
    epoch: u64,
    request: u64,
    completion_sequence: u64,
    revision: u64,
    monotonic_ns: u64,
) -> Result<PmFixtureQueryOccurrence, String> {
    PmFixtureQueryOccurrence::new(
        ConnectionEpoch::new(epoch),
        IngressSequence::new(request),
        PmSnapshotEvidence::new(SnapshotRevision::new(revision))
            .map_err(|error| error.to_string())?,
        completion(epoch, completion_sequence, Some(revision), monotonic_ns),
        monotonic_ns + 1,
    )
    .map_err(|error| error.to_string())
}

pub(crate) fn completion(
    epoch: u64,
    ingress: u64,
    revision: Option<u64>,
    monotonic_ns: u64,
) -> PmFixtureCompletionOccurrence {
    PmFixtureCompletionOccurrence::new(
        ReceivedEventClock::new(None, wall(monotonic_ns), monotonic_ns)
            .expect("fixed receive clock"),
        EventOrdering::new(
            ConnectionEpoch::new(epoch),
            revision.map(SnapshotRevision::new),
            None,
            None,
            IngressSequence::new(ingress),
        )
        .expect("fixed event ordering"),
    )
}

pub(crate) fn allowance_row(
    spender: reap_pm_core::PmSpenderId,
    collateral: PmAssetId,
) -> PmFixtureAllowanceRow {
    let value = if spender.requirement().asset() == collateral {
        PmAllowanceValue::Erc20(U256::from_u64(10_000_000_000))
    } else {
        PmAllowanceValue::Erc1155Operator(PmErc1155OperatorApproval::from_bool(true))
    };
    PmFixtureAllowanceRow::new(spender, value)
}

const fn wall(monotonic_ns: u64) -> u64 {
    1_700_000_000_000_000_000_u64.saturating_add(monotonic_ns)
}
