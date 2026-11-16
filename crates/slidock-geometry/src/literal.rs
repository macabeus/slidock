//! Number tokens in guide formulas and path commands. `build.rs` includes this file, so it depends on nothing.

/// `^-?\d+(\.\d+)?$` (ASCII digits), as a number.
pub fn parse_literal(token: &str) -> Option<f64> {
    let digits = token.strip_prefix('-').unwrap_or(token);
    let (int, frac) = match digits.split_once('.') {
        Some((int, frac)) => (int, Some(frac)),
        None => (digits, None),
    };
    let all_digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !all_digits(int) || frac.is_some_and(|frac| !all_digits(frac)) {
        return None;
    }
    token.parse().ok()
}
