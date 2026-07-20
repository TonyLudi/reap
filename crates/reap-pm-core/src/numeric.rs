use std::cmp::Ordering;
use std::error::Error;
use std::fmt;
use std::str::FromStr;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error as ThisError;

pub const PM_PROTOCOL_SCALE: u32 = 1_000_000;
pub const CLOB_V2_LOT_UNITS: u32 = 10_000;
pub const PM_ORDER_SALT_MAX: u64 = (1_u64 << 53) - 1;
pub const MAX_OKX_REFERENCE_DECIMAL_SCALE: u8 = 18;

const MAX_DECIMAL_INPUT_BYTES: usize = 256;
const MAX_OKX_REFERENCE_INPUT_BYTES: usize = 128;
const U256_DECIMAL_DIGITS: usize = 78;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmNumericError {
    Empty,
    InputTooLong,
    Negative,
    InvalidFormat,
    Zero,
    Underflow,
    Overflow,
    NonRepresentable,
    PriceAtOrAboveOne,
    UnsupportedTick,
    PriceOffTick,
    QuantityOffLot,
    QuantityBelowMinimum,
    NonIntegralOrderAmount,
    DivisionByZero,
    SaltOutsideJsonSafeInteger,
    NonCanonicalSignedZero,
}

impl fmt::Display for PmNumericError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "exact PM decimal input is empty",
            Self::InputTooLong => "exact PM decimal input exceeds its size bound",
            Self::Negative => "exact PM value must not be negative",
            Self::InvalidFormat => "exact PM decimal has invalid syntax",
            Self::Zero => "executable PM value must be positive",
            Self::Underflow => "exact PM decimal is below one protocol unit",
            Self::Overflow => "exact PM value exceeds its fixed-width representation",
            Self::NonRepresentable => {
                "exact PM decimal is not representable in integral protocol units"
            }
            Self::PriceAtOrAboveOne => "executable PM price must be strictly less than one",
            Self::UnsupportedTick => "PM tick is outside the frozen supported tick set",
            Self::PriceOffTick => "PM price is not aligned to the configured tick",
            Self::QuantityOffLot => "PM quantity is not aligned to the fixed CLOB V2 lot",
            Self::QuantityBelowMinimum => "PM quantity is below the market minimum",
            Self::NonIntegralOrderAmount => {
                "PM price-times-quantity is not an integral protocol amount"
            }
            Self::DivisionByZero => "cannot divide an exact PM value by zero",
            Self::SaltOutsideJsonSafeInteger => {
                "PM order salt is outside the JSON safe-integer range"
            }
            Self::NonCanonicalSignedZero => "zero PM delta cannot carry a negative sign",
        })
    }
}

impl Error for PmNumericError {}

/// A heap-free unsigned 256-bit integer stored as four little-endian limbs.
///
/// Decimal Serde uses a string because JSON numbers cannot represent the full
/// domain without loss. Binary identity is the exact 32-byte big-endian value.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct U256([u64; 4]);

impl U256 {
    pub const ZERO: Self = Self([0; 4]);
    pub const ONE: Self = Self([1, 0, 0, 0]);
    pub const MAX: Self = Self([u64::MAX; 4]);

    #[must_use]
    pub const fn from_u64(value: u64) -> Self {
        Self([value, 0, 0, 0])
    }

    /// Constructs the integer from little-endian 64-bit limbs.
    #[must_use]
    pub const fn from_limbs(limbs: [u64; 4]) -> Self {
        Self(limbs)
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0[0] == 0 && self.0[1] == 0 && self.0[2] == 0 && self.0[3] == 0
    }

    #[must_use]
    pub fn to_be_bytes(self) -> [u8; 32] {
        let mut bytes = [0_u8; 32];
        bytes[0..8].copy_from_slice(&self.0[3].to_be_bytes());
        bytes[8..16].copy_from_slice(&self.0[2].to_be_bytes());
        bytes[16..24].copy_from_slice(&self.0[1].to_be_bytes());
        bytes[24..32].copy_from_slice(&self.0[0].to_be_bytes());
        bytes
    }

    #[must_use]
    pub fn from_be_bytes(bytes: [u8; 32]) -> Self {
        Self([
            u64::from_be_bytes(bytes[24..32].try_into().expect("fixed slice")),
            u64::from_be_bytes(bytes[16..24].try_into().expect("fixed slice")),
            u64::from_be_bytes(bytes[8..16].try_into().expect("fixed slice")),
            u64::from_be_bytes(bytes[0..8].try_into().expect("fixed slice")),
        ])
    }

    pub fn checked_add(self, other: Self) -> Result<Self, PmNumericError> {
        let mut result = [0_u64; 4];
        let mut carry = false;
        for (index, output) in result.iter_mut().enumerate() {
            let (partial, first_carry) = self.0[index].overflowing_add(other.0[index]);
            let (sum, second_carry) = partial.overflowing_add(u64::from(carry));
            *output = sum;
            carry = first_carry || second_carry;
        }
        if carry {
            Err(PmNumericError::Overflow)
        } else {
            Ok(Self(result))
        }
    }

    pub fn checked_sub(self, other: Self) -> Result<Self, PmNumericError> {
        let mut result = [0_u64; 4];
        let mut borrow = false;
        for (index, output) in result.iter_mut().enumerate() {
            let (partial, first_borrow) = self.0[index].overflowing_sub(other.0[index]);
            let (difference, second_borrow) = partial.overflowing_sub(u64::from(borrow));
            *output = difference;
            borrow = first_borrow || second_borrow;
        }
        if borrow {
            Err(PmNumericError::Underflow)
        } else {
            Ok(Self(result))
        }
    }

    pub fn checked_mul_u32(self, multiplier: u32) -> Result<Self, PmNumericError> {
        let mut result = [0_u64; 4];
        let mut carry = 0_u128;
        for (index, output) in result.iter_mut().enumerate() {
            let product = u128::from(self.0[index]) * u128::from(multiplier) + carry;
            *output = product as u64;
            carry = product >> 64;
        }
        if carry == 0 {
            Ok(Self(result))
        } else {
            Err(PmNumericError::Overflow)
        }
    }

    pub fn checked_div_rem_u32(self, divisor: u32) -> Result<(Self, u32), PmNumericError> {
        if divisor == 0 {
            return Err(PmNumericError::DivisionByZero);
        }
        let divisor = u128::from(divisor);
        let mut quotient = [0_u64; 4];
        let mut remainder = 0_u128;
        for index in (0..4).rev() {
            let dividend = (remainder << 64) | u128::from(self.0[index]);
            quotient[index] = (dividend / divisor) as u64;
            remainder = dividend % divisor;
        }
        Ok((Self(quotient), remainder as u32))
    }

    fn checked_add_u32(self, value: u32) -> Result<Self, PmNumericError> {
        self.checked_add(Self::from_u64(u64::from(value)))
    }
}

impl Ord for U256 {
    fn cmp(&self, other: &Self) -> Ordering {
        for index in (0..4).rev() {
            match self.0[index].cmp(&other.0[index]) {
                Ordering::Equal => {}
                ordering => return ordering,
            }
        }
        Ordering::Equal
    }
}

impl PartialOrd for U256 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl FromStr for U256 {
    type Err = PmNumericError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        parse_integral_u256(input)
    }
}

impl fmt::Display for U256 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            return formatter.write_str("0");
        }

        let mut digits = [0_u8; U256_DECIMAL_DIGITS];
        let mut cursor = digits.len();
        let mut remaining = *self;
        while !remaining.is_zero() {
            let (quotient, remainder) =
                remaining.checked_div_rem_u32(10).map_err(|_| fmt::Error)?;
            cursor -= 1;
            digits[cursor] = b'0' + u8::try_from(remainder).map_err(|_| fmt::Error)?;
            remaining = quotient;
        }
        let decimal = std::str::from_utf8(&digits[cursor..]).map_err(|_| fmt::Error)?;
        formatter.write_str(decimal)
    }
}

impl Serialize for U256 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

struct U256Visitor;

impl Visitor<'_> for U256Visitor {
    type Value = U256;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a quoted unsigned 256-bit decimal integer")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        value.parse().map_err(E::custom)
    }
}

impl<'de> Deserialize<'de> for U256 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(U256Visitor)
    }
}

/// A positive exact decimal OKX reference price.
///
/// The value is `coefficient * 10^-decimal_scale`. Construction removes
/// coefficient trailing zeroes, so numerically equal decimal text has one
/// equality, hash, display, and serialized identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OkxReferencePrice {
    coefficient: U256,
    decimal_scale: u8,
}

impl OkxReferencePrice {
    pub fn new(coefficient: U256, decimal_scale: u8) -> Result<Self, OkxReferencePriceError> {
        if coefficient.is_zero() {
            return Err(OkxReferencePriceError::Zero);
        }

        let mut coefficient = coefficient;
        let mut decimal_scale = decimal_scale;
        while decimal_scale > 0 {
            let (quotient, remainder) = coefficient
                .checked_div_rem_u32(10)
                .map_err(|_| OkxReferencePriceError::Overflow)?;
            if remainder != 0 {
                break;
            }
            coefficient = quotient;
            decimal_scale -= 1;
        }
        if decimal_scale > MAX_OKX_REFERENCE_DECIMAL_SCALE {
            return Err(OkxReferencePriceError::ScaleTooLarge);
        }
        Ok(Self {
            coefficient,
            decimal_scale,
        })
    }

    pub fn parse_decimal(input: &str) -> Result<Self, OkxReferencePriceError> {
        if input.is_empty() {
            return Err(OkxReferencePriceError::Empty);
        }
        if input.len() > MAX_OKX_REFERENCE_INPUT_BYTES {
            return Err(OkxReferencePriceError::InputTooLong);
        }

        let bytes = input.as_bytes();
        let mut dot_index = None;
        for (index, byte) in bytes.iter().copied().enumerate() {
            if byte == b'.' {
                if dot_index.replace(index).is_some() {
                    return Err(OkxReferencePriceError::InvalidFormat);
                }
            } else if !byte.is_ascii_digit() {
                return Err(OkxReferencePriceError::InvalidFormat);
            }
        }
        if dot_index.is_some_and(|index| index == 0 || index + 1 == bytes.len()) {
            return Err(OkxReferencePriceError::InvalidFormat);
        }

        let fractional_start = dot_index.map_or(bytes.len(), |index| index + 1);
        let mut significant_end = bytes.len();
        while significant_end > fractional_start && bytes[significant_end - 1] == b'0' {
            significant_end -= 1;
        }
        let decimal_scale = significant_end - fractional_start;
        if decimal_scale > usize::from(MAX_OKX_REFERENCE_DECIMAL_SCALE) {
            return Err(OkxReferencePriceError::ScaleTooLarge);
        }

        let mut coefficient = U256::ZERO;
        for (index, byte) in bytes[..significant_end].iter().copied().enumerate() {
            if Some(index) == dot_index {
                continue;
            }
            coefficient = coefficient
                .checked_mul_u32(10)
                .and_then(|value| value.checked_add(U256::from_u64(u64::from(byte - b'0'))))
                .map_err(OkxReferencePriceError::from_numeric)?;
        }
        Self::new(
            coefficient,
            u8::try_from(decimal_scale).expect("bounded decimal scale"),
        )
    }

    #[must_use]
    pub const fn coefficient(self) -> U256 {
        self.coefficient
    }

    #[must_use]
    pub const fn decimal_scale(self) -> u8 {
        self.decimal_scale
    }

    fn split_decimal(self) -> (U256, [u8; MAX_OKX_REFERENCE_DECIMAL_SCALE as usize]) {
        let mut whole = self.coefficient;
        let mut fractional = [b'0'; MAX_OKX_REFERENCE_DECIMAL_SCALE as usize];
        for index in (0..usize::from(self.decimal_scale)).rev() {
            let (quotient, remainder) = whole
                .checked_div_rem_u32(10)
                .expect("nonzero decimal divisor");
            fractional[index] = b'0' + u8::try_from(remainder).expect("single decimal digit");
            whole = quotient;
        }
        (whole, fractional)
    }
}

impl FromStr for OkxReferencePrice {
    type Err = OkxReferencePriceError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse_decimal(input)
    }
}

impl fmt::Display for OkxReferencePrice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (whole, fractional) = self.split_decimal();
        write!(formatter, "{whole}")?;
        if self.decimal_scale > 0 {
            formatter.write_str(".")?;
            let digits = &fractional[..usize::from(self.decimal_scale)];
            formatter.write_str(std::str::from_utf8(digits).map_err(|_| fmt::Error)?)?;
        }
        Ok(())
    }
}

impl Ord for OkxReferencePrice {
    fn cmp(&self, other: &Self) -> Ordering {
        let (self_whole, self_fractional) = self.split_decimal();
        let (other_whole, other_fractional) = other.split_decimal();
        self_whole
            .cmp(&other_whole)
            .then_with(|| self_fractional.cmp(&other_fractional))
    }
}

impl PartialOrd for OkxReferencePrice {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Serialize for OkxReferencePrice {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

struct OkxReferencePriceVisitor;

impl Visitor<'_> for OkxReferencePriceVisitor {
    type Value = OkxReferencePrice;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a quoted positive exact decimal OKX reference price")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        OkxReferencePrice::parse_decimal(value).map_err(E::custom)
    }
}

impl<'de> Deserialize<'de> for OkxReferencePrice {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(OkxReferencePriceVisitor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ThisError)]
pub enum OkxReferencePriceError {
    #[error("OKX reference price is empty")]
    Empty,
    #[error("OKX reference price exceeds its input bound")]
    InputTooLong,
    #[error("OKX reference price has invalid decimal syntax")]
    InvalidFormat,
    #[error("OKX reference price exceeds the supported decimal scale")]
    ScaleTooLarge,
    #[error("OKX reference price must be positive")]
    Zero,
    #[error("OKX reference price exceeds its fixed-width coefficient")]
    Overflow,
}

impl OkxReferencePriceError {
    fn from_numeric(error: PmNumericError) -> Self {
        match error {
            PmNumericError::Overflow => Self::Overflow,
            _ => Self::InvalidFormat,
        }
    }
}

/// An exact in-range PM price candidate in protocol millionths.
///
/// This value alone is not executable authority: market tick, metadata
/// revision, book, risk, account, and quote-profile approval are separate.
/// It intentionally does not implement `Deserialize`, because Serde has no
/// market tick context with which to reject an off-grid candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmPrice {
    units: u32,
}

impl PmPrice {
    pub fn from_units(units: u32) -> Result<Self, PmNumericError> {
        if units == 0 {
            return Err(PmNumericError::Zero);
        }
        if units >= PM_PROTOCOL_SCALE {
            return Err(PmNumericError::PriceAtOrAboveOne);
        }
        Ok(Self::from_units_unchecked(units))
    }

    pub fn parse_decimal(input: &str) -> Result<Self, PmNumericError> {
        let units = parse_scaled_decimal(input)?;
        if units.is_zero() {
            return Err(PmNumericError::Zero);
        }
        if units >= U256::from_u64(u64::from(PM_PROTOCOL_SCALE)) {
            return Err(PmNumericError::PriceAtOrAboveOne);
        }
        Self::from_units(units.0[0] as u32)
    }

    #[must_use]
    pub const fn units(self) -> u32 {
        self.units
    }

    pub fn validate_tick(self, tick: PmTick) -> Result<Self, PmNumericError> {
        if !self.units.is_multiple_of(tick.units) {
            Err(PmNumericError::PriceOffTick)
        } else {
            Ok(self)
        }
    }

    const fn from_units_unchecked(units: u32) -> Self {
        Self { units }
    }
}

impl FromStr for PmPrice {
    type Err = PmNumericError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse_decimal(input)
    }
}

impl fmt::Display for PmPrice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        format_scaled_units(U256::from_u64(u64::from(self.units)), formatter)
    }
}

impl Serialize for PmPrice {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u32(self.units)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmTick {
    units: u32,
}

impl PmTick {
    pub fn from_units(units: u32) -> Result<Self, PmNumericError> {
        match units {
            100_000 | 10_000 | 5_000 | 2_500 | 1_000 | 100 => Ok(Self::from_units_unchecked(units)),
            _ => Err(PmNumericError::UnsupportedTick),
        }
    }

    pub fn parse_decimal(input: &str) -> Result<Self, PmNumericError> {
        let price = PmPrice::parse_decimal(input)?;
        Self::from_units(price.units())
    }

    #[must_use]
    pub const fn units(self) -> u32 {
        self.units
    }

    const fn from_units_unchecked(units: u32) -> Self {
        Self { units }
    }
}

impl FromStr for PmTick {
    type Err = PmNumericError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse_decimal(input)
    }
}

impl fmt::Display for PmTick {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        format_scaled_units(U256::from_u64(u64::from(self.units)), formatter)
    }
}

impl Serialize for PmTick {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u32(self.units)
    }
}

struct PmTickVisitor;

impl Visitor<'_> for PmTickVisitor {
    type Value = PmTick;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("one of the six supported PM ticks in integral millionths")
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        let units = u32::try_from(value).map_err(E::custom)?;
        PmTick::from_units(units).map_err(E::custom)
    }
}

impl<'de> Deserialize<'de> for PmTick {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_u64(PmTickVisitor)
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmQuantity {
    protocol_units: U256,
}

impl PmQuantity {
    pub fn from_protocol_units(protocol_units: U256) -> Result<Self, PmNumericError> {
        if protocol_units.is_zero() {
            return Err(PmNumericError::Zero);
        }
        Ok(Self::from_protocol_units_unchecked(protocol_units))
    }

    pub fn parse_decimal(input: &str) -> Result<Self, PmNumericError> {
        Self::from_protocol_units(parse_scaled_decimal(input)?)
    }

    #[must_use]
    pub const fn protocol_units(self) -> U256 {
        self.protocol_units
    }

    pub fn validate_order(self, minimum: Self) -> Result<Self, PmNumericError> {
        let (_, lot_remainder) = self.protocol_units.checked_div_rem_u32(CLOB_V2_LOT_UNITS)?;
        if lot_remainder != 0 {
            return Err(PmNumericError::QuantityOffLot);
        }
        if self < minimum {
            return Err(PmNumericError::QuantityBelowMinimum);
        }
        Ok(self)
    }

    const fn from_protocol_units_unchecked(protocol_units: U256) -> Self {
        Self { protocol_units }
    }
}

impl FromStr for PmQuantity {
    type Err = PmNumericError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse_decimal(input)
    }
}

impl fmt::Display for PmQuantity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        format_scaled_units(self.protocol_units, formatter)
    }
}

impl Serialize for PmQuantity {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.protocol_units.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PmQuantity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let protocol_units = U256::deserialize(deserializer)?;
        Self::from_protocol_units(protocol_units).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PmBookQuantity {
    Delete,
    Quantity(PmQuantity),
}

impl PmBookQuantity {
    pub fn parse_decimal(input: &str) -> Result<Self, PmNumericError> {
        let units = parse_scaled_decimal(input)?;
        if units.is_zero() {
            Ok(Self::Delete)
        } else {
            Ok(Self::Quantity(PmQuantity::from_protocol_units_unchecked(
                units,
            )))
        }
    }

    #[must_use]
    pub fn from_protocol_units(units: U256) -> Self {
        if units.is_zero() {
            Self::Delete
        } else {
            Self::Quantity(PmQuantity::from_protocol_units_unchecked(units))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PmOrderSide {
    Buy,
    Sell,
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmOrderSalt(u64);

impl PmOrderSalt {
    pub fn from_u64(value: u64) -> Result<Self, PmNumericError> {
        if value > PM_ORDER_SALT_MAX {
            Err(PmNumericError::SaltOutsideJsonSafeInteger)
        } else {
            Ok(Self(value))
        }
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl Serialize for PmOrderSalt {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(self.0)
    }
}

struct PmOrderSaltVisitor;

impl Visitor<'_> for PmOrderSaltVisitor {
    type Value = PmOrderSalt;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an unsigned JSON-safe PM order salt")
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        PmOrderSalt::from_u64(value).map_err(E::custom)
    }
}

impl<'de> Deserialize<'de> for PmOrderSalt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_u64(PmOrderSaltVisitor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PmSign {
    Positive,
    Negative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmSignedUnits {
    sign: PmSign,
    magnitude: U256,
}

impl PmSignedUnits {
    pub const ZERO: Self = Self {
        sign: PmSign::Positive,
        magnitude: U256::ZERO,
    };

    pub fn from_parts(sign: PmSign, magnitude: U256) -> Result<Self, PmNumericError> {
        if magnitude.is_zero() {
            return if sign == PmSign::Negative {
                Err(PmNumericError::NonCanonicalSignedZero)
            } else {
                Ok(Self::ZERO)
            };
        }
        Ok(Self { sign, magnitude })
    }

    #[must_use]
    pub const fn sign(self) -> PmSign {
        self.sign
    }

    #[must_use]
    pub const fn magnitude(self) -> U256 {
        self.magnitude
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PmErc1155OperatorApproval(bool);

impl PmErc1155OperatorApproval {
    #[must_use]
    pub const fn from_bool(approved: bool) -> Self {
        Self(approved)
    }

    #[must_use]
    pub const fn is_approved(self) -> bool {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmOrderAmounts {
    maker: U256,
    taker: U256,
}

impl PmOrderAmounts {
    #[must_use]
    pub const fn maker(self) -> U256 {
        self.maker
    }

    #[must_use]
    pub const fn taker(self) -> U256 {
        self.taker
    }
}

pub fn exact_order_amounts(
    side: PmOrderSide,
    price: PmPrice,
    quantity: PmQuantity,
) -> Result<PmOrderAmounts, PmNumericError> {
    let (whole_shares, remaining_share_units) = quantity
        .protocol_units
        .checked_div_rem_u32(PM_PROTOCOL_SCALE)?;
    let whole_collateral = whole_shares.checked_mul_u32(price.units)?;
    let fractional_product = u64::from(remaining_share_units) * u64::from(price.units);
    let fractional_collateral = fractional_product / u64::from(PM_PROTOCOL_SCALE);
    let fractional_remainder = fractional_product % u64::from(PM_PROTOCOL_SCALE);
    if fractional_remainder != 0 {
        return Err(PmNumericError::NonIntegralOrderAmount);
    }
    let collateral = whole_collateral.checked_add(U256::from_u64(fractional_collateral))?;
    if collateral.is_zero() {
        return Err(PmNumericError::NonIntegralOrderAmount);
    }

    let shares = quantity.protocol_units;
    let (maker, taker) = match side {
        PmOrderSide::Buy => (collateral, shares),
        PmOrderSide::Sell => (shares, collateral),
    };
    Ok(PmOrderAmounts { maker, taker })
}

fn parse_integral_u256(input: &str) -> Result<U256, PmNumericError> {
    if input.is_empty() {
        return Err(PmNumericError::Empty);
    }
    if input.len() > MAX_DECIMAL_INPUT_BYTES {
        return Err(PmNumericError::InputTooLong);
    }
    if input.starts_with('-') {
        return Err(PmNumericError::Negative);
    }
    if input.starts_with('+') || !input.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(PmNumericError::InvalidFormat);
    }

    let mut value = U256::ZERO;
    for digit in input.bytes() {
        value = value
            .checked_mul_u32(10)?
            .checked_add_u32(u32::from(digit - b'0'))?;
    }
    Ok(value)
}

fn parse_scaled_decimal(input: &str) -> Result<U256, PmNumericError> {
    if input.is_empty() {
        return Err(PmNumericError::Empty);
    }
    if input.len() > MAX_DECIMAL_INPUT_BYTES {
        return Err(PmNumericError::InputTooLong);
    }
    if input.starts_with('-') {
        return Err(PmNumericError::Negative);
    }
    if input.starts_with('+') {
        return Err(PmNumericError::InvalidFormat);
    }

    let mut components = input.split('.');
    let integer = components
        .next()
        .expect("split always yields one component");
    let fractional = components.next();
    if components.next().is_some()
        || integer.is_empty()
        || !integer.bytes().all(|byte| byte.is_ascii_digit())
        || fractional.is_some_and(|digits| {
            digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return Err(PmNumericError::InvalidFormat);
    }

    let mut value = U256::ZERO;
    for digit in integer.bytes() {
        value = value
            .checked_mul_u32(10)?
            .checked_add_u32(u32::from(digit - b'0'))?;
    }

    let fractional = fractional.unwrap_or("");
    for index in 0..6 {
        let digit = fractional.as_bytes().get(index).copied().unwrap_or(b'0');
        value = value
            .checked_mul_u32(10)?
            .checked_add_u32(u32::from(digit - b'0'))?;
    }

    if fractional
        .as_bytes()
        .get(6..)
        .is_some_and(|remaining| remaining.iter().any(|digit| *digit != b'0'))
    {
        return Err(if value.is_zero() {
            PmNumericError::Underflow
        } else {
            PmNumericError::NonRepresentable
        });
    }

    Ok(value)
}

fn format_scaled_units(units: U256, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    let (whole, mut fractional) = units
        .checked_div_rem_u32(PM_PROTOCOL_SCALE)
        .map_err(|_| fmt::Error)?;
    write!(formatter, "{whole}")?;
    if fractional == 0 {
        return Ok(());
    }

    let mut width = 6_usize;
    while fractional.is_multiple_of(10) {
        fractional /= 10;
        width -= 1;
    }
    write!(formatter, ".{fractional:0width$}")
}
