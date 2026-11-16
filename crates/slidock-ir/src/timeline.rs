//! Build steps and transitions. An effect is an opaque `<format>:<native>` id and the canonical fallback every runtime
//! can play; the static render is always the fully built slide.
//!
//! Optional fields, left out when unset so an IR without them round-trips byte for byte, add the native effect in the
//! common vocabulary ([`crate::effects`]), a step's timing, a background target and a transition's easing.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::effects::{Easing, NativeStep, NativeTransition, StepTiming, check_kind};
use crate::formats::EffectId;
use crate::ids::ElementId;
use crate::units::Ms;
use crate::validate::{Validate, Validator};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub enum StepFallback {
    Appear,
    Dissolve,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub enum TransitionFallback {
    None,
    Dissolve,
    Push,
    Wipe,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepEffect {
    pub id: EffectId,
    pub fallback: StepFallback,
    /// The effect in the common vocabulary, which a player tries before `fallback`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub native: Option<Box<NativeStep>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionEffect {
    pub id: EffectId,
    pub fallback: TransitionFallback,
    /// The effect in the common vocabulary, which a player tries before `fallback`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub native: Option<Box<NativeTransition>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub enum Trigger {
    Click,
    WithPrevious,
    AfterPrevious,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub enum StepKind {
    Entrance,
    Exit,
    Emphasis,
    Motion,
    Media,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub enum Delivery {
    All,
    ByParagraph,
    ByWord,
    ByChar,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// What a step builds: an element, optionally a range of its paragraphs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepTarget {
    pub element: ElementId,
    /// First and last paragraph index, inclusive.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub paragraphs: Option<(u32, u32)>,
    /// The element's shape without its text (PPTX `animBg`): the step animates the box, the text stays.
    #[serde(default, skip_serializing_if = "is_false")]
    pub background: bool,
}

/// One build step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TimelineStep {
    pub target: StepTarget,
    pub trigger: Trigger,
    pub delay_ms: Ms,
    pub duration_ms: Ms,
    pub kind: StepKind,
    pub effect: StepEffect,
    pub delivery: Delivery,
    /// Timing beyond the delay and the duration.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub timing: Option<Box<StepTiming>>,
}

impl Validate for TimelineStep {
    fn validate(&self, v: &mut Validator) {
        v.field("delayMs", &self.delay_ms);
        v.field("durationMs", &self.duration_ms);
        if let Some(native) = &self.effect.native {
            v.at("effect", |v| {
                v.at("native", |v| {
                    native.validate(v);
                    check_kind(v, &native.effect, self.kind);
                });
            });
        }
        v.field("timing", &self.timing);
    }
}

/// How a slide advances: on a click, and after a delay.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Advance {
    pub on_click: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub after_ms: Option<Ms>,
}

/// The transition into a slide.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Transition {
    pub effect: TransitionEffect,
    pub duration_ms: Ms,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub advance: Option<Advance>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub easing: Option<Easing>,
}

impl Validate for Transition {
    fn validate(&self, v: &mut Validator) {
        v.at("effect", |v| v.field("native", &self.effect.native));
        v.field("durationMs", &self.duration_ms);
        if let Some(advance) = &self.advance {
            v.at("advance", |v| v.field("afterMs", &advance.after_ms));
        }
        v.field("easing", &self.easing);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn round_trips_steps_and_transitions() {
        let step = json!({
            "target": { "element": "ir-s1.title", "paragraphs": [0, 2] }, "trigger": "afterPrevious",
            "delayMs": 0, "durationMs": 500, "kind": "entrance",
            "effect": { "id": "pptx:fade", "fallback": "dissolve" }, "delivery": "byParagraph"
        });
        let parsed: TimelineStep = serde_json::from_value(step.clone()).unwrap();
        assert_eq!(serde_json::to_value(&parsed).unwrap(), step);
        let mut long = step.clone();
        long["durationMs"] = json!(3_600_001);
        let parsed: TimelineStep = serde_json::from_value(long).unwrap();
        assert_eq!(crate::validate::validate(&parsed).len(), 1);
        let mut bad = step;
        bad["effect"]["id"] = json!("figma:fade");
        assert!(serde_json::from_value::<TimelineStep>(bad).is_err());
        let transition = json!({ "effect": { "id": "keynote:Magic_Move", "fallback": "none" }, "durationMs": 1000,
            "advance": { "onClick": true, "afterMs": 2000 } });
        let parsed: Transition = serde_json::from_value(transition.clone()).unwrap();
        assert_eq!(serde_json::to_value(&parsed).unwrap(), transition);
    }
}
