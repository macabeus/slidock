//! The JavaScript number semantics the metrics depend on: `Math.round`, `Math.max`/`Math.min`, typed-array stores
//! and sorts, `**` with an integer exponent, and code-unit string order.

use core::cmp::Ordering;

use crate::sort::stable_sort;

/// 2^52: every double at least this large is an integer.
const INTEGRAL: f64 = 4_503_599_627_370_496.0;

/// `Math.round`: the nearest integer, a half rounding toward +∞, the sign of zero kept (`Math.round(-0.4)` is −0).
pub fn js_round(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 || x.abs() >= INTEGRAL {
        return x;
    }
    if (-0.5..0.0).contains(&x) {
        return -0.0;
    }
    // Exact for |x| < 2^52 (Sterbenz), so the comparison with ½ is too.
    let floor = x.floor();
    if x - floor >= 0.5 { floor + 1.0 } else { floor }
}

/// A value on the results grid of 1e-4, with −0 as +0.
pub fn quantise(value: f64) -> f64 {
    js_round(value * 10_000.0) / 10_000.0 + 0.0
}

/// `Math.max(a, b)`: NaN when either is NaN, and +0 above −0.
pub fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        if a.is_sign_positive() { a } else { b }
    } else if a > b {
        a
    } else {
        b
    }
}

/// `Math.min(a, b)`: NaN when either is NaN, and −0 below +0.
pub fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        if a.is_sign_negative() { a } else { b }
    } else if a < b {
        a
    } else {
        b
    }
}

/// A store into a `Uint8Array` (ToUint8): NaN and infinities are 0, everything else is truncated modulo 256.
pub fn to_uint8(x: f64) -> u8 {
    if x.is_finite() {
        x.trunc().rem_euclid(256.0) as u8
    } else {
        0
    }
}

/// `x ** n` for a non-negative integer `n`, as JavaScriptCore computes it: square and multiply.
pub fn pow_int(mut x: f64, mut n: u32) -> f64 {
    let mut result = 1.0;
    while n != 0 {
        if n & 1 != 0 {
            result *= x;
        }
        x *= x;
        n >>= 1;
    }
    result
}

/// The order of `Float64Array.prototype.sort` and `Float32Array.prototype.sort`: ascending, −0 before +0, NaN last.
pub fn typed_order(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => a.total_cmp(&b),
    }
}

/// `Array.prototype.sort((a, b) => a - b)`, JavaScriptCore's algorithm: a NaN difference counts as neither less nor
/// greater, and equal values keep their order.
pub fn sort_by_difference(values: &mut [f64]) {
    stable_sort(values, |a, b| a - b < 0.0);
}

/// The value at fraction `p` of an ascending list (nearest rank); NaN for an empty list.
pub fn percentile<T: Copy + Into<f64>>(sorted: &[T], p: f64) -> f64 {
    let n = sorted.len() as f64;
    let index = js_min(n - 1.0, js_max(0.0, (p * n).ceil() - 1.0));
    if index.is_nan() {
        return f64::NAN;
    }
    sorted
        .get(index as usize)
        .map_or(f64::NAN, |&value| value.into())
}

/// The median: the middle value, or the mean of the two middle ones; NaN for an empty list.
pub fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sort_by_difference(&mut sorted);
    let mid = sorted.len() >> 1;
    let at = |i: usize| sorted.get(i).copied().unwrap_or(f64::NAN);
    match sorted.len() {
        0 => f64::NAN,
        n if n % 2 == 1 => at(mid),
        _ => (at(mid - 1) + at(mid)) / 2.0,
    }
}

/// JavaScript's `<` on strings: UTF-16 code-unit order.
pub fn utf16_cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// The characters `String.prototype.trim` removes: ECMAScript WhiteSpace and LineTerminator. Unlike
/// `char::is_whitespace` this holds U+FEFF and not U+0085.
pub fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'
            | '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same(a: f64, b: f64) -> bool {
        a.to_bits() == b.to_bits()
    }

    #[test]
    fn round_follows_math_round() {
        let cases = [
            (0.49999999999999994, 0.0),
            (-0.4, -0.0),
            (-0.5, -0.0),
            (-2.5, -2.0),
            (2.5, 3.0),
            (-0.5000000000000001, -1.0),
            (4_503_599_627_370_495.5, 4_503_599_627_370_496.0),
            (-4_503_599_627_370_495.5, -4_503_599_627_370_495.0),
            (1.5, 2.0),
            (-1.5, -1.0),
            (0.5, 1.0),
            (-0.0, -0.0),
            (0.3, 0.0),
        ];
        for (x, want) in cases {
            assert!(
                same(js_round(x), want),
                "Math.round({x}) = {want}, not {}",
                js_round(x)
            );
        }
        assert!(js_round(f64::NAN).is_nan());
    }

    #[test]
    fn quantise_drops_negative_zero() {
        assert!(same(quantise(-0.00001), 0.0));
        assert_eq!(quantise(0.123_456), 0.1235);
        assert_eq!(quantise(2.104_549_9), 2.1045);
        assert_eq!(quantise(-0.247_35), -0.2473);
    }

    #[test]
    fn max_and_min_propagate_nan_and_order_zeros() {
        assert!(js_max(1.0, f64::NAN).is_nan());
        assert!(js_min(f64::NAN, 1.0).is_nan());
        assert!(same(js_max(-0.0, 0.0), 0.0));
        assert!(same(js_max(0.0, -0.0), 0.0));
        assert!(same(js_min(0.0, -0.0), -0.0));
        assert_eq!(js_max(2.0, 3.0), 3.0);
        assert_eq!(js_min(2.0, 3.0), 2.0);
    }

    #[test]
    fn uint8_store() {
        assert_eq!(to_uint8(f64::NAN), 0);
        assert_eq!(to_uint8(255.9), 255);
        assert_eq!(to_uint8(256.0), 0);
        assert_eq!(to_uint8(-1.0), 255);
    }

    #[test]
    fn pow_int_squares_and_multiplies() {
        let x = 37.123_456_789_f64;
        assert!(same(pow_int(x, 2), x * x));
        let x2 = x * x;
        assert!(same(pow_int(x, 7), (x * x2) * (x2 * x2)));
        assert_eq!(pow_int(25.0, 7), 6_103_515_625.0);
        assert_eq!(pow_int(x, 0), 1.0);
    }

    #[test]
    fn typed_sort_puts_nan_last_and_negative_zero_first() {
        let mut values = [3.0, 0.0, -0.0, f64::NAN, 1.0];
        values.sort_by(|a, b| typed_order(*a, *b));
        assert!(same(values[0], -0.0) && same(values[1], 0.0));
        assert_eq!(&values[2..4], &[1.0, 3.0]);
        assert!(values[4].is_nan());
    }

    #[test]
    fn difference_sort_keeps_zeros_in_order() {
        let mut values = [0.0, 2.0, -0.0, 1.0];
        sort_by_difference(&mut values);
        assert!(same(values[0], 0.0) && same(values[1], -0.0));
    }

    #[test]
    fn percentile_is_nearest_rank() {
        let sorted = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&sorted, 0.01), 1.0);
        assert_eq!(percentile(&sorted, 0.5), 2.0);
        assert_eq!(percentile(&sorted, 0.51), 3.0);
        assert_eq!(percentile(&sorted, 0.99), 4.0);
        assert!(percentile::<f64>(&[], 0.5).is_nan());
        assert_eq!(percentile(&[1.5_f32], 0.95), 1.5);
    }

    #[test]
    fn median_of_odd_and_even_lists() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
        assert!(median(&[]).is_nan());
    }

    #[test]
    fn median_of_a_list_with_nan_is_javascriptcores() {
        let nan = f64::NAN;
        let values = [
            3.0, nan, 1.0, 2.0, nan, 0.5, 9.0, 4.0, nan, 7.0, 6.0, 5.0, 8.0, nan, 2.5, 1.5, 0.25,
            3.5, 4.5, 5.5, 6.5,
        ];
        assert_eq!(median(&values), 4.0);
    }

    #[test]
    fn strings_compare_by_code_unit() {
        // U+1F600 is above U+FF61 as a code point, below it in UTF-16 (its first unit is the surrogate D83D).
        assert_eq!(utf16_cmp("\u{1F600}", "\u{FF61}"), Ordering::Less);
        assert_eq!("\u{1F600}".cmp("\u{FF61}"), Ordering::Greater);
    }

    #[test]
    fn js_whitespace_set() {
        assert!(is_js_whitespace('\u{FEFF}'));
        assert!(is_js_whitespace('\u{3000}'));
        assert!(!is_js_whitespace('\u{0085}'));
        assert!(!is_js_whitespace('\u{200B}'));
    }
}
