//! The IR's storage grids: a value is rounded to a multiple of its quantum, and the result is the double nearest that
//! decimal.

use slidock_canon::{EXACT_STEPS, Grid, js_round, to_fixed};

/// `Number((Math.round(x / quantum) * quantum).toFixed(digits))`, with `-0` as `0`. NaN and infinities pass
/// through. For |x| below 10^15 steps this is `steps / 10^digits` ([`slidock_canon::grid_steps`]).
pub fn quantise(x: f64, grid: Grid) -> f64 {
    let k = js_round(x / grid.quantum());
    let rounded = if k.abs() <= EXACT_STEPS {
        k / grid.steps_per_unit() as f64
    } else {
        fixed_round_trip(k * grid.quantum(), grid.digits())
    };
    if rounded == 0.0 { 0.0 } else { rounded }
}

/// Whether `x` is on its grid: `quantise(x) === x` (never for NaN).
pub fn is_quantised(x: f64, grid: Grid) -> bool {
    quantise(x, grid) == x
}

/// `Number(x.toFixed(digits))`.
fn fixed_round_trip(x: f64, digits: u8) -> f64 {
    to_fixed(x, digits)
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(f64::NAN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn divides_by_the_binary_quantum() {
        assert_eq!(quantise(7.1995, Grid::Thousandths), 7.199);
        assert_eq!(quantise(2.675, Grid::Thousandths), 2.675);
        assert_eq!(quantise(1.0005, Grid::Thousandths), 1.0);
        assert_eq!(quantise(33.333333, Grid::HundredThousandths), 33.33333);
    }

    #[test]
    fn ties_go_up_and_zero_is_unsigned() {
        assert_eq!(quantise(0.0005, Grid::Thousandths), 0.001);
        assert_eq!(quantise(-0.0015, Grid::Thousandths), -0.001);
        assert_eq!(quantise(-0.0005, Grid::Thousandths).to_bits(), 0);
        assert_eq!(quantise(-0.0, Grid::HundredThousandths).to_bits(), 0);
    }

    #[test]
    fn passes_non_finite_values_through() {
        assert!(quantise(f64::NAN, Grid::Thousandths).is_nan());
        assert_eq!(quantise(f64::INFINITY, Grid::Thousandths), f64::INFINITY);
        assert_eq!(
            quantise(-1e308, Grid::HundredThousandths),
            f64::NEG_INFINITY
        );
        assert!(!is_quantised(f64::NAN, Grid::Thousandths));
    }

    #[test]
    fn large_values_take_the_to_fixed_path() {
        assert_eq!(quantise(1e20, Grid::Thousandths), 1e20);
        assert_eq!(
            quantise(123_456_789_012_345.67, Grid::Thousandths),
            123_456_789_012_345.67
        );
    }

    #[test]
    fn grid_membership() {
        assert!(is_quantised(7.2, Grid::Thousandths));
        assert!(!is_quantised(7.2001, Grid::Thousandths));
        assert!(is_quantised(7.2001, Grid::HundredThousandths));
    }
}
