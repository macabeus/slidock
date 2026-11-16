//! How a step or a transition runs in time beyond its delay and duration: easing, per-unit stagger, repeats,
//! auto-reverse, what happens after it, and its absolute start inside its click group.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::limits;
use super::params::HexColor;
use crate::diagnostics::codes;
use crate::units::{Ms, Ratio, UnitRatio};
use crate::validate::{Validate, Validator};

const ONE: UnitRatio = UnitRatio::from_steps(100_000);

/// A timing curve. The named curves are CSS's; `accelDecel` is SMIL's accelerate/decelerate (PPTX `accel`/`decel`,
/// fractions of the duration spent speeding up and slowing down, linear in between).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub enum Easing {
    #[serde(rename = "linear")]
    Linear,
    #[serde(rename = "in")]
    In,
    #[serde(rename = "out")]
    Out,
    #[serde(rename = "in-out")]
    InOut,
    /// CSS `cubic-bezier(x1, y1, x2, y2)`.
    #[serde(rename = "bezier")]
    Bezier(Ratio, Ratio, Ratio, Ratio),
    #[serde(rename = "accelDecel")]
    AccelDecel { accel: UnitRatio, decel: UnitRatio },
}

impl Validate for Easing {
    fn validate(&self, v: &mut Validator) {
        match self {
            Easing::Linear | Easing::In | Easing::Out | Easing::InOut => {}
            Easing::Bezier(x1, y1, x2, y2) => v.at("bezier", |v| {
                for (i, (x, y)) in [(x1, y1), (x2, y2)].into_iter().enumerate() {
                    v.at(2 * i, |v| {
                        v.min(x.to_f64(), 0.0, false);
                        v.max(x.to_f64(), 1.0);
                    });
                    v.at(2 * i + 1, |v| {
                        v.min(y.to_f64(), -limits::BEZIER_Y, false);
                        v.max(y.to_f64(), limits::BEZIER_Y);
                    });
                }
            }),
            Easing::AccelDecel { accel, decel } => v.at("accelDecel", |v| {
                v.field("accel", accel);
                v.field("decel", decel);
                if accel.steps() + decel.steps() > ONE.steps() {
                    v.invalid(
                        codes::IR0001,
                        "accel and decel add up to more than the duration",
                    );
                }
            }),
        }
    }
}

/// How much later each unit (paragraph, word or character) starts than the one before.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub enum StaggerAmount {
    /// Milliseconds (PPTX `tmAbs`, Keynote, ODP `iterate-interval`).
    #[serde(rename = "ms")]
    Ms(Ms),
    /// A fraction of the step's duration (PPTX `tmPct`).
    #[serde(rename = "fraction")]
    Fraction(UnitRatio),
}

/// The order units start in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub enum StaggerOrder {
    Forward,
    Backward,
    FromCenter,
    FromEdges,
    /// A shuffle from this seed, the same on every replay.
    Random(u32),
}

/// Per-unit start offsets of a step delivered by paragraph, word or character.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub struct Stagger {
    pub per_unit: StaggerAmount,
    pub order: StaggerOrder,
}

impl Validate for Stagger {
    fn validate(&self, v: &mut Validator) {
        v.at("perUnit", |v| match &self.per_unit {
            StaggerAmount::Ms(ms) => v.field("ms", ms),
            StaggerAmount::Fraction(fraction) => v.field("fraction", fraction),
        });
    }
}

/// How often a step plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub enum Repeat {
    /// This many times in all, possibly fractional (more than 0).
    Count(Ratio),
    UntilNextClick,
    UntilEndOfSlide,
}

impl Validate for Repeat {
    fn validate(&self, v: &mut Validator) {
        if let Repeat::Count(count) = self {
            v.at("count", |v| {
                v.min(count.to_f64(), 0.0, true);
                v.max(count.to_f64(), f64::from(limits::REPEAT_COUNT));
            });
        }
    }
}

/// What happens to the target after the step (PowerPoint's "after animation").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub enum AfterEffect {
    Keep,
    Hide,
    /// Its text takes this colour.
    Dim(HexColor),
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// A step's timing beyond its delay and duration. Every field is optional; an absent one plays the default
/// (start by trigger and delay, linear, all units at once, once, keep).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub struct StepTiming {
    /// The absolute start inside the step's click group, its delay included (PPTX computes it; a player uses it
    /// instead of chaining `afterPrevious` ends).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    #[ts(as = "Option<Ms>", optional)]
    pub offset_ms: Option<Ms>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    #[ts(as = "Option<Easing>", optional)]
    pub easing: Option<Easing>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    #[ts(as = "Option<Stagger>", optional)]
    pub stagger: Option<Stagger>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    #[ts(as = "Option<Repeat>", optional)]
    pub repeat: Option<Repeat>,
    /// Each repeat plays forward, then backward.
    #[serde(default, skip_serializing_if = "is_false")]
    #[ts(as = "Option<bool>", optional)]
    pub auto_reverse: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    #[ts(as = "Option<AfterEffect>", optional)]
    pub after: Option<AfterEffect>,
}

impl StepTiming {
    /// Whether every field is at its default, so the step needs no timing.
    pub fn is_default(&self) -> bool {
        *self == StepTiming::default()
    }
}

impl Validate for StepTiming {
    fn validate(&self, v: &mut Validator) {
        v.field("offsetMs", &self.offset_ms);
        v.field("easing", &self.easing);
        v.field("stagger", &self.stagger);
        v.field("repeat", &self.repeat);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::validate;
    use serde_json::json;

    fn ratio(x: f64) -> Ratio {
        Ratio::exact(x).unwrap()
    }

    #[test]
    fn easings_read_their_tokens() {
        for (easing, wire) in [
            (Easing::Linear, json!("linear")),
            (Easing::InOut, json!("in-out")),
            (
                Easing::Bezier(ratio(0.25), ratio(0.1), ratio(0.25), ratio(1.0)),
                json!({ "bezier": [0.25, 0.1, 0.25, 1.0] }),
            ),
            (
                Easing::AccelDecel {
                    accel: UnitRatio::exact(0.2).unwrap(),
                    decel: UnitRatio::exact(0.3).unwrap(),
                },
                json!({ "accelDecel": { "accel": 0.2, "decel": 0.3 } }),
            ),
        ] {
            assert_eq!(serde_json::to_value(easing).unwrap(), wire);
            assert_eq!(serde_json::from_value::<Easing>(wire).unwrap(), easing);
        }
        assert!(serde_json::from_value::<Easing>(json!("ease-in")).is_err());
        assert!(
            serde_json::from_value::<Easing>(
                json!({ "accelDecel": { "accel": 0.2, "decel": 0.3, "x": 1 } })
            )
            .is_err()
        );
    }

    #[test]
    fn easing_bounds() {
        let ok = Easing::Bezier(ratio(0.0), ratio(-2.0), ratio(1.0), ratio(3.0));
        assert!(validate(&ok).is_empty());
        let bad_x = Easing::Bezier(ratio(1.5), ratio(0.0), ratio(0.5), ratio(1.0));
        let issues = validate(&bad_x);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].path.to_string(), "bezier[0]");
        let bad_y = Easing::Bezier(ratio(0.5), ratio(0.0), ratio(0.5), ratio(11.0));
        assert_eq!(validate(&bad_y)[0].path.to_string(), "bezier[3]");
        let over = Easing::AccelDecel {
            accel: UnitRatio::exact(0.6).unwrap(),
            decel: UnitRatio::exact(0.5).unwrap(),
        };
        assert_eq!(validate(&over).len(), 1);
        let full = Easing::AccelDecel {
            accel: UnitRatio::exact(0.5).unwrap(),
            decel: UnitRatio::exact(0.5).unwrap(),
        };
        assert!(validate(&full).is_empty());
    }

    #[test]
    fn timing_round_trips_and_is_bounded() {
        let wire = json!({
            "offsetMs": 1500, "easing": "out",
            "stagger": { "perUnit": { "fraction": 0.1 }, "order": { "random": 7 } },
            "repeat": { "count": 2.5 }, "autoReverse": true, "after": { "dim": "#808080" }
        });
        let timing: StepTiming = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(&timing).unwrap(), wire);
        assert!(validate(&timing).is_empty());
        assert_eq!(
            serde_json::to_value(StepTiming::default()).unwrap(),
            json!({})
        );
        assert!(StepTiming::default().is_default());
        for (bad, path) in [
            (json!({ "offsetMs": 3_600_001 }), "offsetMs"),
            (json!({ "repeat": { "count": 0 } }), "repeat.count"),
            (json!({ "repeat": { "count": 1001 } }), "repeat.count"),
            (
                json!({ "stagger": { "perUnit": { "ms": 3_600_001 }, "order": "forward" } }),
                "stagger.perUnit.ms",
            ),
        ] {
            let timing: StepTiming = serde_json::from_value(bad).unwrap();
            let issues = validate(&timing);
            assert_eq!(issues.len(), 1, "{path}");
            assert_eq!(issues[0].path.to_string(), path);
        }
        for bad in [
            json!({ "offsetMs": null }),
            json!({ "after": "dim" }),
            json!({ "repeat": "forever" }),
            json!({ "stagger": { "perUnit": { "pct": 10 }, "order": "forward" } }),
            json!({ "delay": 1 }),
        ] {
            assert!(
                serde_json::from_value::<StepTiming>(bad.clone()).is_err(),
                "{bad}"
            );
        }
    }
}
