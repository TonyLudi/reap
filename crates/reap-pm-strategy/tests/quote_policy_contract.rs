mod support;

use reap_pm_core::{PmMarketLifecycle, PmNumericError, PmOrderSide, PmPrice, PmQuantity, U256};
use reap_pm_strategy::{PmQuotePolicyError, PmQuotePolicyInput, validate_passive_quote_candidate};

fn price(value: &str) -> PmPrice {
    PmPrice::parse_decimal(value).unwrap()
}

fn candidate(
    side: PmOrderSide,
    fair_probability: f64,
    quantity: &str,
    bid: Option<&str>,
    ask: Option<&str>,
) -> Result<reap_pm_strategy::PmValidatedQuoteCandidate, PmQuotePolicyError> {
    validate_passive_quote_candidate(PmQuotePolicyInput::new(
        support::instrument(),
        support::metadata(),
        side,
        fair_probability,
        PmQuantity::parse_decimal(quantity).unwrap(),
        bid.map(price),
        ask.map(price),
    ))
}

#[test]
fn directional_rounding_is_side_aware_and_exact() {
    let buy = candidate(PmOrderSide::Buy, 0.403, "10", Some("0.39"), Some("0.50")).unwrap();
    let sell = candidate(PmOrderSide::Sell, 0.403, "10", Some("0.30"), Some("0.50")).unwrap();

    assert_eq!(buy.price(), price("0.40"));
    assert_eq!(sell.price(), price("0.41"));
    assert_eq!(buy.maker_amount(), U256::from_u64(4_000_000));
    assert_eq!(buy.taker_amount(), U256::from_u64(10_000_000));
    assert_eq!(sell.maker_amount(), U256::from_u64(10_000_000));
    assert_eq!(sell.taker_amount(), U256::from_u64(4_100_000));
}

#[test]
fn values_already_on_the_grid_are_not_shifted() {
    for side in [PmOrderSide::Buy, PmOrderSide::Sell] {
        let quote = candidate(side, 0.4, "10", Some("0.30"), Some("0.50")).unwrap();
        assert_eq!(quote.price(), price("0.40"));
    }
}

#[test]
fn non_finite_and_non_executable_model_values_fail_closed() {
    for fair in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            candidate(PmOrderSide::Buy, fair, "10", Some("0.30"), Some("0.50")),
            Err(PmQuotePolicyError::NonFiniteFairProbability)
        );
    }
    for fair in [-0.1, 0.0, 1.0, 1.1] {
        assert_eq!(
            candidate(PmOrderSide::Buy, fair, "10", Some("0.30"), Some("0.50")),
            Err(PmQuotePolicyError::FairProbabilityOutOfRange)
        );
    }
    assert_eq!(
        candidate(PmOrderSide::Buy, 0.001, "10", Some("0.01"), Some("0.50")),
        Err(PmQuotePolicyError::RoundedPriceOutsideExecutableRange)
    );
    assert_eq!(
        candidate(PmOrderSide::Sell, 0.999, "10", Some("0.30"), Some("0.99")),
        Err(PmQuotePolicyError::RoundedPriceOutsideExecutableRange)
    );
}

#[test]
fn passivity_requires_the_opposite_side_and_a_strict_price() {
    assert_eq!(
        candidate(PmOrderSide::Buy, 0.4, "10", Some("0.30"), None),
        Err(PmQuotePolicyError::MissingBestAsk)
    );
    assert_eq!(
        candidate(PmOrderSide::Sell, 0.4, "10", None, Some("0.50")),
        Err(PmQuotePolicyError::MissingBestBid)
    );
    assert_eq!(
        candidate(PmOrderSide::Buy, 0.4, "10", Some("0.30"), Some("0.40")),
        Err(PmQuotePolicyError::QuoteWouldTakeLiquidity)
    );
    assert_eq!(
        candidate(PmOrderSide::Sell, 0.4, "10", Some("0.40"), Some("0.50")),
        Err(PmQuotePolicyError::QuoteWouldTakeLiquidity)
    );
}

#[test]
fn locked_or_crossed_books_fail_before_side_specific_checks() {
    for (bid, ask) in [("0.40", "0.40"), ("0.41", "0.40")] {
        assert_eq!(
            candidate(PmOrderSide::Buy, 0.3, "10", Some(bid), Some(ask)),
            Err(PmQuotePolicyError::LockedOrCrossedBook)
        );
    }
}

#[test]
fn quantity_minimum_and_lot_are_checked_before_candidate_creation() {
    assert_eq!(
        candidate(PmOrderSide::Buy, 0.4, "4.99", Some("0.30"), Some("0.50")),
        Err(PmQuotePolicyError::Numeric(
            PmNumericError::QuantityBelowMinimum
        ))
    );
    assert_eq!(
        candidate(PmOrderSide::Buy, 0.4, "5.001", Some("0.30"), Some("0.50")),
        Err(PmQuotePolicyError::Numeric(PmNumericError::QuantityOffLot))
    );
}

#[test]
fn market_lifecycle_is_part_of_the_policy_boundary() {
    let input = |lifecycle| {
        PmQuotePolicyInput::new(
            support::instrument(),
            support::metadata_with(lifecycle, "0.01", "5"),
            PmOrderSide::Buy,
            0.4,
            PmQuantity::parse_decimal("10").unwrap(),
            Some(price("0.30")),
            Some(price("0.50")),
        )
    };

    assert_eq!(
        validate_passive_quote_candidate(input(PmMarketLifecycle::new(
            false, false, false, true, true
        ))),
        Err(PmQuotePolicyError::MarketInactive)
    );
    assert_eq!(
        validate_passive_quote_candidate(input(PmMarketLifecycle::new(
            true, true, false, true, true
        ))),
        Err(PmQuotePolicyError::MarketClosed)
    );
    assert_eq!(
        validate_passive_quote_candidate(input(PmMarketLifecycle::new(
            true, false, true, true, true
        ))),
        Err(PmQuotePolicyError::MarketArchived)
    );
    assert_eq!(
        validate_passive_quote_candidate(input(PmMarketLifecycle::new(
            true, false, false, false, true
        ))),
        Err(PmQuotePolicyError::OrdersNotAccepted)
    );
    assert_eq!(
        validate_passive_quote_candidate(input(PmMarketLifecycle::new(
            true, false, false, true, false
        ))),
        Err(PmQuotePolicyError::OrderBookDisabled)
    );
}
