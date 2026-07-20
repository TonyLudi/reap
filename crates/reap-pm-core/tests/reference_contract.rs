use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use reap_core::Venue;
use reap_pm_core::{
    ConnectionEpoch, EnvelopeError, EventClock, EventEnvelope, EventOrdering, IngressSequence,
    MAX_OKX_REFERENCE_DECIMAL_SCALE, OkxReferenceEvent, OkxReferenceEventError, OkxReferenceHandle,
    OkxReferencePrice, OkxReferencePriceError, PmAccountHandle, PmConnectionId, PmProductSource,
    PmSourceHandle, SnapshotRevision, U256,
};

fn reference() -> OkxReferenceHandle {
    OkxReferenceHandle::from_ordinal(2)
}

fn source() -> PmProductSource {
    PmProductSource::okx_reference(PmSourceHandle::from_ordinal(1), reference())
}

fn hash(value: OkxReferencePrice) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn reference_values_are_fixed_width_and_need_no_drop() {
    assert!(!std::mem::needs_drop::<OkxReferencePrice>());
    assert!(!std::mem::needs_drop::<OkxReferenceEvent>());
    assert!(std::mem::size_of::<OkxReferencePrice>() <= 40);
    assert!(std::mem::size_of::<OkxReferenceEvent>() <= 64);
}

#[test]
fn canonical_decimal_equivalence_has_one_exact_identity() {
    let integer = OkxReferencePrice::parse_decimal("50000").unwrap();
    let decimal = OkxReferencePrice::parse_decimal("50000.0").unwrap();
    let padded = OkxReferencePrice::parse_decimal("00050000.000000").unwrap();

    assert_eq!(integer, decimal);
    assert_eq!(decimal, padded);
    assert_eq!(hash(integer), hash(padded));
    assert_eq!(integer.coefficient(), U256::from_u64(50_000));
    assert_eq!(integer.decimal_scale(), 0);
    assert_eq!(integer.to_string(), "50000");
    assert_eq!(serde_json::to_string(&integer).unwrap(), "\"50000\"");
    assert_eq!(
        serde_json::from_str::<OkxReferencePrice>("\"50000.0\"").unwrap(),
        integer
    );
}

#[test]
fn fractional_reference_prices_remain_exact_and_totally_ordered() {
    let minimum_scale = OkxReferencePrice::parse_decimal("0.000000000000000001").unwrap();
    assert_eq!(minimum_scale.coefficient(), U256::ONE);
    assert_eq!(
        minimum_scale.decimal_scale(),
        MAX_OKX_REFERENCE_DECIMAL_SCALE
    );
    assert_eq!(minimum_scale.to_string(), "0.000000000000000001");

    let fractional = OkxReferencePrice::parse_decimal("50000.0012300").unwrap();
    assert_eq!(fractional.coefficient(), U256::from_u64(5_000_000_123));
    assert_eq!(fractional.decimal_scale(), 5);
    assert_eq!(fractional.to_string(), "50000.00123");
    assert_eq!(
        serde_json::from_str::<OkxReferencePrice>(&serde_json::to_string(&fractional).unwrap())
            .unwrap(),
        fractional
    );

    assert!(minimum_scale < OkxReferencePrice::parse_decimal("0.01").unwrap());
    assert!(
        OkxReferencePrice::parse_decimal("49999.999999").unwrap()
            < OkxReferencePrice::parse_decimal("50000").unwrap()
    );
    assert!(
        OkxReferencePrice::parse_decimal("50000.000001").unwrap()
            > OkxReferencePrice::parse_decimal("50000").unwrap()
    );
}

#[test]
fn invalid_zero_signed_exponent_and_non_decimal_inputs_are_rejected() {
    for input in [
        "", "0", "0.0", "-1", "+1", "1e3", "1E3", ".1", "1.", "1.2.3", "1_000", "NaN", "inf",
    ] {
        assert!(OkxReferencePrice::parse_decimal(input).is_err(), "{input}");
    }

    assert_eq!(
        OkxReferencePrice::parse_decimal("0"),
        Err(OkxReferencePriceError::Zero)
    );
    assert_eq!(
        OkxReferencePrice::new(U256::ONE, MAX_OKX_REFERENCE_DECIMAL_SCALE + 1),
        Err(OkxReferencePriceError::ScaleTooLarge)
    );
    assert_eq!(
        serde_json::from_str::<OkxReferencePrice>("50000")
            .unwrap_err()
            .classify(),
        serde_json::error::Category::Data
    );
}

#[test]
fn scale_and_u256_overflow_fail_without_rounding() {
    let maximum = OkxReferencePrice::parse_decimal(
        "115792089237316195423570985008687907853269984665640564039457584007913129639935",
    )
    .unwrap();
    assert_eq!(maximum.coefficient(), U256::MAX);
    assert_eq!(
        maximum.to_string(),
        "115792089237316195423570985008687907853269984665640564039457584007913129639935"
    );
    assert_eq!(
        OkxReferencePrice::parse_decimal("0.0000000000000000001"),
        Err(OkxReferencePriceError::ScaleTooLarge)
    );
    assert_eq!(
        OkxReferencePrice::parse_decimal(
            "115792089237316195423570985008687907853269984665640564039457584007913129639936"
        ),
        Err(OkxReferencePriceError::Overflow)
    );
    assert_eq!(
        OkxReferencePrice::parse_decimal(&"9".repeat(129)),
        Err(OkxReferencePriceError::InputTooLong)
    );
}

#[test]
fn checked_constructor_normalizes_trailing_coefficient_zeroes() {
    let price = OkxReferencePrice::new(U256::from_u64(5_000_000), 2).unwrap();
    assert_eq!(price.coefficient(), U256::from_u64(50_000));
    assert_eq!(price.decimal_scale(), 0);
    assert_eq!(price.to_string(), "50000");
    assert_eq!(
        OkxReferencePrice::new(U256::from_u64(10), 19).unwrap(),
        OkxReferencePrice::parse_decimal("0.000000000000000001").unwrap()
    );
    assert_eq!(
        OkxReferencePrice::new(U256::ZERO, 0),
        Err(OkxReferencePriceError::Zero)
    );
}

#[test]
fn reference_event_rejects_wrong_venue_and_reference_handle() {
    let price = OkxReferencePrice::parse_decimal("50000.125").unwrap();
    let event = OkxReferenceEvent::new(source(), reference(), price).unwrap();
    assert_eq!(event.source(), source());
    assert_eq!(event.source().venue(), Venue::Okx);
    assert_eq!(event.reference(), reference());
    assert_eq!(event.price(), price);

    assert_eq!(
        OkxReferenceEvent::new(
            PmProductSource::polymarket_account(
                PmSourceHandle::from_ordinal(1),
                PmAccountHandle::from_ordinal(2),
            ),
            reference(),
            price,
        ),
        Err(OkxReferenceEventError::WrongSource)
    );
    assert_eq!(
        OkxReferenceEvent::new(source(), OkxReferenceHandle::from_ordinal(99), price,),
        Err(OkxReferenceEventError::ReferenceHandleMismatch)
    );
}

#[test]
fn exact_reference_event_constructs_a_source_bound_envelope() {
    let event = OkxReferenceEvent::new(
        source(),
        reference(),
        OkxReferencePrice::parse_decimal("50000.125").unwrap(),
    )
    .unwrap();
    let envelope = EventEnvelope::new(
        Venue::Okx,
        source(),
        PmConnectionId::new("okx-reference-1").unwrap(),
        EventClock::new(Some(100), 200, 300, 310).unwrap(),
        EventOrdering::new(
            ConnectionEpoch::new(1),
            Some(SnapshotRevision::new(1)),
            None,
            None,
            IngressSequence::new(1),
        )
        .unwrap(),
        event,
    )
    .unwrap();
    assert_eq!(envelope.payload(), &event);

    let other_source = PmProductSource::okx_reference(
        PmSourceHandle::from_ordinal(1),
        OkxReferenceHandle::from_ordinal(3),
    );
    assert_eq!(
        EventEnvelope::new(
            Venue::Okx,
            other_source,
            PmConnectionId::new("okx-reference-1").unwrap(),
            EventClock::new(Some(100), 200, 300, 310).unwrap(),
            EventOrdering::new(
                ConnectionEpoch::new(1),
                None,
                None,
                None,
                IngressSequence::new(1),
            )
            .unwrap(),
            event,
        ),
        Err(EnvelopeError::PayloadSourceMismatch {
            envelope_source: other_source,
            payload_source: source(),
        })
    );
}
