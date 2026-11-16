//! The conversions from OOXML's units to the IR's, which happen here, once, at the boundary: EMUs and hundredths of a
//! point to points, thousandths of a percent to ratios, 60000ths of a degree to angles, each clamped to the IR's range
//! and put on its grid.

use slidock_canon::EXACT_STEPS;
use slidock_ir::units::{Angle, Pt, Ratio, Size, UnitRatio};

/// The IR's coordinate range, in points.
const PT_LIMIT: f64 = 100_000.0;
pub(crate) const EMU_PER_PT: f64 = 12_700.0;

fn clamp(x: f64, min: f64, max: f64) -> f64 {
    if x < min {
        min
    } else if x > max {
        max
    } else {
        x
    }
}

/// The grid steps of `x`, saturating at the largest exact step count (a hostile value the verifier then reports).
fn steps(x: f64, grid: slidock_canon::Grid) -> i64 {
    slidock_canon::grid_steps(x, grid).unwrap_or(if x.is_nan() {
        0
    } else if x < 0.0 {
        -(EXACT_STEPS as i64)
    } else {
        EXACT_STEPS as i64
    })
}

/// `quantise(x, 'pt')`.
pub(crate) fn pt(x: f64) -> Pt {
    Pt::from_steps(steps(x, Pt::GRID))
}

/// `quantise(x, 'ratio')`.
pub(crate) fn ratio(x: f64) -> Ratio {
    Ratio::from_steps(steps(x, Ratio::GRID))
}

/// `quantise(x, 'ratio')` as a ratio in [0, 1] (the caller clamps).
pub(crate) fn unit_ratio(x: f64) -> UnitRatio {
    UnitRatio::from_steps(steps(x, UnitRatio::GRID))
}

/// `quantise(x, 'deg')` as a normalised angle (the caller normalises).
pub(crate) fn angle_of(x: f64) -> Angle {
    Angle::from_steps(steps(x, Angle::GRID))
}

/// English Metric Units to points, on the IR's grid and within its range.
pub(crate) fn emu_to_pt(emu: i64) -> Pt {
    pt(clamp(emu as f64 / EMU_PER_PT, -PT_LIMIT, PT_LIMIT))
}

/// A length, clamped at 0.
pub(crate) fn emu_to_size(emu: i64) -> Size {
    Size::from_steps(emu_to_pt(emu.max(0)).steps())
}

/// Hundredths of a point (font sizes, spacing in points).
pub(crate) fn centi_pt(value: i64) -> Pt {
    pt(clamp(value as f64 / 100.0, -PT_LIMIT, PT_LIMIT))
}

/// [`centi_pt`] as a length, for a value the caller keeps at or above 0.
pub(crate) fn centi_size(value: i64) -> Size {
    Size::from_steps(centi_pt(value).steps())
}

/// ST_Percentage and its kin: thousandths of a percent, so 100000 is 1.
pub(crate) fn percent(value: i64) -> Ratio {
    ratio(clamp(value as f64 / 100_000.0, -PT_LIMIT, PT_LIMIT))
}

/// [`percent`] as a unit ratio, for a value the caller keeps within [0, 100000].
pub(crate) fn percent_unit(value: i64) -> UnitRatio {
    UnitRatio::from_steps(percent(value).steps())
}

/// ST_Angle: 60000ths of a degree, clockwise, normalised to [0, 360).
pub(crate) fn angle(value: i64) -> Angle {
    let degrees = ((value as f64 / 60_000.0) % 360.0 + 360.0) % 360.0;
    let quantised = angle_of(degrees);
    if quantised.to_f64() >= 360.0 {
        Angle::ZERO
    } else {
        quantised
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_at_the_boundary_on_the_grid() {
        assert_eq!(emu_to_pt(12_700).to_f64(), 1.0);
        assert_eq!(emu_to_pt(-9_144_000).to_f64(), -720.0);
        assert_eq!(emu_to_pt(i64::MAX / 4).to_f64(), 100_000.0);
        assert_eq!(emu_to_size(-5).to_f64(), 0.0);
        assert_eq!(emu_to_pt(1).to_f64(), 0.0);
        assert_eq!(centi_pt(1050).to_f64(), 10.5);
        assert_eq!(percent(90_000).to_f64(), 0.9);
        assert_eq!(angle(5_400_000).to_f64(), 90.0);
        assert_eq!(angle(-5_400_000).to_f64(), 270.0);
        assert_eq!(angle(21_600_000).to_f64(), 0.0);
        // Rounds up to a whole turn, which is 0.
        assert_eq!(angle(21_599_999).to_f64(), 0.0);
    }

    #[test]
    fn saturates_a_value_past_the_exact_steps() {
        assert_eq!(ratio(1e20).steps(), EXACT_STEPS as i64);
        assert_eq!(ratio(-1e20).steps(), -(EXACT_STEPS as i64));
        assert_eq!(ratio(f64::NAN).steps(), 0);
    }
}
