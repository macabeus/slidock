//! Units. Lengths and angles are stored in thousandths and ratios in hundred-thousandths, as fixed-point integers: a
//! value is on its grid by construction, `-0` cannot exist, and equality is exact. [`Pt::quantise`] (and its siblings)
//! round an `f64` with JS `Math.round(x / quantum)` semantics, so passes that compute in `f64` and quantise at the
//! boundary print the same numbers on every host.
//!
//! The newtypes guarantee the grid; ranges (±100 000, `Size ≥ 0`, `Angle < 360`, …) are bounds the verifier checks
//! ([`crate::validate`]) and reports as IR0008, as zod does. Each unit holds its schema's checks in zod's order
//! ([`Check`]): a narrowed unit keeps the outer range before its own, so a value beyond both fails twice.

use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use slidock_canon::Grid;

use crate::de::failed_checks;
use crate::diagnostics::codes;
use crate::parse::StructureIssue;
use crate::validate::IssueDetail;

/// A value that cannot be stored on its grid.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum UnitError {
    #[error("not a finite number: {0}")]
    NonFinite(f64),
    #[error("{value} is not a multiple of {quantum}")]
    OffGrid { value: f64, quantum: &'static str },
    #[error("{0} is too large for its grid")]
    TooLarge(f64),
}

/// One of the checks zod runs on a number, in its schema's order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Check {
    /// At least `limit`; more than it when `exclusive` (zod's `gt`).
    Min { limit: f64, exclusive: bool },
    /// At most `limit`; less than it when `exclusive` (zod's `lt`).
    Max { limit: f64, exclusive: bool },
    /// On the unit's grid (IR0007).
    Grid,
}

impl Check {
    pub const fn min(limit: f64) -> Check {
        Check::Min {
            limit,
            exclusive: false,
        }
    }

    pub const fn max(limit: f64) -> Check {
        Check::Max {
            limit,
            exclusive: false,
        }
    }

    /// Whether `x` passes, `on_grid` standing for the grid check.
    pub fn holds(self, x: f64, on_grid: bool) -> bool {
        match self {
            Check::Min { limit, exclusive } => x > limit || (!exclusive && x == limit),
            Check::Max { limit, exclusive } => x < limit || (!exclusive && x == limit),
            Check::Grid => on_grid,
        }
    }
}

/// The issues of the checks `x` fails, in order: IR0008 for a bound, IR0007 for the grid.
pub fn failed(x: f64, checks: &[Check], on_grid: bool, quantum: &str) -> Vec<StructureIssue> {
    checks
        .iter()
        .filter(|check| !check.holds(x, on_grid))
        .map(|check| match check {
            Check::Min { limit, .. } | Check::Max { limit, .. } => StructureIssue {
                code: codes::IR0008,
                detail: IssueDetail::Bound {
                    what: "number",
                    limit: slidock_canon::to_js_string(*limit),
                },
            },
            Check::Grid => StructureIssue {
                code: codes::IR0007,
                detail: IssueDetail::Reason(format!("Not a multiple of {quantum}")),
            },
        })
        .collect()
}

const OUTER: f64 = 100_000.0;

struct NumberVisitor;

impl Visitor<'_> for NumberVisitor {
    type Value = f64;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a number")
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<f64, E> {
        Ok(v)
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<f64, E> {
        Ok(v as f64)
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<f64, E> {
        Ok(v as f64)
    }
}

/// Reads any JSON number as an `f64`.
pub(crate) fn deserialize_number<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<f64, D::Error> {
    deserializer.deserialize_f64(NumberVisitor)
}

macro_rules! fixed_unit {
    (
        $(#[$meta:meta])*
        $name:ident, $grid:expr, $quantum:literal, checks: [$($check:expr),* $(,)?]
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, ts_rs::TS)]
        #[ts(export_to = "ir/", type = "number")]
        pub struct $name(i64);

        impl $name {
            pub const ZERO: $name = $name(0);
            pub const GRID: Grid = $grid;
            /// The checks of the unit's schema, in zod's order.
            pub const CHECKS: &'static [Check] = &[$($check),*];

            /// The value of `steps` grid steps.
            pub const fn from_steps(steps: i64) -> Self {
                $name(steps)
            }

            /// The value in grid steps.
            pub const fn steps(self) -> i64 {
                self.0
            }

            /// The value as a JS number: the double nearest the decimal.
            pub fn to_f64(self) -> f64 {
                self.0 as f64 / Self::GRID.steps_per_unit() as f64
            }

            /// Rounds `x` to the grid (`quantise(x, unit)`).
            pub fn quantise(x: f64) -> Result<Self, UnitError> {
                if !x.is_finite() {
                    return Err(UnitError::NonFinite(x));
                }
                slidock_canon::grid_steps(x, Self::GRID).map($name).map_err(|_| UnitError::TooLarge(x))
            }

            /// `x`, which must already be on the grid (IR0007 otherwise).
            pub fn exact(x: f64) -> Result<Self, UnitError> {
                if !x.is_finite() {
                    return Err(UnitError::NonFinite(x));
                }
                if !slidock_math::is_quantised(x, Self::GRID) {
                    return Err(UnitError::OffGrid { value: x, quantum: $quantum });
                }
                Self::quantise(x)
            }

            /// Whether the value passes every check of [`Self::CHECKS`].
            pub fn in_range(self) -> bool {
                let x = self.to_f64();
                Self::CHECKS.iter().all(|check| check.holds(x, true))
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_f64(self.to_f64())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let x = deserialize_number(deserializer)?;
                $name::exact(x).map_err(|error| match error {
                    UnitError::OffGrid { .. } => failed_checks(&failed(x, Self::CHECKS, false, $quantum)),
                    UnitError::TooLarge(_) => failed_checks(&failed(x, Self::CHECKS, true, $quantum)),
                    UnitError::NonFinite(_) => de::Error::custom("Invalid input: expected number"),
                })
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&slidock_canon::to_js_string(self.to_f64()))
            }
        }
    };
}

fixed_unit!(
    /// Points (1/72 in) in presentation space, y down, in thousandths.
    Pt, Grid::Thousandths, "0.001", checks: [Check::min(-OUTER), Check::max(OUTER), Check::Grid]
);
fixed_unit!(
    /// A non-negative length: sizes, widths, insets.
    Size, Grid::Thousandths, "0.001",
    checks: [Check::min(-OUTER), Check::max(OUTER), Check::Grid, Check::min(0.0)]
);
fixed_unit!(
    /// Degrees, clockwise, in thousandths.
    Deg, Grid::Thousandths, "0.001", checks: [Check::min(-OUTER), Check::max(OUTER), Check::Grid]
);
fixed_unit!(
    /// A rotation or an angle, normalised to [0, 360).
    Angle, Grid::Thousandths, "0.001",
    checks: [
        Check::min(-OUTER),
        Check::max(OUTER),
        Check::Grid,
        Check::min(0.0),
        Check::Max { limit: 360.0, exclusive: true },
    ]
);
fixed_unit!(
    /// A unitless number in hundred-thousandths: opacities, fractions, colour components, scales, crops.
    Ratio, Grid::HundredThousandths, "0.00001", checks: [Check::min(-OUTER), Check::max(OUTER), Check::Grid]
);
fixed_unit!(
    /// A ratio in [0, 1].
    UnitRatio, Grid::HundredThousandths, "0.00001",
    checks: [Check::min(-OUTER), Check::max(OUTER), Check::Grid, Check::min(0.0), Check::max(1.0)]
);
fixed_unit!(
    /// CSS pixels of the slide at its native size (1 px = 0.75 pt), in thousandths: motion paths and keyframe
    /// translations, which the player applies in slide pixels.
    Px, Grid::Thousandths, "0.001", checks: [Check::min(-2.0 * OUTER), Check::max(2.0 * OUTER), Check::Grid]
);
fixed_unit!(
    /// A custom path's own units (OOXML `path@w/h`, ODF `svg:viewBox`), which can be EMUs and so exceed the point
    /// range; on the point grid.
    PathUnit, Grid::Thousandths, "0.001", checks: [Check::min(-1e9), Check::max(1e9), Check::Grid]
);

macro_rules! widen {
    ($($from:ident => $to:ident),*) => {
        $(impl From<$from> for $to {
            fn from(value: $from) -> Self {
                $to::from_steps(value.steps())
            }
        })*
    };
}
widen!(Size => Pt, Angle => Deg, UnitRatio => Ratio);

/// Slide pixels per point: a [`Px`] is a CSS pixel of the slide at its native size, 1 px = 0.75 pt.
pub const PX_PER_PT: f64 = 4.0 / 3.0;

/// A length in points as slide pixels ([`Px`]), not yet quantised.
pub fn pt_to_px(pt: f64) -> f64 {
    pt * PX_PER_PT
}

/// Milliseconds, up to an hour.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    ts_rs::TS,
)]
#[serde(transparent)]
#[ts(export_to = "ir/", type = "number")]
pub struct Ms(pub u32);

impl Ms {
    pub const MAX: u32 = 3_600_000;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantises_without_binary_noise() {
        assert_eq!(Pt::quantise(0.1 + 0.2).unwrap().to_f64(), 0.3);
        assert_eq!(Pt::quantise(12.34567).unwrap().to_f64(), 12.346);
        assert_eq!(Ratio::quantise(0.123456789).unwrap().to_f64(), 0.12346);
        assert_eq!(Deg::quantise(359.9996).unwrap().to_f64(), 360.0);
        assert_eq!(Pt::quantise(-0.0001).unwrap().to_f64().to_bits(), 0);
        assert_eq!(Pt::quantise(7.1995).unwrap().steps(), 7199);
    }

    #[test]
    fn quantise_is_idempotent() {
        for x in [
            1.0 / 3.0,
            -2.0 / 7.0,
            1e-4,
            99_999.999_5,
            12_700.0 / 12_700.0,
        ] {
            let once = Pt::quantise(x).unwrap();
            assert_eq!(Pt::exact(once.to_f64()).unwrap(), once);
        }
    }

    #[test]
    fn reads_only_quantised_values() {
        assert_eq!(
            serde_json::from_str::<Pt>("10.125").unwrap().steps(),
            10_125
        );
        assert!(serde_json::from_str::<Pt>("10.1255").is_err());
        assert!(serde_json::from_str::<Ratio>("0.123456").is_err());
        assert_eq!(
            serde_json::from_str::<Ratio>("0.12345").unwrap().steps(),
            12_345
        );
        assert!(matches!(
            Pt::exact(f64::INFINITY),
            Err(UnitError::NonFinite(_))
        ));
        assert!(matches!(Pt::exact(f64::NAN), Err(UnitError::NonFinite(_))));
    }

    #[test]
    fn prints_js_numbers() {
        assert_eq!(
            serde_json::to_string(&Pt::from_steps(7_200)).unwrap(),
            "7.2"
        );
        assert_eq!(Pt::from_steps(72_000).to_string(), "72");
        assert_eq!(Ratio::from_steps(-5).to_string(), "-0.00005");
    }

    #[test]
    fn ranges() {
        assert!(!Pt::exact(100_001.0).unwrap().in_range());
        assert!(!UnitRatio::exact(1.5).unwrap().in_range());
        assert!(Angle::exact(0.0).unwrap().in_range());
        assert!(Angle::exact(359.999).unwrap().in_range());
        assert!(!Angle::exact(360.0).unwrap().in_range());
        assert!(!Angle::exact(-90.0).unwrap().in_range());
        assert!(!Size::exact(-1.0).unwrap().in_range());
    }

    #[test]
    fn a_point_is_four_thirds_of_a_slide_pixel() {
        assert_eq!(pt_to_px(720.0), 960.0);
        assert_eq!(pt_to_px(540.0), 720.0);
        assert_eq!(pt_to_px(0.75), 1.0);
        assert_eq!(pt_to_px(-135.0), -180.0);
        assert_eq!(Px::quantise(pt_to_px(37.125)).unwrap().to_f64(), 49.5);
        // The scale multiplies by the rounded 4/3, as the HTML printer's clip paths do: 1280 pt is not the
        // correctly rounded 1280 × 4 / 3.
        assert_eq!(pt_to_px(1280.0), 1280.0 * PX_PER_PT);
        assert_ne!(pt_to_px(1280.0), 1280.0 * 4.0 / 3.0);
    }
}
