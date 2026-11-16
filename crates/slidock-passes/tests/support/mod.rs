//! What the pass suites share: running passes the way the manager does without its checks, Rust builders for the
//! presentations the tests start from, and, behind `golden`, the private IR fixtures lowered as the IR frontend lowers
//! them.
#![allow(dead_code, reason = "each suite uses part of the support")]

use serde_json::{Value, json};
use slidock_core_api::visit::walk_elements;
use slidock_core_api::{
    HostDecoded, Limits, MediaPolicy, OwnedContainer, Pass, PassEnv, PresentationFacts, Sink,
    view_of,
};
use slidock_fonts::{FontCatalog, builtin_catalog};
use slidock_ir::diagnostics::{Diagnostic, Stage};
use slidock_ir::ids::NodeId;
use slidock_ir::source::{Container, Element, Presentation};

pub fn id(text: &str) -> NodeId {
    NodeId::new(text).unwrap()
}

/// What a pass run gives: the presentation and the diagnostics on the way.
#[derive(Clone, Debug)]
pub struct Applied {
    pub presentation: Presentation,
    pub diagnostics: Vec<Diagnostic>,
}

/// The context passes run in: the built-in fonts, default limits and policy, and the presentation's own facts.
pub struct Env {
    pub limits: Limits,
    pub fonts: FontCatalog,
    pub policy: MediaPolicy,
    pub facts: PresentationFacts,
    /// None, as without a host that decodes.
    pub host_media: Vec<HostDecoded>,
}

impl Env {
    pub fn of(presentation: &Presentation) -> Env {
        Env {
            limits: Limits::default(),
            fonts: builtin_catalog().clone(),
            policy: MediaPolicy::default(),
            facts: facts_of(presentation),
            host_media: Vec::new(),
        }
    }

    pub fn env(&self) -> PassEnv<'_> {
        PassEnv {
            limits: &self.limits,
            fonts: &self.fonts,
            policy: &self.policy,
            facts: &self.facts,
            host_media: &self.host_media,
        }
    }
}

pub fn facts_of(presentation: &Presentation) -> PresentationFacts {
    serde_json::from_value(json!({
        "size": presentation.size,
        "slides": presentation.slides.iter().map(|s| json!({ "id": s.id, "hidden": s.hidden })).collect::<Vec<_>>(),
        "mediaBytes": 0,
    }))
    .unwrap()
}

/// One pass the way the manager runs it, skipping the invariant checks: a presentation-scope pass on the
/// presentation, a slide-scope pass once per container on a view of the input presentation. Failed containers keep
/// their input.
pub fn apply_pass_in(pass: &Pass, presentation: &Presentation, env: &Env) -> Applied {
    let mut diagnostics = Vec::new();
    let mut sink = Sink::new(&mut diagnostics, Stage::Pass, pass.name());
    let env = env.env();
    let presentation = match pass {
        Pass::Presentation(pass) => pass.run(presentation, &mut env.ctx(&mut sink)).unwrap(),
        Pass::Slide(pass) => {
            let mut out = presentation.clone();
            let outputs: Vec<OwnedContainer> = presentation
                .containers()
                .filter(|c| c.failed().is_none())
                .map(|container| {
                    let view = view_of(presentation, container, env.facts);
                    pass.run(&view, &mut env.ctx(&mut sink)).unwrap()
                })
                .collect();
            for output in outputs {
                match output {
                    OwnedContainer::Master(m) => {
                        out.masters.insert(m.id.clone(), m);
                    }
                    OwnedContainer::Layout(l) => {
                        out.layouts.insert(l.id.clone(), l);
                    }
                    OwnedContainer::Slide(s) => {
                        let slot = out.slides.iter_mut().find(|slot| slot.id == s.id).unwrap();
                        *slot = s;
                    }
                }
            }
            out
        }
    };
    Applied {
        presentation,
        diagnostics,
    }
}

pub fn apply_pass(pass: &Pass, presentation: &Presentation) -> Applied {
    apply_pass_in(pass, presentation, &Env::of(presentation))
}

/// The passes in order, collecting every diagnostic.
pub fn apply_passes(passes: &[Pass], presentation: &Presentation) -> Applied {
    let env = Env::of(presentation);
    let mut applied = Applied {
        presentation: presentation.clone(),
        diagnostics: Vec::new(),
    };
    for pass in passes {
        let next = apply_pass_in(pass, &applied.presentation, &env);
        applied.presentation = next.presentation;
        applied.diagnostics.extend(next.diagnostics);
    }
    applied
}

/// The pass of that name.
pub fn pass_named(name: &str) -> Pass {
    slidock_passes::passes()
        .into_iter()
        .find(|pass| pass.name() == name)
        .unwrap_or_else(|| panic!("no pass {name}"))
}

/// The passes before the one of that name.
pub fn passes_before(name: &str) -> Vec<Pass> {
    slidock_passes::passes()
        .into_iter()
        .take_while(|pass| pass.name() != name)
        .collect()
}

/// The element of that id, wherever it is.
pub fn element<'a>(presentation: &'a Presentation, id: &str) -> &'a Element {
    presentation
        .containers()
        .flat_map(|c: Container<'a>| walk_elements(c.elements()))
        .find(|e| e.id().as_str() == id)
        .unwrap_or_else(|| panic!("no element {id}"))
}

pub fn shape<'a>(presentation: &'a Presentation, id: &str) -> &'a slidock_ir::source::Shape {
    match element(presentation, id) {
        Element::Shape(shape) => shape,
        other => panic!("{id} is a {}", other.kind()),
    }
}

/// A presentation from its JSON.
pub fn parse(value: Value) -> Presentation {
    serde_json::from_value(value).unwrap()
}

/// The JSON of a value, for assertions written as JSON, its numbers as the canonical form prints them (a whole
/// number as an integer), so it compares equal to the same JSON built with `json!`.
pub fn to_json<T: serde::Serialize>(value: &T) -> Value {
    serde_json::from_str(&slidock_ir::canonical::to_json(value).unwrap()).unwrap()
}

// Builders.

pub fn agent() -> Value {
    json!({ "kind": "agent" })
}

pub fn rgb(r: f64, g: f64, b: f64) -> Value {
    to_json(&json!({ "kind": "rgb", "space": "srgb", "r": r, "g": g, "b": b, "a": 1 }))
}

pub fn black() -> Value {
    rgb(0.0, 0.0, 0.0)
}

pub fn frame(x: f64, y: f64, w: f64, h: f64) -> Value {
    to_json(
        &json!({ "x": x, "y": y, "w": w, "h": h, "rotation": 0, "flipH": false, "flipV": false }),
    )
}

/// `frame` with these fields changed.
pub fn with(mut value: Value, changes: Value) -> Value {
    for (key, change) in changes.as_object().unwrap() {
        value[key] = change.clone();
    }
    value
}

pub fn rect(id: &str, at: Value) -> Value {
    json!({ "id": id, "kind": "shape", "origin": agent(), "frame": at })
}

pub fn run(text: &str, props: Value) -> Value {
    json!({ "kind": "run", "text": text, "props": props })
}

pub fn paragraph(level: u32, props: Value, inlines: Vec<Value>) -> Value {
    json!({ "level": level, "props": props, "inlines": inlines })
}

/// A text body with no box properties of its own.
pub fn text(paragraphs: Vec<Value>) -> Value {
    json!({ "body": {}, "paragraphs": paragraphs })
}

fn para_defaults() -> Value {
    json!({
        "align": "left", "marginLeft": 0, "marginRight": 0, "indent": 0,
        "spaceBefore": { "kind": "pt", "value": 0 }, "spaceAfter": { "kind": "pt", "value": 0 },
        "lineSpacing": { "kind": "multiple", "value": 1, "basis": "size-1.2" }, "tabSize": 72,
        "bullet": { "kind": "none" }, "rtl": false
    })
}

fn run_defaults() -> Value {
    json!({
        "font": { "kind": "theme", "slot": "minor" }, "size": 18, "weight": 400, "italic": false,
        "underline": "none", "strike": "none", "color": { "kind": "theme", "slot": "tx1", "mods": [] },
        "highlight": null, "baseline": 0, "caps": "none", "letterSpacing": 0, "link": null,
        "synthesizeBold": true, "scriptScale": 1
    })
}

/// PowerPoint-like defaults: left-aligned single-spaced 18 pt text in the theme's body font, every level alike.
pub fn defaults() -> Value {
    let levels: serde_json::Map<String, Value> = (0..9)
        .map(|level| {
            (
                level.to_string(),
                json!({ "para": para_defaults(), "run": run_defaults() }),
            )
        })
        .collect();
    json!({
        "text": levels,
        "body": { "insets": { "l": 7.2, "t": 3.6, "r": 7.2, "b": 3.6 }, "anchor": "top", "wrap": "square",
                  "autofit": { "kind": "none" }, "overflow": "spread" }
    })
}

pub fn slide(id: &str, elements: Vec<Value>) -> Value {
    json!({ "id": id, "origin": agent(), "hidden": false, "showLayout": true, "timeline": [], "elements": elements })
}

/// One 720 × 405 slide with one centred line of white 40 pt Liberation Sans on a dark background, and nothing
/// else: no theme, master or layout.
pub fn minimal_json() -> Value {
    let hello = with(
        rect("ir-s1.hello", frame(60.0, 160.0, 600.0, 80.0)),
        json!({
            "geometry": { "kind": "preset", "name": "rect", "adjust": {} },
            "text": text(vec![paragraph(0, json!({ "align": "center" }), vec![run(
                "Hello, presentation",
                json!({ "font": { "kind": "family", "family": "Liberation Sans" }, "size": 40,
                        "color": rgb(1.0, 1.0, 1.0) }),
            )])]),
        }),
    );
    let mut first = slide("ir-s1", vec![hello]);
    first["background"] = json!({ "kind": "solid", "color": rgb(0.12941, 0.12941, 0.12941) });
    json!({
        "$schema": slidock_ir::presentation::SchemaUrl::WIRE,
        "size": { "w": 720, "h": 405 },
        "defaults": defaults(),
        "themes": {}, "phase": "source", "masters": {}, "layouts": {},
        "slides": [first], "fonts": {}, "media": {}
    })
}

pub fn minimal() -> Presentation {
    parse(minimal_json())
}

/// The minimal presentation with these elements on its slide.
pub fn with_elements(elements: Vec<Value>) -> Presentation {
    let mut presentation = minimal_json();
    presentation["slides"][0]["elements"] = json!(elements);
    parse(presentation)
}

#[cfg(feature = "golden")]
#[path = "../../../../tests/golden/support.rs"]
pub mod golden;

#[cfg(feature = "golden")]
pub mod fixtures;
