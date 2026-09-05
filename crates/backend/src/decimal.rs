//! Checked conversion for decimal counters returned by external providers.

use std::fmt;

/// Why a provider decimal could not be represented as integer millionths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DecimalError {
    Invalid,
    Negative,
    Overflow,
}

impl fmt::Display for DecimalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid => formatter.write_str("value is not a decimal"),
            Self::Negative => formatter.write_str("value must be non-negative"),
            Self::Overflow => formatter.write_str("value is too large"),
        }
    }
}

/// Parse a provider decimal into integer millionths without floating point.
///
/// Accept provider decimal/scientific notation, a leading `+`, and an omitted
/// zero before the decimal point. Extra precision rounds to the nearest
/// millionth, with ties rounded up. Exponents shift digits directly, so even
/// extremely large exponents never allocate an exponent-sized intermediate.
pub(crate) fn decimal_to_millionths(value: &str) -> Result<u64, DecimalError> {
    let value = value.trim();
    if value.starts_with('-') {
        return Err(DecimalError::Negative);
    }
    let value = value.strip_prefix('+').unwrap_or(value);
    let (mantissa, exponent) = match value.find(['e', 'E']) {
        Some(index) => (&value[..index], parse_exponent(&value[index + 1..])?),
        None => (value, 0),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if (whole.is_empty() && fraction.is_empty())
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(DecimalError::Invalid);
    }
    let digits: Vec<u8> = whole.bytes().chain(fraction.bytes()).skip_while(|digit| *digit == b'0').collect();
    if digits.is_empty() {
        return Ok(0);
    }
    let shift = i128::from(exponent) + 6 - fraction.len() as i128;
    let (kept, trailing_zeros, round_up) = if shift >= 0 {
        if digits.len() as i128 + shift > 20 {
            return Err(DecimalError::Overflow);
        }
        (digits.len(), shift as usize, false)
    } else {
        let removed = -shift;
        if removed > digits.len() as i128 {
            return Ok(0);
        }
        let kept = digits.len() - removed as usize;
        (kept, 0, digits[kept] >= b'5')
    };
    let mut result = 0_u64;
    for digit in &digits[..kept] {
        result = result
            .checked_mul(10)
            .and_then(|value| value.checked_add(u64::from(digit - b'0')))
            .ok_or(DecimalError::Overflow)?;
    }
    for _ in 0..trailing_zeros {
        result = result.checked_mul(10).ok_or(DecimalError::Overflow)?;
    }
    result.checked_add(u64::from(round_up)).ok_or(DecimalError::Overflow)
}

fn parse_exponent(value: &str) -> Result<i64, DecimalError> {
    let negative = value.starts_with('-');
    let digits = value.strip_prefix(['+', '-']).unwrap_or(value);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(DecimalError::Invalid);
    }
    // Saturation preserves the result: a nonzero coefficient then either
    // overflows or rounds to zero; a zero coefficient always remains zero.
    let exponent =
        digits.bytes().fold(0_i64, |value, digit| value.saturating_mul(10).saturating_add(i64::from(digit - b'0')));
    Ok(if negative { -exponent } else { exponent })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_provider_decimal_spellings_and_rounds_long_fractions() {
        assert_eq!(decimal_to_millionths("+0.1"), Ok(100_000));
        assert_eq!(decimal_to_millionths(".5"), Ok(500_000));
        assert_eq!(decimal_to_millionths("1."), Ok(1_000_000));
        assert_eq!(decimal_to_millionths("1.234567499999"), Ok(1_234_567));
        assert_eq!(decimal_to_millionths("1.234567500000"), Ok(1_234_568));
    }

    #[test]
    fn scientific_provider_costs_preserve_exact_rounding_and_integer_boundaries() {
        for (text, expected) in [
            ("0E-28", 0),
            ("1e2", 100_000_000),
            ("+1.25E-3", 1250),
            ("4.999999999999e-7", 0),
            ("5e-7", 1),
            ("1.2345675e0", 1_234_568),
            ("1.8446744073709551615e13", u64::MAX),
            ("0.0003333333333333333333333333333", 333),
            ("0.00022659972310066222289062500", 227),
        ] {
            assert_eq!(decimal_to_millionths(text), Ok(expected), "{text}");
        }
        assert_eq!(decimal_to_millionths("1.84467440737095516155e13"), Err(DecimalError::Overflow));
        assert_eq!(decimal_to_millionths("1e999999999999999999999"), Err(DecimalError::Overflow));
        assert_eq!(decimal_to_millionths("1e-999999999999999999999"), Ok(0));
        assert_eq!(decimal_to_millionths("0e999999999999999999999"), Ok(0));
        for invalid in ["1e+", "1e-", "1e1e2", "1.2.3e4", "e1", "1e 2", "1e+-2"] {
            assert_eq!(decimal_to_millionths(invalid), Err(DecimalError::Invalid), "{invalid}");
        }
    }

    #[test]
    fn rejects_negative_invalid_and_overflowing_values() {
        assert_eq!(decimal_to_millionths("-0.1"), Err(DecimalError::Negative));
        for value in ["", "+", ".", "+.", "1e", "NaN", "1..2", "1,000"] {
            assert_eq!(decimal_to_millionths(value), Err(DecimalError::Invalid), "{value}");
        }

        assert_eq!(decimal_to_millionths("18446744073709.551615"), Ok(u64::MAX));
        assert_eq!(decimal_to_millionths("18446744073709.5516155"), Err(DecimalError::Overflow));
        assert_eq!(decimal_to_millionths("18446744073710"), Err(DecimalError::Overflow));
    }
}
