//! Numbers as the lowering computes them: JavaScript's `Math.min`/`Math.max` (NaN wins), `Math.round`, and the IR's
//! grids. The file's floats are widened to `f64` before any arithmetic. A value the IR cannot hold (NaN, which a
//! clamp lets through) is stored as 0.

use slidock_canon::js_round;
pub use slidock_geometry::{js_max, js_min};
use slidock_ir::units::{Angle, Deg, Ms, PathUnit, Pt, Ratio, Size, UnitError, UnitRatio};

/// `Math.min(hi, Math.max(lo, x))`.
pub fn clamp(x: f64, lo: f64, hi: f64) -> f64 {
    js_min(hi, js_max(lo, x))
}

/// `x` when it is a finite number, else `fallback` (`Number.isFinite(x) ? x : fallback`).
pub fn finite_or(x: Option<f64>, fallback: f64) -> f64 {
    x.filter(|x| x.is_finite()).unwrap_or(fallback)
}

/// A file's float, widened.
pub fn wide(x: Option<f32>) -> Option<f64> {
    x.map(f64::from)
}

fn on_grid<T: Copy>(x: f64, quantise: fn(f64) -> Result<T, UnitError>, zero: T) -> T {
    quantise(x).unwrap_or(zero)
}

pub fn pt(x: f64) -> Pt {
    on_grid(x, Pt::quantise, Pt::ZERO)
}

pub fn size(x: f64) -> Size {
    on_grid(x, Size::quantise, Size::ZERO)
}

pub fn ratio(x: f64) -> Ratio {
    on_grid(x, Ratio::quantise, Ratio::ZERO)
}

pub fn unit_ratio(x: f64) -> UnitRatio {
    on_grid(x, UnitRatio::quantise, UnitRatio::ZERO)
}

pub fn angle(x: f64) -> Angle {
    on_grid(x, Angle::quantise, Angle::ZERO)
}

pub fn deg(x: f64) -> Deg {
    on_grid(x, Deg::quantise, Deg::ZERO)
}

pub fn path_unit(x: f64) -> PathUnit {
    on_grid(x, PathUnit::quantise, PathUnit::ZERO)
}

/// The longest time a step or transition may take.
const MAX_MS: f64 = 3_600_000.0;

/// Seconds as whole milliseconds, within an hour; a value that is not finite is 0.
pub fn ms(seconds: Option<f64>) -> Ms {
    let x = js_round(clamp(finite_or(seconds, 0.0) * 1000.0, 0.0, MAX_MS));
    // In [0, 3 600 000] by the clamp.
    Ms(x as u32)
}

/// An angle in degrees, normalised to [0, 360) on the degree grid.
pub fn normalised_angle(degrees: f64) -> Angle {
    let angle = slidock_math::quantise(((degrees % 360.0) + 360.0) % 360.0, Deg::GRID);
    self::angle(if angle >= 360.0 { 0.0 } else { angle })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_seconds_to_milliseconds_within_an_hour() {
        assert_eq!(ms(Some(1.25)), Ms(1250));
        assert_eq!(ms(Some(0.0005)), Ms(1));
        assert_eq!(ms(Some(-2.0)), Ms(0));
        assert_eq!(ms(Some(1e9)), Ms(3_600_000));
        assert_eq!(ms(Some(f64::NAN)), Ms(0));
        assert_eq!(ms(None), Ms(0));
    }

    #[test]
    fn normalises_angles_on_the_degree_grid() {
        assert_eq!(normalised_angle(-30.0).to_f64(), 330.0);
        assert_eq!(normalised_angle(720.0).to_f64(), 0.0);
        assert_eq!(normalised_angle(-0.0).to_f64(), 0.0);
        assert_eq!(normalised_angle(359.9999).to_f64(), 0.0);
        assert_eq!(normalised_angle(f64::NAN).to_f64(), 0.0);
    }

    #[test]
    fn stores_what_the_ir_cannot_hold_as_zero() {
        assert_eq!(pt(clamp(f64::NAN, -1e5, 1e5)), Pt::ZERO);
        assert_eq!(pt(clamp(f64::INFINITY, -1e5, 1e5)).to_f64(), 100_000.0);
        assert_eq!(size(12.3456).to_f64(), 12.346);
    }
}
