use reap_pm_core::{
    ConnectionEpoch, EvmAddress, IngressSequence, MAX_REQUIRED_SPENDERS, OkxReferenceHandle,
    OkxReferencePrice, PmAssetId, PmChainId, PmConditionId, PmInstrumentHandle, PmMarketHandle,
    PmMarketId, PmMarketLifecycle, PmMarketMetadata, PmOrderSide, PmOutcomeLabel,
    PmOutcomeMetadata, PmPrice, PmQuantity, PmSpenderDomain, PmSpenderRequirement, PmTick,
    PmTokenHandle, PmTokenId, SnapshotRevision, U256,
};
use reap_pm_strategy::{
    PmModelInputRequirements, PmQuoteModelOutput, PmQuoteModelRequirements, PmQuoteSides,
};

use super::*;

#[test]
fn correlation_storage_is_fixed_but_not_carried_inline_by_the_owner() {
    let ring = CorrelationRing::new();
    assert_eq!(ring.values.len(), MAX_COPIED_EFFECT_CORRELATIONS);
    assert_eq!(
        ring.reserved_capacity_bytes(),
        std::mem::size_of::<Option<CopiedEffectCorrelation>>() * MAX_COPIED_EFFECT_CORRELATIONS
    );
    assert!(
        std::mem::size_of::<CorrelationRing>() <= 32,
        "the fixed correlation store must not make every product-owner move copy its payload"
    );
}

#[derive(Debug)]
struct ThresholdModel {
    requirements: PmModelInputRequirements,
    threshold: OkxReferencePrice,
}

#[test]
fn product_owner_keeps_large_fixed_storage_out_of_inline_moves() {
    let inline_bytes = std::mem::size_of::<PmCoordinator<ThresholdModel>>();
    assert!(
        inline_bytes <= 64 * 1024,
        "PM product coordinator is {inline_bytes} inline bytes"
    );
}

impl PmQuoteModelRequirements for ThresholdModel {
    fn input_requirements(&self) -> PmModelInputRequirements {
        self.requirements
    }
}

impl PmQuoteModel for ThresholdModel {
    fn evaluate(&self, input: PmQuoteModelInput) -> PmQuoteModelOutput {
        let probability = if input.reference() >= self.threshold {
            0.60
        } else {
            0.40
        };
        PmQuoteModelOutput::new(
            probability,
            PmQuantity::parse_decimal("5").expect("quantity"),
            PmQuoteSides::Both,
        )
        .expect("fixed threshold output")
    }
}

#[test]
fn exact_okx_value_drives_exact_passive_pm_candidates() {
    let mut state = decision_state("50000.01", 90, 90);
    let DecisionOutcome::Candidates(high) = state.evaluate(100).expect("high evaluation") else {
        panic!("fresh exact inputs produce candidates");
    };
    let [Some(buy), Some(sell)] = high.candidates else {
        panic!("both configured sides are present");
    };
    assert_eq!(buy.side(), PmOrderSide::Buy);
    assert_eq!(sell.side(), PmOrderSide::Sell);
    assert_eq!(buy.price(), PmPrice::from_units(600_000).unwrap());
    assert_eq!(sell.price(), PmPrice::from_units(600_000).unwrap());
    assert_eq!(buy.price().units(), 600_000);
    assert_eq!(sell.price().units(), 600_000);

    state.reference.as_mut().expect("reference").price =
        OkxReferencePrice::parse_decimal("49999.99").unwrap();
    let DecisionOutcome::Candidates(low) = state.evaluate(101).expect("low evaluation") else {
        panic!("fresh exact inputs produce candidates");
    };
    assert_eq!(
        low.candidates[0].expect("buy").price(),
        PmPrice::parse_decimal("0.40").unwrap()
    );
    assert_eq!(
        low.candidates[1].expect("sell").price(),
        PmPrice::parse_decimal("0.40").unwrap()
    );
}

#[test]
fn pm_book_passivity_and_freshness_fail_closed() {
    let mut constrained = decision_state("50000.01", 90, 90);
    constrained.book.as_mut().expect("book").best_ask =
        Some(PmPrice::parse_decimal("0.60").unwrap());
    let DecisionOutcome::Candidates(batch) =
        constrained.evaluate(100).expect("constrained evaluation")
    else {
        panic!("fresh state reaches quote policy");
    };
    assert!(batch.candidates[0].is_none());
    assert_eq!(
        batch.policy_errors[0],
        Some(PmQuotePolicyError::QuoteWouldTakeLiquidity)
    );
    assert!(batch.candidates[1].is_some());

    let mut stale_reference = decision_state("50000.01", 1, 90);
    assert_eq!(
        stale_reference.evaluate(100).unwrap(),
        DecisionOutcome::Suppressed(PmQuoteSuppression::ReferenceStale)
    );

    let mut stale_book = decision_state("50000.01", 90, 1);
    assert_eq!(
        stale_book.evaluate(100).unwrap(),
        DecisionOutcome::Suppressed(PmQuoteSuppression::BookStale)
    );
}

fn decision_state(
    reference_price: &str,
    reference_observed_ns: u64,
    book_observed_ns: u64,
) -> PmDecisionState<ThresholdModel> {
    let instrument = PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(1),
        PmTokenHandle::from_ordinal(1),
    );
    let reference_handle = OkxReferenceHandle::from_ordinal(1);
    let metadata = metadata();
    PmDecisionState {
        model: ThresholdModel {
            requirements: PmModelInputRequirements::new(reference_handle, instrument),
            threshold: OkxReferencePrice::parse_decimal("50000").unwrap(),
        },
        reference_handle,
        instrument,
        expected_metadata: metadata,
        policy: PmCoordinatorPolicy::new(20, 20, 20).unwrap(),
        reference: Some(ReferenceState {
            occurrence: ExactOccurrence::new(ConnectionEpoch::new(1), IngressSequence::new(1)),
            price: OkxReferencePrice::parse_decimal(reference_price).unwrap(),
            observed_monotonic_ns: reference_observed_ns,
            revision: 1,
        }),
        market: Some(MarketState {
            occurrence: ExactOccurrence::new(ConnectionEpoch::new(1), IngressSequence::new(1)),
            metadata,
            metadata_revision: SnapshotRevision::new(1),
            observed_monotonic_ns: 90,
        }),
        book: Some(BookState {
            occurrence: ExactOccurrence::new(ConnectionEpoch::new(1), IngressSequence::new(1)),
            metadata_revision: SnapshotRevision::new(1),
            snapshot_revision: Some(SnapshotRevision::new(1)),
            readiness_revision: 1,
            best_bid: Some(PmPrice::parse_decimal("0.30").unwrap()),
            best_ask: Some(PmPrice::parse_decimal("0.70").unwrap()),
            observed_monotonic_ns: book_observed_ns,
            ready: true,
        }),
        next_reference_revision: 2,
        next_model_revision: 1,
    }
}

fn metadata() -> PmMarketMetadata {
    let token = PmTokenId::new(U256::from_u64(11)).unwrap();
    let chain = PmChainId::new(137).unwrap();
    let exchange = EvmAddress::from_bytes([9; 20]).unwrap();
    let mut spenders = [None; MAX_REQUIRED_SPENDERS];
    spenders[0] = Some(PmSpenderRequirement::new(
        chain,
        exchange,
        PmSpenderDomain::Standard,
        PmAssetId::collateral(EvmAddress::from_bytes([7; 20]).unwrap()),
    ));
    spenders[1] = Some(PmSpenderRequirement::new(
        chain,
        exchange,
        PmSpenderDomain::Standard,
        PmAssetId::outcome(EvmAddress::from_bytes([8; 20]).unwrap(), token),
    ));
    PmMarketMetadata::new(
        PmConditionId::parse("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .unwrap(),
        PmMarketId::parse("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
            .unwrap(),
        PmOutcomeMetadata::new(token, PmOutcomeLabel::new("Yes").unwrap()),
        PmMarketLifecycle::new(true, false, false, true, true),
        PmTick::parse_decimal("0.01").unwrap(),
        PmQuantity::parse_decimal("1").unwrap(),
        false,
        chain,
        exchange,
        spenders,
        2,
    )
    .unwrap()
}
