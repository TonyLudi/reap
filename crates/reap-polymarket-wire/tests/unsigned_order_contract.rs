use reap_pm_core::{
    EvmAddress, PmNumericError, PmOrderSalt, PmOrderSide, PmPrice, PmQuantity, PmTick, PmTokenId,
    U256,
};
use reap_polymarket_wire::{
    PM_CLOB_V2_EMPTY_BYTES32, PM_CLOB_V2_EOA_SIGNATURE_TYPE, PmUnsignedClobV2Order,
    PmUnsignedOrderError,
};

const MAKER: &str = "0x1111111111111111111111111111111111111111";
const OTHER: &str = "0x2222222222222222222222222222222222222222";

fn address(value: &str) -> EvmAddress {
    EvmAddress::parse(value).unwrap()
}

fn order(side: PmOrderSide, price: &str, quantity: &str) -> PmUnsignedClobV2Order {
    PmUnsignedClobV2Order::new_goal_f(
        PmOrderSalt::from_u64(123_456_789).unwrap(),
        address(MAKER),
        address(MAKER),
        PmTokenId::new(U256::from_u64(123)).unwrap(),
        side,
        PmPrice::parse_decimal(price).unwrap(),
        PmQuantity::parse_decimal(quantity).unwrap(),
        PmTick::parse_decimal("0.01").unwrap(),
        PmQuantity::parse_decimal("5").unwrap(),
        1_760_000_000_000,
    )
    .unwrap()
}

#[test]
fn buy_unsigned_order_matches_the_frozen_canonical_json() {
    let serialized = serde_json::to_string(&order(PmOrderSide::Buy, "0.40", "10")).unwrap();
    assert_eq!(
        serialized,
        concat!(
            r#"{"builder":"0x0000000000000000000000000000000000000000000000000000000000000000","#,
            r#""maker":"0x1111111111111111111111111111111111111111","#,
            r#""makerAmount":"4000000","#,
            r#""metadata":"0x0000000000000000000000000000000000000000000000000000000000000000","#,
            r#""salt":123456789,"side":"BUY","signatureType":0,"#,
            r#""signer":"0x1111111111111111111111111111111111111111","#,
            r#""takerAmount":"10000000","timestamp":"1760000000000","tokenId":"123"}"#
        )
    );
}

#[test]
fn sell_amounts_reverse_principal_and_shares_without_rounding() {
    let value = serde_json::to_value(order(PmOrderSide::Sell, "0.40", "10")).unwrap();
    assert_eq!(value["side"], "SELL");
    assert_eq!(value["makerAmount"], "10000000");
    assert_eq!(value["takerAmount"], "4000000");
    assert_eq!(value["signatureType"], PM_CLOB_V2_EOA_SIGNATURE_TYPE);
    assert_eq!(value["metadata"], PM_CLOB_V2_EMPTY_BYTES32);
    assert_eq!(value["builder"], PM_CLOB_V2_EMPTY_BYTES32);
    assert!(value.get("expiration").is_none());
    assert!(value.get("owner").is_none());
    assert!(value.get("signature").is_none());
}

#[test]
fn structural_accessors_preserve_the_exact_unsigned_values() {
    let order = order(PmOrderSide::Buy, "0.40", "10");
    assert_eq!(order.salt().value(), 123_456_789);
    assert_eq!(order.maker(), address(MAKER));
    assert_eq!(order.signer(), address(MAKER));
    assert_eq!(order.token_id().units(), U256::from_u64(123));
    assert_eq!(order.maker_amount(), U256::from_u64(4_000_000));
    assert_eq!(order.taker_amount(), U256::from_u64(10_000_000));
    assert_eq!(order.side(), PmOrderSide::Buy);
    assert_eq!(order.timestamp_ms(), 1_760_000_000_000);
}

#[test]
fn lowering_revalidates_eoa_grid_lot_minimum_and_timestamp() {
    let make = |maker, signer, price: &str, quantity: &str, timestamp_ms| {
        PmUnsignedClobV2Order::new_goal_f(
            PmOrderSalt::from_u64(1).unwrap(),
            address(maker),
            address(signer),
            PmTokenId::new(U256::from_u64(123)).unwrap(),
            PmOrderSide::Buy,
            PmPrice::parse_decimal(price).unwrap(),
            PmQuantity::parse_decimal(quantity).unwrap(),
            PmTick::parse_decimal("0.01").unwrap(),
            PmQuantity::parse_decimal("5").unwrap(),
            timestamp_ms,
        )
    };

    assert_eq!(
        make(MAKER, OTHER, "0.40", "10", 1),
        Err(PmUnsignedOrderError::MakerIdentityMismatch)
    );
    assert_eq!(
        make(MAKER, MAKER, "0.40", "10", 0),
        Err(PmUnsignedOrderError::ZeroTimestamp)
    );
    assert_eq!(
        make(MAKER, MAKER, "0.405", "10", 1),
        Err(PmUnsignedOrderError::Numeric(PmNumericError::PriceOffTick))
    );
    assert_eq!(
        make(MAKER, MAKER, "0.40", "5.001", 1),
        Err(PmUnsignedOrderError::Numeric(
            PmNumericError::QuantityOffLot
        ))
    );
    assert_eq!(
        make(MAKER, MAKER, "0.40", "4.99", 1),
        Err(PmUnsignedOrderError::Numeric(
            PmNumericError::QuantityBelowMinimum
        ))
    );
}
