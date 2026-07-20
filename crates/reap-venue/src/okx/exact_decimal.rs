use std::fmt;
use std::str::FromStr;

use thiserror::Error;

const MAX_EXACT_DECIMAL_INPUT_BYTES: usize = 512;
const MAX_ABS_BASE_TEN_EXPONENT: i32 = 400;

/// A positive exchange decimal retained without a binary floating-point round trip.
///
/// The representation is normalized as a checked `u128` coefficient multiplied
/// by a bounded power of ten. It deliberately has no Serde implementation:
/// callers may inspect or format values received from trusted venue metadata,
/// but cannot accidentally turn this sidecar into a persisted public schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OkxExactDecimal {
    coefficient: u128,
    base_ten_exponent: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum OkxExactDecimalError {
    #[error("exact decimal input is empty")]
    Empty,
    #[error("exact decimal input exceeds its size bound")]
    InputTooLong,
    #[error("exact decimal must not be negative")]
    Negative,
    #[error("exact decimal has invalid syntax")]
    InvalidFormat,
    #[error("exact decimal must be positive")]
    Zero,
    #[error("exact decimal coefficient exceeds u128")]
    CoefficientOverflow,
    #[error("exact decimal base-ten exponent is outside its supported bound")]
    ExponentOutOfRange,
    #[error("exact decimal underflows its required finite positive f64 projection")]
    Underflow,
    #[error("exact decimal overflows its required finite f64 projection")]
    Overflow,
    #[error("minimum order size is not an integral number of lots")]
    MinimumSizeNotIntegralLots,
}

impl OkxExactDecimal {
    pub fn parse(input: &str) -> Result<Self, OkxExactDecimalError> {
        if input.is_empty() {
            return Err(OkxExactDecimalError::Empty);
        }
        if input.len() > MAX_EXACT_DECIMAL_INPUT_BYTES {
            return Err(OkxExactDecimalError::InputTooLong);
        }

        let unsigned = if let Some(rest) = input.strip_prefix('-') {
            if rest.is_empty() {
                return Err(OkxExactDecimalError::InvalidFormat);
            }
            return Err(OkxExactDecimalError::Negative);
        } else {
            input.strip_prefix('+').unwrap_or(input)
        };
        if unsigned.is_empty() {
            return Err(OkxExactDecimalError::InvalidFormat);
        }

        let (mantissa, explicit_exponent) = split_exponent(unsigned)?;
        let (coefficient, fractional_digits, trailing_zeros) = parse_mantissa(mantissa)?;
        if coefficient == 0 {
            return Err(OkxExactDecimalError::Zero);
        }

        let projected = input
            .parse::<f64>()
            .map_err(|_| OkxExactDecimalError::InvalidFormat)?;
        if projected == 0.0 {
            return Err(OkxExactDecimalError::Underflow);
        }
        if !projected.is_finite() {
            return Err(OkxExactDecimalError::Overflow);
        }

        let base_ten_exponent = explicit_exponent
            .checked_sub(fractional_digits)
            .and_then(|value| value.checked_add(trailing_zeros))
            .ok_or(OkxExactDecimalError::ExponentOutOfRange)?;
        Self::from_normalized_parts(coefficient, base_ten_exponent)
    }

    /// The finite positive model-number projection already validated at parse time.
    #[must_use]
    pub fn as_f64(self) -> f64 {
        let projected = self.to_string().parse::<f64>().unwrap_or(f64::INFINITY);
        debug_assert!(projected.is_finite() && projected > 0.0);
        projected
    }

    /// Multiplies this increment by a positive integral unit count.
    ///
    /// Trailing decimal zeroes in the unit count are folded into the exponent
    /// before multiplication, avoiding false coefficient overflow.
    pub fn checked_mul_units(self, units: u64) -> Result<Self, OkxExactDecimalError> {
        if units == 0 {
            return Err(OkxExactDecimalError::Zero);
        }
        let mut reduced_units = u128::from(units);
        let mut additional_exponent = 0_i32;
        while reduced_units.is_multiple_of(10) {
            reduced_units /= 10;
            additional_exponent += 1;
        }
        let mut reduced_coefficient = self.coefficient;
        while reduced_coefficient.is_multiple_of(2) && reduced_units.is_multiple_of(5) {
            reduced_coefficient /= 2;
            reduced_units /= 5;
            additional_exponent += 1;
        }
        while reduced_coefficient.is_multiple_of(5) && reduced_units.is_multiple_of(2) {
            reduced_coefficient /= 5;
            reduced_units /= 2;
            additional_exponent += 1;
        }
        let coefficient = reduced_coefficient
            .checked_mul(reduced_units)
            .ok_or(OkxExactDecimalError::CoefficientOverflow)?;
        let exponent = i32::from(self.base_ten_exponent)
            .checked_add(additional_exponent)
            .ok_or(OkxExactDecimalError::ExponentOutOfRange)?;
        let product = Self::from_parts_normalizing(coefficient, exponent)?;
        let projected = product.to_string().parse::<f64>().map_err(|_| {
            if exponent.is_negative() {
                OkxExactDecimalError::Underflow
            } else {
                OkxExactDecimalError::Overflow
            }
        })?;
        if projected == 0.0 {
            return Err(OkxExactDecimalError::Underflow);
        }
        if !projected.is_finite() {
            return Err(OkxExactDecimalError::Overflow);
        }
        Ok(product)
    }

    fn from_parts_normalizing(
        mut coefficient: u128,
        mut base_ten_exponent: i32,
    ) -> Result<Self, OkxExactDecimalError> {
        if coefficient == 0 {
            return Err(OkxExactDecimalError::Zero);
        }
        while coefficient.is_multiple_of(10) {
            coefficient /= 10;
            base_ten_exponent = base_ten_exponent
                .checked_add(1)
                .ok_or(OkxExactDecimalError::ExponentOutOfRange)?;
        }
        Self::from_normalized_parts(coefficient, base_ten_exponent)
    }

    fn from_normalized_parts(
        coefficient: u128,
        base_ten_exponent: i32,
    ) -> Result<Self, OkxExactDecimalError> {
        if !(-MAX_ABS_BASE_TEN_EXPONENT..=MAX_ABS_BASE_TEN_EXPONENT).contains(&base_ten_exponent) {
            return Err(OkxExactDecimalError::ExponentOutOfRange);
        }
        let base_ten_exponent = i16::try_from(base_ten_exponent)
            .map_err(|_| OkxExactDecimalError::ExponentOutOfRange)?;
        Ok(Self {
            coefficient,
            base_ten_exponent,
        })
    }

    fn is_integral_multiple_of(self, increment: Self) -> bool {
        let exponent_delta =
            i32::from(self.base_ten_exponent) - i32::from(increment.base_ten_exponent);
        if exponent_delta >= 0 {
            // self / increment = (self.coefficient * 10^delta) / increment.coefficient.
            // Avoid overflowing the numerator: after cancelling the coefficient
            // gcd, the remaining divisor only needs factors supplied by 10^delta.
            let common = gcd(self.coefficient, increment.coefficient);
            let mut remaining = increment.coefficient / common;
            let mut twos = exponent_delta;
            while twos > 0 && remaining.is_multiple_of(2) {
                remaining /= 2;
                twos -= 1;
            }
            let mut fives = exponent_delta;
            while fives > 0 && remaining.is_multiple_of(5) {
                remaining /= 5;
                fives -= 1;
            }
            remaining == 1
        } else {
            // self / increment = self.coefficient /
            // (increment.coefficient * 10^(-delta)). Divide in stages so the
            // mathematical denominator never needs to fit in u128.
            if !self.coefficient.is_multiple_of(increment.coefficient) {
                return false;
            }
            let mut quotient = self.coefficient / increment.coefficient;
            for _ in 0..(-exponent_delta) {
                if !quotient.is_multiple_of(10) {
                    return false;
                }
                quotient /= 10;
            }
            true
        }
    }
}

impl FromStr for OkxExactDecimal {
    type Err = OkxExactDecimalError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input)
    }
}

impl fmt::Display for OkxExactDecimal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let coefficient = self.coefficient.to_string();
        let exponent = i32::from(self.base_ten_exponent);
        if exponent >= 0 {
            formatter.write_str(&coefficient)?;
            for _ in 0..exponent {
                formatter.write_str("0")?;
            }
            return Ok(());
        }

        let decimal_position = i32::try_from(coefficient.len()).map_err(|_| fmt::Error)? + exponent;
        if decimal_position > 0 {
            let decimal_position = usize::try_from(decimal_position).map_err(|_| fmt::Error)?;
            formatter.write_str(&coefficient[..decimal_position])?;
            formatter.write_str(".")?;
            formatter.write_str(&coefficient[decimal_position..])
        } else {
            formatter.write_str("0.")?;
            for _ in 0..-decimal_position {
                formatter.write_str("0")?;
            }
            formatter.write_str(&coefficient)
        }
    }
}

/// Exact regular-order increments captured from one OKX instrument response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OkxRegularOrderRules {
    tick_size: OkxExactDecimal,
    lot_size: OkxExactDecimal,
    min_size: OkxExactDecimal,
}

impl OkxRegularOrderRules {
    pub fn from_exchange_decimals(
        tick_size: &str,
        lot_size: &str,
        min_size: &str,
    ) -> Result<Self, OkxExactDecimalError> {
        Self::new(
            OkxExactDecimal::parse(tick_size)?,
            OkxExactDecimal::parse(lot_size)?,
            OkxExactDecimal::parse(min_size)?,
        )
    }

    pub fn new(
        tick_size: OkxExactDecimal,
        lot_size: OkxExactDecimal,
        min_size: OkxExactDecimal,
    ) -> Result<Self, OkxExactDecimalError> {
        if !min_size.is_integral_multiple_of(lot_size) {
            return Err(OkxExactDecimalError::MinimumSizeNotIntegralLots);
        }
        Ok(Self {
            tick_size,
            lot_size,
            min_size,
        })
    }

    #[must_use]
    pub fn tick_size(self) -> OkxExactDecimal {
        self.tick_size
    }

    #[must_use]
    pub fn lot_size(self) -> OkxExactDecimal {
        self.lot_size
    }

    #[must_use]
    pub fn min_size(self) -> OkxExactDecimal {
        self.min_size
    }
}

fn split_exponent(input: &str) -> Result<(&str, i32), OkxExactDecimalError> {
    let mut marker = None;
    for (index, byte) in input.bytes().enumerate() {
        if (byte == b'e' || byte == b'E') && marker.replace(index).is_some() {
            return Err(OkxExactDecimalError::InvalidFormat);
        }
    }
    let Some(marker) = marker else {
        return Ok((input, 0));
    };
    let mantissa = &input[..marker];
    let exponent = &input[marker + 1..];
    if mantissa.is_empty() || exponent.is_empty() {
        return Err(OkxExactDecimalError::InvalidFormat);
    }
    let exponent = exponent
        .parse::<i32>()
        .map_err(|_| OkxExactDecimalError::ExponentOutOfRange)?;
    Ok((mantissa, exponent))
}

fn parse_mantissa(input: &str) -> Result<(u128, i32, i32), OkxExactDecimalError> {
    let mut coefficient = 0_u128;
    let mut saw_digit = false;
    let mut saw_decimal_point = false;
    let mut fractional_digits = 0_i32;
    let mut pending_zeros = 0_i32;
    let mut saw_non_zero = false;

    for byte in input.bytes() {
        if byte == b'.' {
            if saw_decimal_point {
                return Err(OkxExactDecimalError::InvalidFormat);
            }
            saw_decimal_point = true;
            continue;
        }
        if !byte.is_ascii_digit() {
            return Err(OkxExactDecimalError::InvalidFormat);
        }
        saw_digit = true;
        if saw_decimal_point {
            fractional_digits = fractional_digits
                .checked_add(1)
                .ok_or(OkxExactDecimalError::ExponentOutOfRange)?;
        }
        let digit = byte - b'0';
        if digit == 0 {
            if saw_non_zero {
                pending_zeros = pending_zeros
                    .checked_add(1)
                    .ok_or(OkxExactDecimalError::ExponentOutOfRange)?;
            }
            continue;
        }

        saw_non_zero = true;
        for _ in 0..pending_zeros {
            coefficient = coefficient
                .checked_mul(10)
                .ok_or(OkxExactDecimalError::CoefficientOverflow)?;
        }
        pending_zeros = 0;
        coefficient = coefficient
            .checked_mul(10)
            .and_then(|value| value.checked_add(u128::from(digit)))
            .ok_or(OkxExactDecimalError::CoefficientOverflow)?;
    }

    if !saw_digit {
        return Err(OkxExactDecimalError::InvalidFormat);
    }
    Ok((coefficient, fractional_digits, pending_zeros))
}

fn gcd(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_decimal_normalizes_plain_and_scientific_exchange_values() {
        let cases = [
            ("1", "1"),
            ("1.0", "1"),
            ("001.2300", "1.23"),
            (".1", "0.1"),
            ("1.", "1"),
            ("+0.0500", "0.05"),
            ("1e-4", "0.0001"),
            ("1.25E+3", "1250"),
            ("1000e-2", "10"),
        ];

        for (wire, canonical) in cases {
            let parsed = OkxExactDecimal::parse(wire).unwrap();
            assert_eq!(parsed.to_string(), canonical, "{wire}");
            assert_eq!(wire.parse::<OkxExactDecimal>().unwrap(), parsed);
            assert!(parsed.as_f64().is_finite());
            assert!(parsed.as_f64() > 0.0);
        }
    }

    #[test]
    fn exact_decimal_rejects_non_positive_malformed_and_unrepresentable_values() {
        for wire in [
            "", ".", "e1", "1e", "1e+", "--1", "1.2.3", "1 2", " 1", "1 ", "NaN", "inf", "0",
            "0.000", "-0", "-1",
        ] {
            assert!(OkxExactDecimal::parse(wire).is_err(), "{wire}");
        }

        assert_eq!(
            OkxExactDecimal::parse("1e-400"),
            Err(OkxExactDecimalError::Underflow)
        );
        assert_eq!(
            OkxExactDecimal::parse("1e400"),
            Err(OkxExactDecimalError::Overflow)
        );
        assert!(matches!(
            OkxExactDecimal::parse("340282366920938463463374607431768211456"),
            Err(OkxExactDecimalError::CoefficientOverflow)
        ));

        let oversized = "1".repeat(513);
        assert_eq!(
            OkxExactDecimal::parse(&oversized),
            Err(OkxExactDecimalError::InputTooLong)
        );
    }

    #[test]
    fn exact_decimal_checked_unit_multiplication_is_canonical_and_bounded() {
        assert_eq!(
            OkxExactDecimal::parse("0.05")
                .unwrap()
                .checked_mul_units(3)
                .unwrap()
                .to_string(),
            "0.15"
        );
        assert_eq!(
            OkxExactDecimal::parse("1e-4")
                .unwrap()
                .checked_mul_units(12)
                .unwrap()
                .to_string(),
            "0.0012"
        );
        assert_eq!(
            OkxExactDecimal::parse("1").unwrap().checked_mul_units(0),
            Err(OkxExactDecimalError::Zero)
        );
        assert_eq!(
            OkxExactDecimal::parse("340282366920938463463374607431768211455")
                .unwrap()
                .checked_mul_units(2)
                .unwrap()
                .to_string(),
            "680564733841876926926749214863536422910"
        );
        assert_eq!(
            OkxExactDecimal::parse("1e308")
                .unwrap()
                .checked_mul_units(10),
            Err(OkxExactDecimalError::Overflow)
        );
    }

    #[test]
    fn regular_order_rules_require_minimum_size_to_be_integral_lots() {
        let rules = OkxRegularOrderRules::from_exchange_decimals("0.1", "0.05", "0.10").unwrap();
        assert_eq!(rules.tick_size().to_string(), "0.1");
        assert_eq!(rules.lot_size().to_string(), "0.05");
        assert_eq!(rules.min_size().to_string(), "0.1");

        assert_eq!(
            OkxRegularOrderRules::from_exchange_decimals("0.1", "0.05", "0.06"),
            Err(OkxExactDecimalError::MinimumSizeNotIntegralLots)
        );
        assert_eq!(
            OkxRegularOrderRules::from_exchange_decimals("0.1", "10", "1"),
            Err(OkxExactDecimalError::MinimumSizeNotIntegralLots)
        );
    }
}
