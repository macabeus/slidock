//! `Number(text)`, JavaScript's StringToNumber, with the rounding of JavaScriptCore, the engine the reference
//! output comes from: a `0x`, `0o` or `0b` integer past 2^53 is summed from its last digit, which can round its last
//! bit apart from the nearest double.

use crate::js_trim;

/// 2^53, from which a radix integer is no longer exact as it is read.
const EXACT: f64 = 9_007_199_254_740_992.0;

/// `Number(text)`: the literal between JavaScript whitespace, 0 when there is none, NaN when it is no decimal (with
/// an optional sign, or `Infinity`) and no unsigned `0x`, `0o` or `0b` integer.
pub fn js_number(text: &str) -> f64 {
    let literal = js_trim(text);
    if literal.is_empty() {
        return 0.0;
    }
    match radix_literal(literal) {
        Some((radix, digits)) => radix_integer(digits, radix),
        None => decimal(literal),
    }
}

/// The radix and digits of a literal with a `0x`, `0o` or `0b` prefix, in either case.
fn radix_literal(literal: &str) -> Option<(u32, &str)> {
    let mut chars = literal.strip_prefix('0')?.chars();
    let radix = match chars.next()? {
        'x' | 'X' => 16,
        'o' | 'O' => 8,
        'b' | 'B' => 2,
        _ => return None,
    };
    Some((radix, chars.as_str()))
}

fn radix_integer(digits: &str, radix: u32) -> f64 {
    let Some(values) = digits
        .chars()
        .map(|c| c.to_digit(radix))
        .collect::<Option<Vec<u32>>>()
        .filter(|values| !values.is_empty())
    else {
        return f64::NAN;
    };
    let base = f64::from(radix);
    let value = values
        .iter()
        .fold(0.0, |value, &digit| value * base + f64::from(digit));
    if value < EXACT {
        return value;
    }
    let mut sum = 0.0;
    let mut scale = 1.0;
    for &digit in values.iter().rev() {
        if scale == f64::INFINITY {
            if digit != 0 {
                return f64::INFINITY;
            }
        } else {
            sum += f64::from(digit) * scale;
        }
        scale *= base;
    }
    sum
}

/// `[+-]?(Infinity|digits[.digits][e[+-]digits]|.digits[e[+-]digits])`, correctly rounded.
fn decimal(literal: &str) -> f64 {
    let (sign, unsigned) = match literal.strip_prefix('-') {
        Some(rest) => (-1.0, rest),
        None => (1.0, literal.strip_prefix('+').unwrap_or(literal)),
    };
    if unsigned == "Infinity" {
        return sign * f64::INFINITY;
    }
    let starts = unsigned
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_digit() || b == b'.');
    let grammar = unsigned
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'));
    if starts && grammar {
        unsigned.parse::<f64>().map_or(f64::NAN, |n| sign * n)
    } else {
        f64::NAN
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;

    fn bits(n: f64) -> String {
        if n.is_nan() {
            "nan".to_owned()
        } else {
            format!("{:016x}", n.to_bits())
        }
    }

    /// `tests/data/js-number.json` holds Bun's `Number(text)` for each grammar StringToNumber accepts and near misses,
    /// radix integers past 2^53 up to overflow among them. It is made by
    /// `~/.slidock/tools/html-js-number-vectors.ts`.
    #[test]
    fn reads_numbers_as_javascript_does() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("../tests/data/js-number.json")).unwrap();
        let vectors = vectors["number"].as_array().unwrap();
        assert!(vectors.len() > 1500);
        let mut misses = Vec::new();
        for vector in vectors {
            let text = vector[0].as_str().unwrap();
            let expected = vector[1].as_str().unwrap();
            if bits(js_number(text)) != expected {
                misses.push(format!("{text:?}: {} != {expected}", bits(js_number(text))));
            }
        }
        assert!(
            misses.is_empty(),
            "{} misses: {:?}",
            misses.len(),
            &misses[..misses.len().min(5)]
        );
    }

    #[test]
    fn reads_radix_integers_with_either_prefix_case_and_no_sign() {
        assert_eq!(js_number("0x1f"), 31.0);
        assert_eq!(js_number("0X1F"), 31.0);
        assert_eq!(js_number("0o17"), 15.0);
        assert_eq!(js_number("0b101"), 5.0);
        assert_eq!(js_number("\u{a0}0x10\u{2028}"), 16.0);
        for text in ["0x", "-0x10", "+0x10", "0x1g", "0o8", "0b2", "00x1", "0x_1"] {
            assert!(js_number(text).is_nan(), "{text}");
        }
    }

    #[test]
    fn sums_a_radix_integer_past_two_to_the_53_from_its_last_digit() {
        // 2^64 - 1025 is nearer 2^64 - 2048, but the sum from the last digit rounds it up to 2^64.
        assert_eq!(
            js_number("0xfffffffffffffbff"),
            18_446_744_073_709_551_616.0
        );
        assert_eq!(js_number("0x20000000000003"), 9_007_199_254_740_996.0);
        assert_eq!(js_number(&format!("0x{}", "f".repeat(256))), f64::INFINITY);
        assert_eq!(js_number(&format!("0x1{}", "0".repeat(300))), f64::INFINITY);
    }

    #[test]
    fn reads_decimals_with_a_sign_and_infinity_in_its_own_case() {
        assert_eq!(js_number(""), 0.0);
        assert_eq!(js_number(" "), 0.0);
        assert!(js_number("-0").is_sign_negative());
        assert_eq!(js_number("+.5"), 0.5);
        assert_eq!(js_number("1.e5"), 100_000.0);
        assert_eq!(js_number("-Infinity"), f64::NEG_INFINITY);
        assert_eq!(js_number("1e400"), f64::INFINITY);
        for text in [
            "infinity", "inf", "NaN", "1e", ".", "+-1", "1_000", "1n", "\u{85}1",
        ] {
            assert!(js_number(text).is_nan(), "{text}");
        }
    }
}
