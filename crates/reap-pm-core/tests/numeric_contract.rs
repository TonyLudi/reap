use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::mem::{needs_drop, size_of};
use std::str::FromStr;

use reap_pm_core::{
    CLOB_V2_LOT_UNITS, PM_PROTOCOL_SCALE, PmBookQuantity, PmErc1155OperatorApproval,
    PmNumericError, PmOrderAmounts, PmOrderSalt, PmOrderSide, PmPrice, PmQuantity, PmSign,
    PmSignedUnits, PmTick, U256, exact_order_amounts,
};

fn hash<T: Hash>(value: T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn exact_values_are_heap_free_fixed_width_values() {
    assert_eq!(size_of::<U256>(), 32);
    assert_eq!(size_of::<PmQuantity>(), 32);
    assert_eq!(size_of::<PmPrice>(), 4);
    assert_eq!(PM_PROTOCOL_SCALE, 1_000_000);
    assert_eq!(CLOB_V2_LOT_UNITS, 10_000);

    assert!(!needs_drop::<U256>());
    assert!(!needs_drop::<PmPrice>());
    assert!(!needs_drop::<PmTick>());
    assert!(!needs_drop::<PmQuantity>());
    assert!(!needs_drop::<PmBookQuantity>());
    assert!(!needs_drop::<PmOrderAmounts>());
    assert!(!needs_drop::<PmOrderSalt>());
    assert!(!needs_drop::<PmSignedUnits>());
    assert!(!needs_drop::<PmErc1155OperatorApproval>());
}

#[test]
fn price_range_decimal_exactness_and_executable_grid_boundaries_are_checked() {
    let minimum_candidate = PmPrice::parse_decimal("0.000001").expect("minimum candidate");
    let maximum_candidate = PmPrice::parse_decimal("0.999999").expect("maximum candidate");
    assert_eq!(minimum_candidate.units(), 1);
    assert_eq!(maximum_candidate.units(), 999_999);
    assert_eq!(minimum_candidate.to_string(), "0.000001");
    assert_eq!(maximum_candidate.to_string(), "0.999999");

    let finest_tick = PmTick::parse_decimal("0.0001").unwrap();
    let minimum_executable = PmPrice::parse_decimal("0.0001")
        .unwrap()
        .validate_tick(finest_tick)
        .unwrap();
    let maximum_executable = PmPrice::parse_decimal("0.9999")
        .unwrap()
        .validate_tick(finest_tick)
        .unwrap();
    assert_eq!(minimum_executable.units(), 100);
    assert_eq!(maximum_executable.units(), 999_900);
    assert_eq!(
        minimum_candidate.validate_tick(finest_tick),
        Err(PmNumericError::PriceOffTick)
    );
    assert_eq!(
        maximum_candidate.validate_tick(finest_tick),
        Err(PmNumericError::PriceOffTick)
    );

    assert_eq!(PmPrice::parse_decimal("0"), Err(PmNumericError::Zero));
    assert_eq!(
        PmPrice::parse_decimal("1"),
        Err(PmNumericError::PriceAtOrAboveOne)
    );
    assert_eq!(
        PmPrice::parse_decimal("1.000001"),
        Err(PmNumericError::PriceAtOrAboveOne)
    );
    assert_eq!(
        PmPrice::parse_decimal("-0.1"),
        Err(PmNumericError::Negative)
    );
    assert_eq!(
        PmPrice::parse_decimal("0.0000001"),
        Err(PmNumericError::Underflow)
    );
    assert_eq!(
        PmPrice::parse_decimal("0.1000001"),
        Err(PmNumericError::NonRepresentable)
    );
}

#[test]
fn numerically_equal_price_text_has_one_identity_and_serde_value() {
    let short = PmPrice::parse_decimal("0.1").expect("short price");
    let padded = PmPrice::parse_decimal("0.10").expect("padded price");
    let exact_extra_zero = PmPrice::parse_decimal("0.1000000").expect("exact trailing zero");

    assert_eq!(short, padded);
    assert_eq!(short, exact_extra_zero);
    assert_eq!(hash(short), hash(padded));
    assert_eq!(short.units(), 100_000);
    assert_eq!(short.to_string(), "0.1");

    assert_eq!(serde_json::to_string(&short).unwrap(), "100000");

    let wire_distinct = PmPrice::parse_decimal("0.1001").unwrap();
    assert_ne!(short, wire_distinct);
    assert_ne!(hash(short), hash(wire_distinct));
    assert_ne!(
        short.units().to_be_bytes(),
        wire_distinct.units().to_be_bytes()
    );
}

#[test]
fn only_the_six_frozen_ticks_are_admitted() {
    let supported = [
        ("0.1", 100_000),
        ("0.01", 10_000),
        ("0.005", 5_000),
        ("0.0025", 2_500),
        ("0.001", 1_000),
        ("0.0001", 100),
    ];

    for (decimal, units) in supported {
        let tick = PmTick::parse_decimal(decimal).expect("supported tick");
        assert_eq!(tick.units(), units);
        assert_eq!(PmTick::from_units(units).expect("supported units"), tick);
        assert_eq!(
            serde_json::from_str::<PmTick>(&units.to_string()).expect("tick serde"),
            tick
        );
    }

    assert_eq!(
        PmTick::parse_decimal("0.00001"),
        Err(PmNumericError::UnsupportedTick)
    );
    assert_eq!(
        PmTick::from_units(250),
        Err(PmNumericError::UnsupportedTick)
    );
    assert!(serde_json::from_str::<PmTick>("250").is_err());
}

#[test]
fn price_grid_validation_is_explicit_and_exact() {
    let tick = PmTick::parse_decimal("0.005").expect("tick");
    assert!(
        PmPrice::parse_decimal("0.4")
            .unwrap()
            .validate_tick(tick)
            .is_ok()
    );
    assert_eq!(
        PmPrice::parse_decimal("0.401").unwrap().validate_tick(tick),
        Err(PmNumericError::PriceOffTick)
    );
}

#[test]
fn quantity_parsing_lot_and_minimum_are_exact() {
    let minimum = PmQuantity::parse_decimal("0.05").expect("minimum");
    let quantity = PmQuantity::parse_decimal("0.10").expect("quantity");
    assert_eq!(quantity.protocol_units(), U256::from_u64(100_000));
    assert_eq!(quantity.to_string(), "0.1");
    assert_eq!(quantity.validate_order(minimum), Ok(quantity));

    let off_lot = PmQuantity::parse_decimal("0.011").expect("exact but off lot");
    assert_eq!(
        off_lot.validate_order(minimum),
        Err(PmNumericError::QuantityOffLot)
    );
    let below_minimum = PmQuantity::parse_decimal("0.04").expect("below minimum");
    assert_eq!(
        below_minimum.validate_order(minimum),
        Err(PmNumericError::QuantityBelowMinimum)
    );

    assert_eq!(PmQuantity::parse_decimal("0"), Err(PmNumericError::Zero));
    assert_eq!(
        PmQuantity::parse_decimal("-0.01"),
        Err(PmNumericError::Negative)
    );
    assert_eq!(
        PmQuantity::parse_decimal("0.0000001"),
        Err(PmNumericError::Underflow)
    );
    assert_eq!(
        PmQuantity::parse_decimal("0.0100001"),
        Err(PmNumericError::NonRepresentable)
    );
}

#[test]
fn quantity_serde_uses_canonical_integral_protocol_units() {
    let quantity = PmQuantity::parse_decimal("0.010").expect("quantity");
    assert_eq!(serde_json::to_string(&quantity).unwrap(), "\"10000\"");
    assert_eq!(
        serde_json::from_str::<PmQuantity>("\"010000\"").unwrap(),
        quantity
    );
    assert!(serde_json::from_str::<PmQuantity>("\"0\"").is_err());
    assert!(serde_json::from_str::<PmQuantity>("10000").is_err());
    assert!(serde_json::from_str::<PmQuantity>("0.01").is_err());
}

#[test]
fn zero_book_size_is_an_explicit_delete_not_an_executable_quantity() {
    assert_eq!(
        PmBookQuantity::parse_decimal("0").unwrap(),
        PmBookQuantity::Delete
    );
    assert_eq!(
        PmBookQuantity::parse_decimal("0.000000").unwrap(),
        PmBookQuantity::Delete
    );
    let executable = PmQuantity::parse_decimal("0.01").unwrap();
    assert_eq!(
        PmBookQuantity::parse_decimal("0.01").unwrap(),
        PmBookQuantity::Quantity(executable)
    );
    assert!(PmQuantity::from_protocol_units(U256::ZERO).is_err());
}

#[test]
fn u256_parsing_arithmetic_ordering_and_bytes_are_exact() {
    const MAX: &str =
        "115792089237316195423570985008687907853269984665640564039457584007913129639935";
    const OVERFLOW: &str =
        "115792089237316195423570985008687907853269984665640564039457584007913129639936";

    assert_eq!(U256::from_str(MAX).unwrap(), U256::MAX);
    assert_eq!(U256::from_str(OVERFLOW), Err(PmNumericError::Overflow));
    assert_eq!(U256::MAX.to_string(), MAX);
    assert_eq!(U256::from_u64(1).to_be_bytes()[31], 1);
    assert!(U256::from_u64(u64::MAX) < U256::from_limbs([0, 1, 0, 0]));
    assert_eq!(
        U256::MAX.checked_add(U256::ONE),
        Err(PmNumericError::Overflow)
    );
    assert_eq!(
        U256::ZERO.checked_sub(U256::ONE),
        Err(PmNumericError::Underflow)
    );
    assert_eq!(U256::MAX.checked_mul_u32(2), Err(PmNumericError::Overflow));

    let bytes = U256::MAX.to_be_bytes();
    assert_eq!(bytes, [0xff; 32]);
    assert_eq!(U256::from_be_bytes(bytes), U256::MAX);
    assert_eq!(
        serde_json::to_string(&U256::MAX).unwrap(),
        format!("\"{MAX}\"")
    );
    assert_eq!(serde_json::from_str::<U256>("\"0001\"").unwrap(), U256::ONE);
}

#[test]
fn quantity_supports_the_full_u256_unit_domain_without_text_rounding() {
    let quantity = PmQuantity::from_protocol_units(U256::MAX).expect("positive max");
    assert_eq!(
        PmQuantity::parse_decimal(&quantity.to_string()).expect("decimal round trip"),
        quantity
    );
    assert_eq!(
        serde_json::from_str::<PmQuantity>(&serde_json::to_string(&quantity).unwrap()).unwrap(),
        quantity
    );
}

#[test]
fn buy_and_sell_amounts_use_exact_side_specific_u256_units() {
    let price = PmPrice::parse_decimal("0.4").unwrap();
    let quantity = PmQuantity::parse_decimal("10").unwrap();
    let collateral = U256::from_u64(4_000_000);
    let shares = U256::from_u64(10_000_000);

    let buy = exact_order_amounts(PmOrderSide::Buy, price, quantity).unwrap();
    assert_eq!(buy.maker(), collateral);
    assert_eq!(buy.taker(), shares);

    let sell = exact_order_amounts(PmOrderSide::Sell, price, quantity).unwrap();
    assert_eq!(sell.maker(), shares);
    assert_eq!(sell.taker(), collateral);

    let mut collateral_bytes = [0_u8; 32];
    collateral_bytes[28..].copy_from_slice(&[0x00, 0x3d, 0x09, 0x00]);
    let mut share_bytes = [0_u8; 32];
    share_bytes[28..].copy_from_slice(&[0x00, 0x98, 0x96, 0x80]);
    assert_eq!(buy.maker().to_be_bytes(), collateral_bytes);
    assert_eq!(buy.taker().to_be_bytes(), share_bytes);
    assert_eq!(buy.maker().to_string().as_bytes(), b"4000000");
    assert_eq!(buy.taker().to_string().as_bytes(), b"10000000");
}

#[test]
fn maker_taker_fractional_cross_products_match_exact_protocol_vectors() {
    for (price, quantity, collateral_units, share_units) in [
        ("0.1", "0.01", 1_000_u64, 10_000_u64),
        ("0.0025", "0.04", 100, 40_000),
        ("0.3333", "1.23", 409_959, 1_230_000),
        ("0.9999", "99.99", 99_980_001, 99_990_000),
    ] {
        let price = PmPrice::parse_decimal(price).unwrap();
        let quantity = PmQuantity::parse_decimal(quantity).unwrap();
        let buy = exact_order_amounts(PmOrderSide::Buy, price, quantity).unwrap();
        let sell = exact_order_amounts(PmOrderSide::Sell, price, quantity).unwrap();
        let collateral = U256::from_u64(collateral_units);
        let shares = U256::from_u64(share_units);
        assert_eq!((buy.maker(), buy.taker()), (collateral, shares));
        assert_eq!((sell.maker(), sell.taker()), (shares, collateral));
    }
}

#[test]
fn maker_taker_conversion_handles_full_width_values_and_rejects_half_units() {
    let half_unit_quantity = PmQuantity::parse_decimal("0.000001").unwrap();
    let half_price = PmPrice::parse_decimal("0.5").unwrap();
    assert_eq!(
        half_unit_quantity.validate_order(PmQuantity::parse_decimal("0.01").unwrap()),
        Err(PmNumericError::QuantityOffLot)
    );
    assert_eq!(
        exact_order_amounts(PmOrderSide::Buy, half_price, half_unit_quantity),
        Err(PmNumericError::NonIntegralOrderAmount)
    );

    let maximum_quantity = PmQuantity::from_protocol_units(U256::MAX).unwrap();
    assert_eq!(
        exact_order_amounts(PmOrderSide::Sell, half_price, maximum_quantity),
        Err(PmNumericError::NonIntegralOrderAmount)
    );

    let near_max_even = U256::MAX.checked_sub(U256::ONE).unwrap();
    let near_max_quantity = PmQuantity::from_protocol_units(near_max_even).unwrap();
    let expected_collateral = near_max_even.checked_div_rem_u32(2).unwrap().0;
    let exact = exact_order_amounts(PmOrderSide::Buy, half_price, near_max_quantity).unwrap();
    assert_eq!(exact.maker(), expected_collateral);
    assert_eq!(exact.taker(), near_max_even);
}

#[test]
fn order_salt_is_exactly_the_json_safe_integer_domain() {
    const MAX_SAFE: u64 = 9_007_199_254_740_991;
    let zero = PmOrderSalt::from_u64(0).unwrap();
    let maximum = PmOrderSalt::from_u64(MAX_SAFE).unwrap();

    assert_eq!(zero.value(), 0);
    assert_eq!(maximum.value(), MAX_SAFE);
    assert_eq!(
        serde_json::to_string(&maximum).unwrap(),
        MAX_SAFE.to_string()
    );
    assert_eq!(
        serde_json::from_str::<PmOrderSalt>(&MAX_SAFE.to_string()).unwrap(),
        maximum
    );
    assert_eq!(
        PmOrderSalt::from_u64(MAX_SAFE + 1),
        Err(PmNumericError::SaltOutsideJsonSafeInteger)
    );
    assert!(serde_json::from_str::<PmOrderSalt>(&(MAX_SAFE + 1).to_string()).is_err());
    assert!(serde_json::from_str::<PmOrderSalt>("\"9007199254740991\"").is_err());
}

#[test]
fn signed_units_and_operator_approval_cannot_be_confused_with_unsigned_amounts() {
    let fee = PmSignedUnits::from_parts(PmSign::Negative, U256::from_u64(25)).unwrap();
    assert_eq!(fee.sign(), PmSign::Negative);
    assert_eq!(fee.magnitude(), U256::from_u64(25));
    assert_eq!(
        PmSignedUnits::from_parts(PmSign::Negative, U256::ZERO),
        Err(PmNumericError::NonCanonicalSignedZero)
    );
    assert_eq!(
        PmSignedUnits::from_parts(PmSign::Positive, U256::ZERO).unwrap(),
        PmSignedUnits::ZERO
    );

    let approved = PmErc1155OperatorApproval::from_bool(true);
    let denied = PmErc1155OperatorApproval::from_bool(false);
    assert!(approved.is_approved());
    assert!(!denied.is_approved());
    assert_eq!(serde_json::to_string(&approved).unwrap(), "true");
    assert_eq!(
        serde_json::from_str::<PmErc1155OperatorApproval>("false").unwrap(),
        denied
    );
    assert!(serde_json::from_str::<PmErc1155OperatorApproval>("1").is_err());
}

#[test]
fn malformed_and_oversized_decimal_inputs_are_rejected() {
    for malformed in ["", ".", "0.", ".1", "1.2.3", "+0.1", "1e-1", "NaN", "inf"] {
        assert!(PmPrice::parse_decimal(malformed).is_err(), "{malformed}");
    }
    let oversized = "1".repeat(257);
    assert_eq!(
        PmQuantity::parse_decimal(&oversized),
        Err(PmNumericError::InputTooLong)
    );
}
