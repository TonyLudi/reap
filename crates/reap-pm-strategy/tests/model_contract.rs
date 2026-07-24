mod support;

use reap_pm_core::{OkxReferenceHandle, OkxReferencePrice, PmOrderSide, PmQuantity};
use reap_pm_strategy::{
    PmFixtureQuoteModel, PmModelInputRequirements, PmQuoteModel, PmQuoteModelError,
    PmQuoteModelInput, PmQuoteModelRequirements, PmQuoteSides,
};

#[test]
fn fixture_model_is_static_pure_and_retains_reached_reference_input() {
    let requirements =
        PmModelInputRequirements::new(OkxReferenceHandle::from_ordinal(0), support::instrument());
    let model = PmFixtureQuoteModel::new(
        requirements,
        0.403,
        PmQuantity::parse_decimal("10").unwrap(),
        PmQuoteSides::Both,
    )
    .unwrap();
    let input = PmQuoteModelInput::new(
        OkxReferencePrice::parse_decimal("64123.45").unwrap(),
        7,
        support::instrument(),
        900,
    )
    .unwrap();

    let first = model.evaluate(input);
    let second = model.evaluate(input);
    assert_eq!(first, second);
    assert_eq!(first.fair_probability(), 0.403);
    assert_eq!(
        first.sides().ordered(),
        [Some(PmOrderSide::Buy), Some(PmOrderSide::Sell)]
    );
    assert_eq!(model.input_requirements(), requirements);
    assert_eq!(input.reference_revision(), 7);
}

#[test]
fn fixture_model_rejects_invented_or_non_finite_probabilities() {
    let requirements =
        PmModelInputRequirements::new(OkxReferenceHandle::from_ordinal(0), support::instrument());
    for probability in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.1, 1.0] {
        assert_eq!(
            PmFixtureQuoteModel::new(
                requirements,
                probability,
                PmQuantity::parse_decimal("10").unwrap(),
                PmQuoteSides::Buy,
            ),
            Err(PmQuoteModelError::InvalidFixtureProbability)
        );
    }
}
