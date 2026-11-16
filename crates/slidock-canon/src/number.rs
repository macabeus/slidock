//! ECMAScript number semantics: `Number::toString`, `Math.round`, `Number.prototype.toFixed`, and the fixed-grid
//! printers the HTML backend uses for CSS values.

use crate::error::NumberError;

/// `String(n)`: the shortest decimal that round-trips, in ECMAScript's notation (`1e+21`, `1e-7`, `NaN`,
/// `-Infinity`; `-0` prints `0`).
pub fn to_js_string(n: f64) -> String {
    ryu_js::Buffer::new().format(n).to_owned()
}

/// The canonical form of a number in IR JSON, manifests and reports: `String(n)`, refusing non-finite values and
/// values that need e-notation (|n| ≥ 1e21 or 0 < |n| < 1e-6).
pub fn format_number(n: f64) -> Result<String, NumberError> {
    if !n.is_finite() {
        return Err(NumberError::NonFinite(n));
    }
    let text = to_js_string(n);
    if text.contains('e') {
        return Err(NumberError::Exponent(text));
    }
    Ok(text)
}

/// `Math.round`: the nearest integer, ties toward +∞ (`-2.5` → `-2`, `2.5` → `3`). It keeps the sign of zero
/// (`Math.round(-0.4)` is `-0`) and passes NaN and infinities through. `f64::round` differs: it ties away from zero.
pub fn js_round(v: f64) -> f64 {
    let up = v.ceil();
    if up - 0.5 > v { up - 1.0 } else { up }
}

/// The most fraction digits `Number.prototype.toFixed` accepts.
pub const MAX_FIXED_DIGITS: u8 = 100;

/// `x.toFixed(digits)`: the exact value of `x` rounded half up to `digits` decimals, `String(x)` for |x| ≥ 1e21,
/// and `NaN`/`Infinity` for non-finite values. `-0` prints unsigned; a negative value that rounds to zero keeps
/// its sign (`(-0.001).toFixed(2)` is `-0.00`).
pub fn to_fixed(x: f64, digits: u8) -> Result<String, NumberError> {
    if digits > MAX_FIXED_DIGITS {
        return Err(NumberError::FixedDigits(digits));
    }
    if !x.is_finite() || x.abs() >= 1e21 {
        return Ok(to_js_string(x));
    }
    Ok(crate::fixed::to_fixed_finite(x, digits))
}

/// A decimal storage grid: the IR stores lengths and angles in thousandths, ratios in hundred-thousandths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Grid {
    /// 0.001: points and degrees.
    Thousandths,
    /// 0.00001: ratios.
    HundredThousandths,
}

impl Grid {
    /// The grid step as JavaScript writes it (`0.001`, `0.00001`). Quantising divides by this value.
    pub const fn quantum(self) -> f64 {
        match self {
            Grid::Thousandths => 0.001,
            Grid::HundredThousandths => 0.00001,
        }
    }

    /// Decimals on the grid.
    pub const fn digits(self) -> u8 {
        match self {
            Grid::Thousandths => 3,
            Grid::HundredThousandths => 5,
        }
    }

    /// Grid steps per unit (`10^digits`).
    pub const fn steps_per_unit(self) -> i64 {
        match self {
            Grid::Thousandths => 1_000,
            Grid::HundredThousandths => 100_000,
        }
    }
}

/// The largest step count handled exactly: below 10^15 steps the decimal `k / 10^digits` has at most 15
/// significant digits, so it is the shortest form of its double, and `k · quantum` is within half a step of it
/// (that holds up to 2^51), so `toFixed` recovers it.
pub const EXACT_STEPS: f64 = 999_999_999_999_999.0;

/// The grid step count of `x`: `Math.round(x / quantum)`. The division by the binary quantum is deliberate:
/// `7.1995 / 0.001` is `7199.499999999999`, so `7.1995` lands on `7.199`. Fails on non-finite input or when
/// |steps| exceeds [`EXACT_STEPS`].
pub fn grid_steps(x: f64, grid: Grid) -> Result<i64, NumberError> {
    if !x.is_finite() {
        return Err(NumberError::NonFinite(x));
    }
    let k = js_round(x / grid.quantum());
    if k.abs() > EXACT_STEPS {
        return Err(NumberError::OutOfRange(x));
    }
    // |k| < 10^15 and integral, so the conversion is exact.
    Ok(k as i64)
}

/// `formatNumber(quantise(x))` with the grid's decimals: at most `digits` decimals, trailing zeros trimmed, `0`
/// never `-0`, never an exponent (`7.199`, `-12.5`, `0`): CSS lengths in thousandths and ratios in hundred-thousandths.
pub fn format_grid(x: f64, grid: Grid) -> Result<String, NumberError> {
    Ok(format_steps(grid_steps(x, grid)?, grid))
}

/// Prints `steps / 10^digits` as [`format_grid`] does.
pub fn format_steps(steps: i64, grid: Grid) -> String {
    let scale = grid.steps_per_unit().unsigned_abs();
    let magnitude = steps.unsigned_abs();
    let (int, frac) = (magnitude / scale, magnitude % scale);
    let mut out = String::new();
    if steps < 0 {
        out.push('-');
    }
    out.push_str(&int.to_string());
    if frac != 0 {
        let width = usize::from(grid.digits());
        let digits = format!("{frac:0width$}");
        out.push('.');
        out.push_str(digits.trim_end_matches('0'));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_round_ties_toward_positive_infinity() {
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(js_round(-0.5).to_bits(), (-0.0f64).to_bits());
        assert_eq!(js_round(0.49999999999999994), 0.0);
        assert_eq!(js_round(-0.4).to_bits(), (-0.0f64).to_bits());
        assert!(js_round(f64::NAN).is_nan());
        assert_eq!(js_round(f64::NEG_INFINITY), f64::NEG_INFINITY);
    }

    #[test]
    fn format_number_refuses_exponents_and_non_finite() {
        assert_eq!(format_number(-0.0).as_deref(), Ok("0"));
        assert_eq!(format_number(7.2).as_deref(), Ok("7.2"));
        assert_eq!(
            format_number(1e21),
            Err(NumberError::Exponent("1e+21".into()))
        );
        assert_eq!(
            format_number(1e-7),
            Err(NumberError::Exponent("1e-7".into()))
        );
        assert!(matches!(
            format_number(f64::NAN),
            Err(NumberError::NonFinite(_))
        ));
    }

    /// Values at the edges of the grids' rounding, printed on the 0.001 and the 0.00001 grids.
    #[test]
    fn grid_printing_matches_the_reference_table() {
        let cases: [(f64, &str, &str); 12] = [
            (7.1994, "7.199", "7.1994"),
            (7.1995, "7.199", "7.1995"),
            (7.1996, "7.2", "7.1996"),
            (2.675, "2.675", "2.675"),
            (1.0005, "1", "1.0005"),
            (0.0005, "0.001", "0.0005"),
            (-0.0005, "0", "-0.0005"),
            (-0.0015, "-0.001", "-0.0015"),
            (0.1 + 0.2, "0.3", "0.3"),
            (1e-7, "0", "0"),
            (33.333333, "33.333", "33.33333"),
            (-0.0, "0", "0"),
        ];
        for (x, fixed, ratio) in cases {
            assert_eq!(
                format_grid(x, Grid::Thousandths).as_deref(),
                Ok(fixed),
                "fixed {x}"
            );
            assert_eq!(
                format_grid(x, Grid::HundredThousandths).as_deref(),
                Ok(ratio),
                "ratio {x}"
            );
        }
    }

    #[test]
    fn grid_steps_refuse_unbounded_values() {
        assert!(matches!(
            grid_steps(f64::INFINITY, Grid::Thousandths),
            Err(NumberError::NonFinite(_))
        ));
        assert!(matches!(
            grid_steps(1e13, Grid::Thousandths),
            Err(NumberError::OutOfRange(_))
        ));
        assert_eq!(grid_steps(1e11, Grid::Thousandths), Ok(100_000_000_000_000));
        assert!(matches!(
            grid_steps(1e12, Grid::Thousandths),
            Err(NumberError::OutOfRange(_))
        ));
    }

    #[test]
    fn to_fixed_refuses_too_many_digits() {
        assert_eq!(to_fixed(1.0, 101), Err(NumberError::FixedDigits(101)));
        assert_eq!(to_fixed(1.005, 2).as_deref(), Ok("1.00"));
    }
}
