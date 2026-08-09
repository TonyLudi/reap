use std::fmt;

use reap_pm_core::U256;

use crate::PmPublicPositionError;

pub const MAX_POSITION_DECIMAL_BYTES: usize = 96;
const MAX_ABSOLUTE_DECIMAL_EXPONENT: i32 = 256;
const PM_PROTOCOL_DECIMAL_PLACES: i16 = 6;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PmExactPositionDecimal {
    lexeme: Box<str>,
    negative: bool,
    coefficient: U256,
    decimal_exponent: i16,
}

impl PmExactPositionDecimal {
    pub(crate) fn parse(
        field: &'static str,
        input: &str,
        nonnegative: bool,
    ) -> Result<Self, PmPublicPositionError> {
        if input.is_empty() {
            return Err(PmPublicPositionError::InvalidField(field));
        }
        if input.len() > MAX_POSITION_DECIMAL_BYTES {
            return Err(PmPublicPositionError::FieldTooLong(field));
        }

        let bytes = input.as_bytes();
        let mut cursor = 0_usize;
        let negative = bytes.first() == Some(&b'-');
        if negative {
            if nonnegative {
                return Err(PmPublicPositionError::InvalidField(field));
            }
            cursor += 1;
        } else if bytes.first() == Some(&b'+') {
            return Err(PmPublicPositionError::InvalidField(field));
        }

        let integer_start = cursor;
        match bytes.get(cursor).copied() {
            Some(b'0') => {
                cursor += 1;
                if bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
                    return Err(PmPublicPositionError::InvalidField(field));
                }
            }
            Some(b'1'..=b'9') => {
                cursor += 1;
                while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
                    cursor += 1;
                }
            }
            _ => return Err(PmPublicPositionError::InvalidField(field)),
        }
        let integer_end = cursor;

        let mut fractional_start = cursor;
        let mut fractional_end = cursor;
        if bytes.get(cursor) == Some(&b'.') {
            cursor += 1;
            fractional_start = cursor;
            while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
                cursor += 1;
            }
            fractional_end = cursor;
            if fractional_start == fractional_end {
                return Err(PmPublicPositionError::InvalidField(field));
            }
        }

        let mut explicit_exponent = 0_i32;
        if matches!(bytes.get(cursor), Some(b'e' | b'E')) {
            cursor += 1;
            let exponent_negative = bytes.get(cursor) == Some(&b'-');
            if exponent_negative || bytes.get(cursor) == Some(&b'+') {
                cursor += 1;
            }
            let exponent_start = cursor;
            while let Some(digit @ b'0'..=b'9') = bytes.get(cursor).copied() {
                explicit_exponent = explicit_exponent
                    .checked_mul(10)
                    .and_then(|value| value.checked_add(i32::from(digit - b'0')))
                    .ok_or(PmPublicPositionError::InvalidField(field))?;
                if explicit_exponent
                    > MAX_ABSOLUTE_DECIMAL_EXPONENT
                        + i32::try_from(MAX_POSITION_DECIMAL_BYTES).expect("small decimal bound")
                {
                    return Err(PmPublicPositionError::InvalidField(field));
                }
                cursor += 1;
            }
            if exponent_start == cursor {
                return Err(PmPublicPositionError::InvalidField(field));
            }
            if exponent_negative {
                explicit_exponent = -explicit_exponent;
            }
        }
        if cursor != bytes.len() {
            return Err(PmPublicPositionError::InvalidField(field));
        }

        let mut coefficient = U256::ZERO;
        for digit in bytes[integer_start..integer_end]
            .iter()
            .chain(bytes[fractional_start..fractional_end].iter())
            .copied()
        {
            coefficient = coefficient
                .checked_mul_u32(10)
                .and_then(|value| value.checked_add(U256::from_u64(u64::from(digit - b'0'))))
                .map_err(|_| PmPublicPositionError::InvalidField(field))?;
        }
        if negative && coefficient.is_zero() {
            return Err(PmPublicPositionError::InvalidField(field));
        }

        let fractional_digits =
            i32::try_from(fractional_end - fractional_start).expect("bounded decimal input");
        let decimal_exponent = explicit_exponent - fractional_digits;
        if decimal_exponent.abs() > MAX_ABSOLUTE_DECIMAL_EXPONENT {
            return Err(PmPublicPositionError::InvalidField(field));
        }

        Ok(Self {
            lexeme: input.into(),
            negative,
            coefficient,
            decimal_exponent: i16::try_from(decimal_exponent)
                .expect("bounded exact position exponent"),
        })
    }

    #[must_use]
    pub fn lexeme(&self) -> &str {
        &self.lexeme
    }

    #[must_use]
    pub const fn is_negative(&self) -> bool {
        self.negative
    }

    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.coefficient.is_zero()
    }

    #[must_use]
    pub const fn coefficient(&self) -> U256 {
        self.coefficient
    }

    /// The exact value is `sign * coefficient * 10^decimal_exponent`.
    #[must_use]
    pub const fn decimal_exponent(&self) -> i16 {
        self.decimal_exponent
    }

    /// Converts this exact decimal into Polymarket's six-decimal protocol
    /// units without rounding or crossing binary floating point.
    ///
    /// Negative values, sub-unit fractions, and values whose scaled integer
    /// exceeds `U256` are rejected. Zero remains representable because a
    /// monitored position may be present with an exact zero size.
    pub(crate) fn to_protocol_units_exact(
        &self,
        field: &'static str,
    ) -> Result<U256, PmPublicPositionError> {
        if self.negative {
            return Err(PmPublicPositionError::NonRepresentableProtocolUnits(field));
        }

        let scaled_exponent = self
            .decimal_exponent
            .checked_add(PM_PROTOCOL_DECIMAL_PLACES)
            .ok_or(PmPublicPositionError::NonRepresentableProtocolUnits(field))?;
        let mut units = self.coefficient;
        if scaled_exponent >= 0 {
            for _ in 0..scaled_exponent {
                units = units
                    .checked_mul_u32(10)
                    .map_err(|_| PmPublicPositionError::NonRepresentableProtocolUnits(field))?;
            }
        } else {
            for _ in scaled_exponent..0 {
                let (quotient, remainder) = units
                    .checked_div_rem_u32(10)
                    .map_err(|_| PmPublicPositionError::NonRepresentableProtocolUnits(field))?;
                if remainder != 0 {
                    return Err(PmPublicPositionError::NonRepresentableProtocolUnits(field));
                }
                units = quotient;
            }
        }
        Ok(units)
    }
}

impl fmt::Display for PmExactPositionDecimal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.lexeme)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_decimal_lexemes_without_binary_floating_point() {
        let decimal = PmExactPositionDecimal::parse("size", "12.3400e-2", true).unwrap();
        assert_eq!(decimal.lexeme(), "12.3400e-2");
        assert_eq!(decimal.coefficient(), U256::from_u64(123_400));
        assert_eq!(decimal.decimal_exponent(), -6);
        assert!(!decimal.is_negative());

        let signed = PmExactPositionDecimal::parse("cashPnl", "-0.125", false).unwrap();
        assert!(signed.is_negative());
        assert_eq!(signed.coefficient(), U256::from_u64(125));
        assert_eq!(signed.decimal_exponent(), -3);
    }

    #[test]
    fn rejects_noncanonical_negative_or_unbounded_numbers() {
        for invalid in ["", "+1", "01", "1.", ".1", "1e", "-0", "1e257"] {
            assert!(PmExactPositionDecimal::parse("size", invalid, false).is_err());
        }
        assert!(PmExactPositionDecimal::parse("size", "-1", true).is_err());
        let oversized = "1".repeat(MAX_POSITION_DECIMAL_BYTES + 1);
        assert_eq!(
            PmExactPositionDecimal::parse("size", &oversized, true),
            Err(PmPublicPositionError::FieldTooLong("size"))
        );
    }

    #[test]
    fn converts_only_exact_six_decimal_protocol_units() {
        let ordinary = PmExactPositionDecimal::parse("size", "12.3400e-2", true).unwrap();
        assert_eq!(
            ordinary.to_protocol_units_exact("size"),
            Ok(U256::from_u64(123_400))
        );

        let precise = PmExactPositionDecimal::parse("size", "0.000001", true).unwrap();
        assert_eq!(precise.to_protocol_units_exact("size"), Ok(U256::ONE));

        let zero = PmExactPositionDecimal::parse("size", "0", true).unwrap();
        assert_eq!(zero.to_protocol_units_exact("size"), Ok(U256::ZERO));

        let subunit = PmExactPositionDecimal::parse("size", "0.0000001", true).unwrap();
        assert_eq!(
            subunit.to_protocol_units_exact("size"),
            Err(PmPublicPositionError::NonRepresentableProtocolUnits("size"))
        );

        let negative = PmExactPositionDecimal::parse("cashPnl", "-1", false).unwrap();
        assert_eq!(
            negative.to_protocol_units_exact("cashPnl"),
            Err(PmPublicPositionError::NonRepresentableProtocolUnits(
                "cashPnl"
            ))
        );

        let overflow = PmExactPositionDecimal::parse("size", "1e256", true).unwrap();
        assert_eq!(
            overflow.to_protocol_units_exact("size"),
            Err(PmPublicPositionError::NonRepresentableProtocolUnits("size"))
        );
    }
}
