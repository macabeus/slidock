//! Motion fixtures without a frontend: builders for build steps and transitions in the common vocabulary, and a
//! source presentation of plain text boxes that carries them. Tests use them for expected values; written as JSON,
//! the presentation compiles with the IR frontend (`slidock compile deck.presentation-ir.json --out deck/`) into a
//! deck a player can play.
//!
//! Defaults: a step is click-triggered, starts at once, lasts 500 ms, delivers all at once and falls back to its
//! effect's fallback for its kind; a transition lasts 500 ms and advances on a click. Both name their effect
//! `ir:<effect name>` as their native id.

use serde_json::{Value, json};

use crate::effects::{
    AfterEffect, BuildEffect, Easing, NativeId, NativeStep, NativeTransition, Repeat, Stagger,
    StepTiming,
};
use crate::formats::SourceFormat;
use crate::ids::{ElementId, SlideId};
use crate::parse::{StructureError, from_value};
use crate::source::Presentation;
use crate::timeline::{
    Advance, Delivery, StepEffect, StepFallback, StepKind, StepTarget, TimelineStep, Transition,
    TransitionEffect, TransitionFallback, Trigger,
};
use crate::units::Ms;

/// The default duration of a fixture step or transition.
pub const DURATION_MS: u32 = 500;

/// A build step under construction.
#[derive(Clone, Debug, PartialEq)]
pub struct StepBuilder(TimelineStep);

impl StepBuilder {
    /// A step of `kind` that plays `effect` on `element`.
    pub fn new(element: ElementId, kind: StepKind, effect: BuildEffect) -> StepBuilder {
        let id = NativeId::new(SourceFormat::Ir, effect.name());
        StepBuilder(TimelineStep {
            target: StepTarget {
                element,
                paragraphs: None,
                background: false,
            },
            trigger: Trigger::Click,
            delay_ms: Ms(0),
            duration_ms: Ms(DURATION_MS),
            kind,
            effect: StepEffect {
                id: id.effect_id(),
                fallback: effect.fallback(kind),
                native: Some(Box::new(NativeStep { id, effect })),
            },
            delivery: Delivery::All,
            timing: None,
        })
    }

    pub fn entrance(element: ElementId, effect: BuildEffect) -> StepBuilder {
        StepBuilder::new(element, StepKind::Entrance, effect)
    }

    pub fn exit(element: ElementId, effect: BuildEffect) -> StepBuilder {
        StepBuilder::new(element, StepKind::Exit, effect)
    }

    pub fn emphasis(element: ElementId, effect: BuildEffect) -> StepBuilder {
        StepBuilder::new(element, StepKind::Emphasis, effect)
    }

    /// A step with only an opaque id and its fallback, as the frontends lower an effect they do not map.
    pub fn opaque(
        element: ElementId,
        kind: StepKind,
        id: NativeId,
        fallback: StepFallback,
    ) -> StepBuilder {
        let mut step = StepBuilder::new(element, kind, BuildEffect::Appear);
        step.0.effect = StepEffect {
            id: id.effect_id(),
            fallback,
            native: None,
        };
        step
    }

    pub fn trigger(mut self, trigger: Trigger) -> StepBuilder {
        self.0.trigger = trigger;
        self
    }

    pub fn delay(mut self, ms: u32) -> StepBuilder {
        self.0.delay_ms = Ms(ms);
        self
    }

    pub fn duration(mut self, ms: u32) -> StepBuilder {
        self.0.duration_ms = Ms(ms);
        self
    }

    /// Builds paragraphs `first..=last` of the target.
    pub fn paragraphs(mut self, first: u32, last: u32) -> StepBuilder {
        self.0.target.paragraphs = Some((first, last));
        self
    }

    pub fn delivery(mut self, delivery: Delivery) -> StepBuilder {
        self.0.delivery = delivery;
        self
    }

    /// Builds the target's shape without its text.
    pub fn background(mut self) -> StepBuilder {
        self.0.target.background = true;
        self
    }

    /// Names the native effect `id` (`pptx:entr.2/4`).
    pub fn native_id(mut self, id: NativeId) -> StepBuilder {
        self.0.effect.id = id.effect_id();
        if let Some(native) = &mut self.0.effect.native {
            native.id = id;
        }
        self
    }

    pub fn fallback(mut self, fallback: StepFallback) -> StepBuilder {
        self.0.effect.fallback = fallback;
        self
    }

    fn timing_mut(&mut self) -> &mut StepTiming {
        self.0.timing.get_or_insert_with(Box::default)
    }

    pub fn offset(mut self, ms: u32) -> StepBuilder {
        self.timing_mut().offset_ms = Some(Ms(ms));
        self
    }

    pub fn easing(mut self, easing: Easing) -> StepBuilder {
        self.timing_mut().easing = Some(easing);
        self
    }

    pub fn stagger(mut self, stagger: Stagger) -> StepBuilder {
        self.timing_mut().stagger = Some(stagger);
        self
    }

    pub fn repeat(mut self, repeat: Repeat) -> StepBuilder {
        self.timing_mut().repeat = Some(repeat);
        self
    }

    pub fn auto_reverse(mut self) -> StepBuilder {
        self.timing_mut().auto_reverse = true;
        self
    }

    pub fn after(mut self, after: AfterEffect) -> StepBuilder {
        self.timing_mut().after = Some(after);
        self
    }

    pub fn build(self) -> TimelineStep {
        self.0
    }
}

impl From<StepBuilder> for TimelineStep {
    fn from(step: StepBuilder) -> TimelineStep {
        step.0
    }
}

/// A transition under construction.
#[derive(Clone, Debug, PartialEq)]
pub struct TransitionBuilder(Transition);

impl TransitionBuilder {
    /// A transition that plays `effect`.
    pub fn new(effect: crate::effects::TransitionEffect) -> TransitionBuilder {
        let id = NativeId::new(SourceFormat::Ir, effect.name());
        TransitionBuilder(Transition {
            effect: TransitionEffect {
                id: id.effect_id(),
                fallback: effect.fallback(),
                native: Some(Box::new(NativeTransition { id, effect })),
            },
            duration_ms: Ms(DURATION_MS),
            advance: Some(Advance {
                on_click: true,
                after_ms: None,
            }),
            easing: None,
        })
    }

    /// A transition with only an opaque id and its fallback.
    pub fn opaque(id: NativeId, fallback: TransitionFallback) -> TransitionBuilder {
        let mut into = TransitionBuilder::new(crate::effects::TransitionEffect::None);
        into.0.effect = TransitionEffect {
            id: id.effect_id(),
            fallback,
            native: None,
        };
        into
    }

    pub fn duration(mut self, ms: u32) -> TransitionBuilder {
        self.0.duration_ms = Ms(ms);
        self
    }

    /// Advances on a click when `on_click`, and after `after_ms` when given.
    pub fn advance(mut self, on_click: bool, after_ms: Option<u32>) -> TransitionBuilder {
        self.0.advance = Some(Advance {
            on_click,
            after_ms: after_ms.map(Ms),
        });
        self
    }

    pub fn easing(mut self, easing: Easing) -> TransitionBuilder {
        self.0.easing = Some(easing);
        self
    }

    /// Names the native effect `id` (`pptx:push`).
    pub fn native_id(mut self, id: NativeId) -> TransitionBuilder {
        self.0.effect.id = id.effect_id();
        if let Some(native) = &mut self.0.effect.native {
            native.id = id;
        }
        self
    }

    pub fn fallback(mut self, fallback: TransitionFallback) -> TransitionBuilder {
        self.0.effect.fallback = fallback;
        self
    }

    pub fn build(self) -> Transition {
        self.0
    }
}

impl From<TransitionBuilder> for Transition {
    fn from(into: TransitionBuilder) -> Transition {
        into.0
    }
}

/// A text box: a rectangle at `x, y` of `w × h` points, one paragraph per string, black 24 pt text.
#[derive(Clone, Debug, PartialEq)]
pub struct TextBox {
    pub id: ElementId,
    pub frame: [f64; 4],
    pub paragraphs: Vec<String>,
    /// A solid sRGB fill as `[r, g, b]`, each 0 to 1, or none.
    pub fill: Option<[f64; 3]>,
    pub match_key: Option<String>,
}

impl TextBox {
    pub fn new(id: ElementId, frame: [f64; 4], paragraphs: &[&str]) -> TextBox {
        TextBox {
            id,
            frame,
            paragraphs: paragraphs.iter().map(|text| (*text).to_owned()).collect(),
            fill: None,
            match_key: None,
        }
    }

    pub fn fill(mut self, rgb: [f64; 3]) -> TextBox {
        self.fill = Some(rgb);
        self
    }

    pub fn match_key(mut self, key: &str) -> TextBox {
        self.match_key = Some(key.to_owned());
        self
    }
}

/// A slide of text boxes with its build steps and the transition into it.
#[derive(Clone, Debug, PartialEq)]
pub struct Slide {
    pub id: SlideId,
    pub boxes: Vec<TextBox>,
    pub timeline: Vec<TimelineStep>,
    pub transition: Option<Transition>,
}

impl Slide {
    pub fn new(id: SlideId, boxes: Vec<TextBox>) -> Slide {
        Slide {
            id,
            boxes,
            timeline: Vec::new(),
            transition: None,
        }
    }

    pub fn step(mut self, step: impl Into<TimelineStep>) -> Slide {
        self.timeline.push(step.into());
        self
    }

    pub fn transition(mut self, transition: impl Into<Transition>) -> Slide {
        self.transition = Some(transition.into());
        self
    }
}

fn rgb(rgb: [f64; 3]) -> Value {
    let [r, g, b] = rgb;
    json!({ "kind": "rgb", "space": "srgb", "r": r, "g": g, "b": b, "a": 1 })
}

fn text_box(element: &TextBox) -> Value {
    let [x, y, w, h] = element.frame;
    let paragraphs: Vec<Value> = element
        .paragraphs
        .iter()
        .map(|text| {
            json!({ "level": 0, "props": {}, "inlines": [{ "kind": "run", "text": text,
                "props": { "size": 24, "color": rgb([0.0, 0.0, 0.0]) } }] })
        })
        .collect();
    let mut shape = json!({
        "id": element.id.as_str(), "origin": { "kind": "agent" }, "kind": "shape",
        "frame": { "x": x, "y": y, "w": w, "h": h, "rotation": 0, "flipH": false, "flipV": false },
        "geometry": { "kind": "preset", "name": "rect", "adjust": {} },
        "text": { "body": {}, "paragraphs": paragraphs }
    });
    if let Some(object) = shape.as_object_mut() {
        if let Some(fill) = element.fill {
            object.insert(
                "fill".to_owned(),
                json!({ "kind": "solid", "color": rgb(fill) }),
            );
        }
        if let Some(key) = &element.match_key {
            object.insert("matchKey".to_owned(), Value::from(key.as_str()));
        }
    }
    shape
}

/// The source presentation of `slides` as IR JSON, 720 × 405 pt, with the defaults the IR frontend needs.
pub fn presentation_json(slides: &[Slide]) -> Result<Value, serde_json::Error> {
    let level = json!({
        "para": { "align": "left", "bullet": { "kind": "none" }, "indent": 0,
                  "lineSpacing": { "basis": "size-1.2", "kind": "multiple", "value": 1 }, "marginLeft": 0,
                  "marginRight": 0, "rtl": false, "spaceAfter": { "kind": "pt", "value": 0 },
                  "spaceBefore": { "kind": "pt", "value": 0 }, "tabSize": 72 },
        "run": { "baseline": 0, "caps": "none", "color": { "kind": "theme", "mods": [], "slot": "tx1" },
                 "font": { "kind": "theme", "slot": "minor" }, "highlight": null, "italic": false,
                 "letterSpacing": 0, "link": null, "scriptScale": 1, "size": 18, "strike": "none",
                 "synthesizeBold": true, "underline": "none", "weight": 400 }
    });
    let text: serde_json::Map<String, Value> =
        (0..9).map(|i| (i.to_string(), level.clone())).collect();
    let mut values = Vec::with_capacity(slides.len());
    for slide in slides {
        let mut value = serde_json::Map::new();
        value.insert("id".to_owned(), Value::from(slide.id.as_str()));
        value.insert("origin".to_owned(), json!({ "kind": "agent" }));
        value.insert("hidden".to_owned(), Value::Bool(false));
        value.insert("showLayout".to_owned(), Value::Bool(true));
        value.insert(
            "timeline".to_owned(),
            serde_json::to_value(&slide.timeline)?,
        );
        if let Some(transition) = &slide.transition {
            value.insert("transition".to_owned(), serde_json::to_value(transition)?);
        }
        let boxes: Vec<Value> = slide.boxes.iter().map(text_box).collect();
        value.insert("elements".to_owned(), Value::Array(boxes));
        values.push(Value::Object(value));
    }
    Ok(json!({
        "$schema": crate::presentation::SchemaUrl::WIRE, "size": { "w": 720, "h": 405 },
        "defaults": { "body": { "anchor": "top", "autofit": { "kind": "none" },
                                "insets": { "b": 3.6, "l": 7.2, "r": 7.2, "t": 3.6 }, "overflow": "spread",
                                "wrap": "square" }, "text": text },
        "themes": {}, "masters": {}, "layouts": {}, "fonts": {}, "media": {}, "phase": "source",
        "slides": values
    }))
}

/// Why a fixture presentation cannot be built.
#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Structure(#[from] StructureError),
}

/// [`presentation_json`], read as a source presentation.
pub fn presentation(slides: &[Slide]) -> Result<Presentation, FixtureError> {
    Ok(from_value(presentation_json(slides)?)?)
}
