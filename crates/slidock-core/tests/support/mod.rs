//! Fixtures and fake passes and frontends for the core suites.
//!
//! `full` is a presentation of our own with every container kind and the element kinds the manager walks: a theme,
//! a master and a layout with title and body placeholders, a slide inheriting them with a shape, a picture, nested
//! groups and a Foreign table with fallback elements, and a hidden slide with a slide link and a table. It is
//! written in its resolved form and marked as source, so every resolved invariant already holds: identity passes
//! resolve it, and a test pass that breaks one rule shows exactly that rule. `minimal` is one slide with one shape.
#![allow(dead_code, reason = "each suite uses part of the support")]

use std::borrow::Cow;

use serde_json::{Value, json};
use slidock_core::{PassManager, RunOptions, RunResult, VerifyMode};
use slidock_core_api::{
    ContainerOrder, ContainerView, Limits, MediaPolicy, OwnedContainer, Pass, PassCtx, PassEnv,
    PassError, PassInfo, PresentationFacts, PresentationPass, SlidePass, Unverified,
    sort_diagnostics,
};
use slidock_fonts::{FontCatalog, builtin_catalog};
use slidock_ir::diagnostics::Diagnostic;
use slidock_ir::ids::{NodeId, Sha256};
use slidock_ir::parse;
use slidock_ir::source::Presentation;
use slidock_ir::verify::Invariant;

pub fn id(text: &str) -> NodeId {
    NodeId::new(text).unwrap()
}

fn agent() -> Value {
    json!({ "kind": "agent" })
}

fn rgb(r: f64, g: f64, b: f64) -> Value {
    json!({ "kind": "rgb", "space": "srgb", "r": r, "g": g, "b": b, "a": 1 })
}

pub fn photo_sha() -> String {
    Sha256::of(b"photo").to_hex()
}

pub fn logo_sha() -> String {
    Sha256::of(b"logo").to_hex()
}

fn run_props(family: &str, size: f64) -> Value {
    json!({
        "font": { "kind": "family", "family": family }, "size": size, "weight": 400, "italic": false,
        "underline": "none", "strike": "none", "color": rgb(0.0, 0.0, 0.0), "highlight": null, "baseline": 0,
        "caps": "none", "letterSpacing": 0, "link": null, "synthesizeBold": true
    })
}

fn para_props(metrics: bool) -> Value {
    let mut props = json!({
        "align": "left", "marginLeft": 0, "marginRight": 0, "indent": 0,
        "spaceBefore": { "kind": "pt", "value": 0 }, "spaceAfter": { "kind": "pt", "value": 0 },
        "lineSpacing": { "kind": "multiple", "value": 1, "basis": "size-1.2" }, "tabSize": 72,
        "bullet": { "kind": "none" }, "rtl": false
    });
    if metrics {
        props["lineHeight"] = json!({ "kind": "ratio", "value": 1.2 });
        props["spaceBeforePt"] = json!(0);
        props["spaceAfterPt"] = json!(0);
    }
    props
}

fn body() -> Value {
    json!({ "insets": { "l": 7.2, "t": 3.6, "r": 7.2, "b": 3.6 }, "anchor": "top", "wrap": "square",
            "autofit": { "kind": "none" }, "overflow": "spread" })
}

/// A resolved text body of one paragraph per line, each one run of `family`.
fn text(family: &str, lines: &[&str]) -> Value {
    let paragraphs: Vec<Value> = lines
        .iter()
        .map(|line| {
            json!({ "level": 0, "props": para_props(true),
                    "inlines": [{ "kind": "run", "text": line, "props": run_props(family, 18.0) }] })
        })
        .collect();
    json!({ "body": body(), "rotation": 0, "paragraphs": paragraphs })
}

fn frame(x: f64, y: f64, w: f64, h: f64) -> Value {
    json!({ "x": x, "y": y, "w": w, "h": h, "rotation": 0, "flipH": false, "flipV": false })
}

fn rect(w: f64, h: f64) -> Value {
    json!({ "kind": "paths", "preset": "rect", "paths": [{ "fill": true, "stroke": true, "fillRule": "nonzero",
        "segments": [["M", 0, 0], ["L", w, 0], ["L", w, h], ["L", 0, h], ["Z"]] }] })
}

fn shape(id: &str, at: Value, extra: Value) -> Value {
    let (w, h) = (at["w"].as_f64().unwrap(), at["h"].as_f64().unwrap());
    let mut shape = json!({ "id": id, "origin": agent(), "kind": "shape", "frame": at, "geometry": rect(w, h) });
    for (key, value) in extra.as_object().unwrap() {
        shape[key] = value.clone();
    }
    shape
}

fn picture(id: &str, at: Value, sha: String) -> Value {
    json!({ "id": id, "origin": agent(), "kind": "picture", "frame": at, "media": sha })
}

fn cell(family: &str, words: &str, extra: Value) -> Value {
    let mut cell = json!({ "text": text(family, &[words]) });
    for (key, value) in extra.as_object().unwrap() {
        cell[key] = value.clone();
    }
    cell
}

fn binding(family: &str) -> Value {
    let metrics = json!({ "unitsPerEm": 1000, "hhea": { "ascender": 900, "descender": -200, "lineGap": 0 },
        "typo": { "ascender": 900, "descender": -200, "lineGap": 0 }, "win": { "ascent": 900, "descent": 200 } });
    json!({ "kind": "installed", "family": family, "generic": "sans-serif", "metrics": metrics })
}

fn media(sha: String, part: &str) -> Value {
    json!({ "sha256": sha, "type": "image/png", "bytes": 10, "origin": { "kind": "source", "part": part },
        "plan": { "action": "copy", "output": { "file": format!("media/{sha}.png"), "type": "image/png" } } })
}

fn defaults() -> Value {
    let levels: serde_json::Map<String, Value> = (0..9)
        .map(|i| {
            let mut run = run_props("Lato", 18.0);
            run["scriptScale"] = json!(1);
            (
                i.to_string(),
                json!({ "para": para_props(false), "run": run }),
            )
        })
        .collect();
    json!({ "text": levels, "body": body() })
}

fn reading_order(slide: &mut Value) {
    let order: Vec<Value> = slide["elements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].clone())
        .collect();
    slide["readingOrder"] = json!(order);
}

/// The full presentation as JSON.
pub fn full_json() -> Value {
    let colors: serde_json::Map<String, Value> = [
        "dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5",
        "accent6", "hlink", "folHlink",
    ]
    .iter()
    .map(|slot| ((*slot).to_owned(), rgb(0.5, 0.5, 0.5)))
    .collect();
    let master_title = shape(
        "ir-master.title",
        frame(36.0, 20.0, 648.0, 60.0),
        json!({ "placeholder": { "role": "title" }, "text": text("Lato", &["Master title"]) }),
    );
    let master_body = shape(
        "ir-master.body",
        frame(36.0, 100.0, 648.0, 260.0),
        json!({ "placeholder": { "role": "body", "index": 1 }, "text": text("Lato", &["Master body"]) }),
    );
    let logo = picture(
        "ir-master.logo",
        frame(640.0, 360.0, 48.0, 24.0),
        logo_sha(),
    );
    let layout_title = shape(
        "ir-layout.title",
        frame(36.0, 20.0, 648.0, 60.0),
        json!({ "placeholder": { "role": "title" }, "inherits": "ir-master.title" }),
    );
    let layout_body = shape(
        "ir-layout.body",
        frame(36.0, 100.0, 648.0, 260.0),
        json!({ "placeholder": { "role": "body", "index": 1 }, "inherits": "ir-master.body" }),
    );
    let title = shape(
        "ir-s1.title",
        frame(36.0, 20.0, 648.0, 60.0),
        json!({ "placeholder": { "role": "title" }, "inherits": "ir-layout.title",
                "text": text("Lato", &["Presentation IR"]) }),
    );
    let body_shape = shape(
        "ir-s1.body",
        frame(36.0, 100.0, 300.0, 120.0),
        json!({ "placeholder": { "role": "body", "index": 1 }, "inherits": "ir-layout.body",
                "text": text("Lato", &["First point", "Second point"]) }),
    );
    let arrow = shape(
        "ir-s1.arrow",
        frame(360.0, 120.0, 120.0, 40.0),
        json!({ "fill": { "kind": "solid", "color": rgb(0.2, 0.4, 0.6) }, "stroke": null }),
    );
    let photo = picture("ir-s1.photo", frame(36.0, 300.0, 120.0, 90.0), photo_sha());
    let group = json!({ "id": "ir-s1.group", "origin": agent(), "kind": "group", "frame": frame(200.0, 300.0, 100.0, 80.0),
        "children": [
            shape("ir-s1.group.dot", frame(0.0, 0.0, 50.0, 50.0), json!({})),
            { "id": "ir-s1.group.inner", "origin": agent(), "kind": "group", "frame": frame(50.0, 50.0, 50.0, 30.0),
              "children": [picture("ir-s1.group.inner.logo", frame(0.0, 0.0, 50.0, 30.0), logo_sha())] }
        ] });
    let table = json!({ "id": "ir-s1.table", "origin": agent(), "kind": "foreign", "dialect": "pptx", "type": "table",
        "frame": frame(320.0, 200.0, 360.0, 120.0),
        "fallback": { "elements": [shape("ir-s1.table.cell", frame(0.0, 0.0, 360.0, 120.0),
            json!({ "text": text("Lato", &["a b c"]) }))] } });
    let mut slide1 = json!({ "id": "ir-s1", "origin": agent(), "layout": "ir-layout", "hidden": false,
        "showLayout": true, "elements": [title, body_shape, arrow, photo, group, table],
        "timeline": [{ "target": { "element": "ir-s1.body" }, "trigger": "click", "delayMs": 0, "durationMs": 500,
            "kind": "entrance", "effect": { "id": "keynote:dissolve", "fallback": "dissolve" }, "delivery": "all" }],
        "title": "Presentation IR" });
    reading_order(&mut slide1);
    let mut note = shape(
        "ir-s2.note",
        frame(60.0, 60.0, 600.0, 80.0),
        json!({ "text": text("Lato", &["Back to the start"]) }),
    );
    note["text"]["paragraphs"][0]["inlines"][0]["props"]["link"] =
        json!({ "kind": "slide", "slide": "ir-s1" });
    let grid = json!({ "id": "ir-s2.grid", "origin": agent(), "kind": "table", "frame": frame(60.0, 200.0, 250.0, 70.0),
        "columns": [{ "width": 100 }, { "width": 150 }],
        "rows": [
            { "height": 30, "cells": [cell("Lato", "Header", json!({ "colSpan": 2 })), null] },
            { "height": 40, "cells": [cell("Lato", "a", json!({})), cell("Lato", "b", json!({}))] }
        ] });
    let mut slide2 = json!({ "id": "ir-s2", "origin": agent(), "layout": "ir-layout", "hidden": true,
        "showLayout": false, "elements": [note, grid], "timeline": [], "title": "Back to the start",
        "background": { "kind": "solid", "color": rgb(0.9, 0.9, 0.9) } });
    reading_order(&mut slide2);
    json!({
        "size": { "w": 720, "h": 405 },
        "defaults": defaults(),
        "themes": { "ir-theme": { "id": "ir-theme", "origin": agent(), "name": "Fixture theme", "colors": colors,
            "fonts": { "major": { "latin": "Lato" }, "minor": { "latin": "Lato" } } } },
        "phase": "source",
        "masters": { "ir-master": { "id": "ir-master", "origin": agent(), "name": "Fixture master", "theme": "ir-theme",
            "background": { "kind": "solid", "color": rgb(1.0, 1.0, 1.0) },
            "elements": [master_title, master_body, logo] } },
        "layouts": { "ir-layout": { "id": "ir-layout", "origin": agent(), "name": "Title and Content", "type": "obj",
            "master": "ir-master", "showMaster": true, "elements": [layout_title, layout_body] } },
        "slides": [slide1, slide2],
        "fonts": { "lato": { "request": { "family": "Lato", "faces": [{ "weight": 400, "style": "normal" }],
            "postscriptNames": [], "embedded": false }, "binding": binding("Lato") } },
        "media": { photo_sha(): media(photo_sha(), "media/photo.png"), logo_sha(): media(logo_sha(), "media/logo.png") }
    })
}

/// The minimal presentation as JSON.
pub fn minimal_json() -> Value {
    let mut slide = json!({ "id": "ir-s1", "origin": agent(), "hidden": false, "showLayout": true, "timeline": [],
        "elements": [shape("ir-s1.hello", frame(60.0, 160.0, 600.0, 80.0),
            json!({ "text": text("Liberation Sans", &["Hello, presentation"]) }))],
        "title": "Hello, presentation",
        "background": { "kind": "solid", "color": rgb(0.12941, 0.12941, 0.12941) } });
    reading_order(&mut slide);
    json!({
        "size": { "w": 720, "h": 405 },
        "defaults": defaults(),
        "themes": {},
        "phase": "source",
        "masters": {},
        "layouts": {},
        "slides": [slide],
        "fonts": { "liberationsans": { "request": { "family": "Liberation Sans",
            "faces": [{ "weight": 400, "style": "normal" }], "postscriptNames": [], "embedded": false },
            "binding": binding("Liberation Sans") } },
        "media": {}
    })
}

pub fn typed(value: Value) -> Presentation {
    parse::from_value(value).unwrap()
}

pub fn full() -> Presentation {
    typed(full_json())
}

pub fn minimal() -> Presentation {
    typed(minimal_json())
}

/// A copy of slide `from` with every id prefix `from` renamed to `to`, appended to the show, optionally without its
/// layout.
pub fn copy_slide(presentation: &Value, from: &str, to: &str, layout: bool) -> Value {
    let slide = presentation["slides"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == from)
        .unwrap();
    let text = serde_json::to_string(slide)
        .unwrap()
        .replace(&format!("\"{from}"), &format!("\"{to}"));
    let mut copy: Value = serde_json::from_str(&text).unwrap();
    if !layout {
        copy.as_object_mut().unwrap().shift_remove("layout");
    }
    let mut out = presentation.clone();
    out["slides"].as_array_mut().unwrap().push(copy);
    out
}

pub fn facts_of(presentation: &Presentation) -> PresentationFacts {
    let value = json!({
        "size": presentation.size,
        "slides": presentation.slides.iter().map(|s| json!({ "id": s.id, "hidden": s.hidden })).collect::<Vec<_>>(),
        "mediaBytes": 0,
    });
    serde_json::from_value(value).unwrap()
}

/// What a pass context holds besides its sink, for one presentation.
pub struct Shared {
    pub limits: Limits,
    pub policy: MediaPolicy,
    pub facts: PresentationFacts,
}

impl Shared {
    pub fn of(presentation: &Presentation) -> Self {
        Shared {
            limits: Limits::default(),
            policy: MediaPolicy::default(),
            facts: facts_of(presentation),
        }
    }

    pub fn env(&self) -> PassEnv<'_> {
        self.env_with(builtin_catalog())
    }

    pub fn env_with<'a>(&'a self, fonts: &'a FontCatalog) -> PassEnv<'a> {
        PassEnv {
            limits: &self.limits,
            fonts,
            policy: &self.policy,
            facts: &self.facts,
            host_media: &[],
        }
    }
}

type SlideFn = dyn Fn(&ContainerView<'_>, &mut PassCtx<'_>) -> Result<OwnedContainer, PassError>;
type PresentationFn = dyn Fn(&Presentation, &mut PassCtx<'_>) -> Result<Presentation, PassError>;

/// A slide-scope pass from a function.
pub struct FnSlidePass {
    info: PassInfo,
    run: Box<SlideFn>,
}

impl SlidePass for FnSlidePass {
    fn info(&self) -> &PassInfo {
        &self.info
    }

    fn run(
        &self,
        view: &ContainerView<'_>,
        ctx: &mut PassCtx<'_>,
    ) -> Result<OwnedContainer, PassError> {
        (self.run)(view, ctx)
    }
}

/// A presentation-scope pass from a function.
pub struct FnPresentationPass {
    info: PassInfo,
    run: Box<PresentationFn>,
}

impl PresentationPass for FnPresentationPass {
    fn info(&self) -> &PassInfo {
        &self.info
    }

    fn run(
        &self,
        presentation: &Presentation,
        ctx: &mut PassCtx<'_>,
    ) -> Result<Presentation, PassError> {
        (self.run)(presentation, ctx)
    }
}

pub fn info(name: &str, requires: &[Invariant], provides: &[Invariant]) -> PassInfo {
    PassInfo {
        name: Cow::Owned(name.to_owned()),
        version: 1,
        requires: Cow::Owned(requires.to_vec()),
        provides: Cow::Owned(provides.to_vec()),
    }
}

pub fn slide_pass_with(
    info: PassInfo,
    run: impl Fn(&ContainerView<'_>, &mut PassCtx<'_>) -> Result<OwnedContainer, PassError> + 'static,
) -> Pass {
    Pass::slide(FnSlidePass {
        info,
        run: Box::new(run),
    })
}

/// A slide pass that returns its container unchanged.
pub fn identity(name: &str) -> Pass {
    slide_pass_with(info(name, &[], &[]), |view, _| Ok(view.unchanged()))
}

pub fn slide_pass(
    name: &str,
    run: impl Fn(&ContainerView<'_>, &mut PassCtx<'_>) -> Result<OwnedContainer, PassError> + 'static,
) -> Pass {
    slide_pass_with(info(name, &[], &[]), run)
}

pub fn presentation_pass_with(
    info: PassInfo,
    run: impl Fn(&Presentation, &mut PassCtx<'_>) -> Result<Presentation, PassError> + 'static,
) -> Pass {
    Pass::presentation(FnPresentationPass {
        info,
        run: Box::new(run),
    })
}

pub fn presentation_pass(
    name: &str,
    run: impl Fn(&Presentation, &mut PassCtx<'_>) -> Result<Presentation, PassError> + 'static,
) -> Pass {
    presentation_pass_with(info(name, &[], &[]), run)
}

/// A presentation pass that claims every resolved invariant except `except`, for presentations that already hold
/// them.
pub fn claim_all(except: &[Invariant]) -> Pass {
    let provides: Vec<Invariant> = Invariant::RESOLVED
        .into_iter()
        .filter(|invariant| !except.contains(invariant))
        .collect();
    presentation_pass_with(info("claim-all", &[], &provides), |p, _| Ok(p.clone()))
}

/// A slide pass that strips run sizes on one slide: it claims text.cascaded and breaks it there.
pub fn cascade_liar(target: &str) -> Pass {
    let target = id(target);
    slide_pass_with(
        info("cascade-liar", &[], &[Invariant::TextCascaded]),
        move |view, _| {
            let mut out = view.unchanged();
            if view.container.id() != &target {
                return Ok(out);
            }
            if let OwnedContainer::Slide(slide) = &mut out {
                for element in &mut slide.elements {
                    if let slidock_ir::source::Element::Shape(shape) = element
                        && let Some(text) = &mut shape.text
                    {
                        for paragraph in &mut text.paragraphs {
                            for inline in &mut paragraph.inlines {
                                if let slidock_ir::text::Inline::Run { props, .. } = inline {
                                    props.size = None;
                                }
                            }
                        }
                    }
                }
            }
            Ok(out)
        },
    )
}

/// A clock that moves `step` milliseconds every time it is read.
pub struct Ticks {
    now: std::cell::Cell<f64>,
    step: f64,
}

impl Ticks {
    pub fn new(step: f64) -> Self {
        Ticks {
            now: std::cell::Cell::new(0.0),
            step,
        }
    }

    /// Moves the clock by `ms` without a read.
    pub fn advance(&self, ms: f64) {
        self.now.set(self.now.get() + ms);
    }
}

impl slidock_core_api::Clock for Ticks {
    fn now_ms(&self) -> f64 {
        self.now.set(self.now.get() + self.step);
        self.now.get()
    }
}

pub fn manager(passes: Vec<Pass>) -> PassManager {
    PassManager::new(passes).unwrap()
}

/// The passes over `presentation` in `verify` mode, with a fresh context.
/// Run options besides the mode and the context.
#[derive(Clone, Copy, Default)]
pub struct Extra<'a> {
    pub clock: Option<&'a dyn slidock_core_api::Clock>,
    pub visit_order: Option<slidock_core::manager::VisitOrder<'a>>,
    pub trace: bool,
}

pub fn run_with(
    passes: &PassManager,
    presentation: &Presentation,
    verify: VerifyMode,
    extra: Extra<'_>,
) -> RunResult {
    let shared = Shared::of(presentation);
    let options = RunOptions {
        verify,
        env: shared.env(),
        clock: extra.clock,
        deadline: slidock_core_api::Deadline::never(),
        visit_order: extra.visit_order,
        trace: extra.trace,
    };
    passes.run(&Unverified::from(presentation.clone()), &options)
}

pub fn run(passes: Vec<Pass>, presentation: &Presentation, verify: VerifyMode) -> RunResult {
    run_with(&manager(passes), presentation, verify, Extra::default())
}

/// `[code, container]` of each diagnostic.
pub fn codes(diagnostics: &[Diagnostic]) -> Vec<(String, Option<String>)> {
    diagnostics
        .iter()
        .map(|d| {
            (
                d.code.code().to_owned(),
                d.container.as_ref().map(|c| c.to_string()),
            )
        })
        .collect()
}

pub fn errors(diagnostics: &[Diagnostic]) -> Vec<Diagnostic> {
    diagnostics
        .iter()
        .filter(|d| d.severity == slidock_ir::diagnostics::Severity::Error)
        .cloned()
        .collect()
}

/// Presentation and sorted diagnostics as bytes, for byte-identity checks.
pub fn bytes(result: &RunResult) -> String {
    let order = match &result.presentation {
        Some(presentation) => ContainerOrder::of_resolved(presentation),
        None => ContainerOrder::of(result.source.as_ref()),
    };
    let diagnostics = sort_diagnostics(result.diagnostics.clone(), &order);
    slidock_ir::canonical::to_json(
        &json!({ "presentation": result.presentation, "diagnostics": diagnostics }),
    )
    .unwrap()
}

/// A data value of a diagnostic, as a string.
pub fn data(diagnostic: &Diagnostic, key: &str) -> Option<String> {
    diagnostic
        .data
        .as_ref()
        .and_then(|data| data.get(key))
        .map(|value| value.to_js_string())
}

pub fn find<'a>(diagnostics: &'a [Diagnostic], code: &str) -> Option<&'a Diagnostic> {
    diagnostics.iter().find(|d| d.code.code() == code)
}

type SniffFn = dyn Fn(&[u8], &slidock_core_api::SniffHints<'_>) -> bool;
type AnalyzeFn = dyn Fn(
    &Presentation,
    &mut slidock_core_api::FrontendCtx<'_>,
) -> Result<slidock_core_api::Analysis, slidock_core_api::FrontendError>;
type HookFn =
    dyn Fn(&mut slidock_core_api::FrontendCtx<'_>) -> Result<(), slidock_core_api::FrontendError>;

/// What a fake frontend's `lower` was asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct LowerCall {
    pub facts: PresentationFacts,
    pub only: Option<Vec<NodeId>>,
}

/// A frontend that "reads" any input starting with `FAKE` as its presentation.
pub struct FakeFrontend {
    pub format: slidock_ir::formats::SourceFormat,
    presentation: Presentation,
    sniff: Box<SniffFn>,
    analyze: Box<AnalyzeFn>,
    before_lower: Box<HookFn>,
    pub calls: std::rc::Rc<std::cell::RefCell<Vec<LowerCall>>>,
    /// Hand on the presentation as JSON, as the IR frontend does.
    pub as_json: bool,
}

pub fn analysis_of(presentation: &Presentation) -> slidock_core_api::Analysis {
    slidock_core_api::Analysis {
        format: slidock_ir::formats::SourceFormat::File(slidock_ir::formats::FileFormat::Pptx),
        size: presentation.size.clone(),
        slides: presentation
            .slides
            .iter()
            .map(|s| slidock_core_api::frontend::SlideFacts {
                id: s.id.clone(),
                hidden: s.hidden,
                title: None,
            })
            .collect(),
        features: Vec::new(),
        media_bytes: 0,
        refusal: None,
    }
}

impl FakeFrontend {
    pub fn new(presentation: Presentation) -> Self {
        FakeFrontend {
            format: slidock_ir::formats::SourceFormat::File(slidock_ir::formats::FileFormat::Pptx),
            presentation,
            sniff: Box::new(|head, _| head.starts_with(b"FAKE")),
            analyze: Box::new(|presentation, _| Ok(analysis_of(presentation))),
            before_lower: Box::new(|_| Ok(())),
            calls: std::rc::Rc::default(),
            as_json: false,
        }
    }

    /// A frontend of `format` that only sniffs.
    pub fn sniffing(format: slidock_ir::formats::SourceFormat, sniffs: bool) -> Self {
        FakeFrontend {
            format,
            sniff: Box::new(move |_, _| sniffs),
            ..FakeFrontend::new(minimal())
        }
    }

    /// Hands on the presentation as JSON.
    pub fn json(mut self) -> Self {
        self.as_json = true;
        self
    }

    pub fn with_sniff(
        mut self,
        sniff: impl Fn(&[u8], &slidock_core_api::SniffHints<'_>) -> bool + 'static,
    ) -> Self {
        self.sniff = Box::new(sniff);
        self
    }

    pub fn with_analyze(
        mut self,
        analyze: impl Fn(
            &Presentation,
            &mut slidock_core_api::FrontendCtx<'_>,
        ) -> Result<slidock_core_api::Analysis, slidock_core_api::FrontendError>
        + 'static,
    ) -> Self {
        self.analyze = Box::new(analyze);
        self
    }

    pub fn before_lower(
        mut self,
        hook: impl Fn(
            &mut slidock_core_api::FrontendCtx<'_>,
        ) -> Result<(), slidock_core_api::FrontendError>
        + 'static,
    ) -> Self {
        self.before_lower = Box::new(hook);
        self
    }
}

impl slidock_core_api::Frontend for FakeFrontend {
    fn format(&self) -> slidock_ir::formats::SourceFormat {
        self.format
    }

    fn version(&self) -> &str {
        "test"
    }

    fn sniff(&self, head: &[u8], hints: &slidock_core_api::SniffHints<'_>) -> bool {
        (self.sniff)(head, hints)
    }

    fn analyze(
        &self,
        _input: &slidock_core_api::Input<'_>,
        ctx: &mut slidock_core_api::FrontendCtx<'_>,
    ) -> Result<slidock_core_api::Analysis, slidock_core_api::FrontendError> {
        (self.analyze)(&self.presentation, ctx)
    }

    fn lower(
        &self,
        input: &slidock_core_api::Input<'_>,
        ctx: &mut slidock_core_api::FrontendCtx<'_>,
        options: &slidock_core_api::LowerOptions<'_>,
    ) -> Result<slidock_core_api::LowerResult, slidock_core_api::FrontendError> {
        self.calls.borrow_mut().push(LowerCall {
            facts: options.facts.clone(),
            only: options.only.map(<[NodeId]>::to_vec),
        });
        (self.before_lower)(ctx)?;
        ctx.diag.report(
            slidock_ir::diagnostics::codes::IR0905,
            slidock_ir::diagnostics::DiagnosticSite::new()
                .data("stage", "note")
                .data("reason", "frontend diagnostics are stamped"),
        );
        let mut presentation = self.presentation.clone();
        if let Some(only) = options.only {
            presentation.slides.retain(|slide| only.contains(&slide.id));
        }
        let provenance = slidock_core_api::Provenance {
            format: self.format,
            name: input.name.to_owned(),
            sha256: Sha256([0; 32]),
            bytes: input.size(),
            app: None,
        };
        let mut features = indexmap::IndexMap::new();
        features.insert("presentation".to_owned(), Vec::new());
        Ok(slidock_core_api::LowerResult {
            provenance,
            presentation: if self.as_json {
                Unverified::Json(serde_json::to_value(&presentation).unwrap())
            } else {
                Unverified::from(presentation)
            },
            media: Box::new(slidock_core_api::EmptyMediaStore),
            fonts: Vec::new(),
            features,
        })
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                0xedb8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A zip of `files` in order, every entry STORED.
pub fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, content) in files {
        let offset = out.len() as u32;
        let crc = crc32(content);
        let common = |header: &mut Vec<u8>| {
            header.extend(20u16.to_le_bytes()); // version needed
            header.extend(0u16.to_le_bytes()); // flags
            header.extend(0u16.to_le_bytes()); // STORED
            header.extend(0u16.to_le_bytes()); // time
            header.extend(0x21u16.to_le_bytes()); // date: 1980-01-01
            header.extend(crc.to_le_bytes());
            header.extend((content.len() as u32).to_le_bytes());
            header.extend((content.len() as u32).to_le_bytes());
            header.extend((name.len() as u16).to_le_bytes());
            header.extend(0u16.to_le_bytes()); // extra field
        };
        out.extend(0x0403_4b50u32.to_le_bytes());
        common(&mut out);
        out.extend(name.as_bytes());
        out.extend(*content);
        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend(20u16.to_le_bytes()); // made by
        common(&mut central);
        central.extend([0; 6]); // comment, disk, internal attributes
        central.extend(0u32.to_le_bytes()); // external attributes
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let directory_offset = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0; 4]);
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(directory_offset.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

fn put(table: &mut [u8], at: usize, bytes: &[u8]) {
    table[at..at + bytes.len()].copy_from_slice(bytes);
}

fn name_table(records: &[(u16, &str)]) -> Vec<u8> {
    let mut strings: Vec<u8> = Vec::new();
    let mut directory = Vec::new();
    for (id, value) in records {
        let bytes: Vec<u8> = value.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for field in [3, 1, 0x409, *id, bytes.len() as u16, strings.len() as u16] {
            directory.extend(field.to_be_bytes());
        }
        strings.extend(bytes);
    }
    let mut table = Vec::new();
    table.extend(0u16.to_be_bytes());
    table.extend((records.len() as u16).to_be_bytes());
    table.extend((6 + directory.len() as u16).to_be_bytes());
    table.extend(directory);
    table.extend(strings);
    table
}

/// A minimal TrueType file of one face: the tables the sfnt reader reads and those a browser's sanitizer requires (one
/// empty glyph, `A` in the `cmap`), with `ascender` as its hhea ascender.
pub fn font(family: &str, weight: u16, italic: bool, ascender: i16) -> Vec<u8> {
    font_with(family, weight, italic, ascender, Vec::new())
}

/// A `cmap` with one Windows full-repertoire format 12 subtable mapping each code point (sorted, distinct) to a glyph.
pub fn cmap(points: &[u32]) -> Vec<u8> {
    let mut subtable = Vec::new();
    subtable.extend(12u16.to_be_bytes());
    subtable.extend(0u16.to_be_bytes());
    subtable.extend(((16 + 12 * points.len()) as u32).to_be_bytes());
    subtable.extend(0u32.to_be_bytes());
    subtable.extend((points.len() as u32).to_be_bytes());
    for (glyph, point) in points.iter().enumerate() {
        subtable.extend(point.to_be_bytes());
        subtable.extend(point.to_be_bytes());
        subtable.extend((glyph as u32 + 1).to_be_bytes());
    }
    let mut table = Vec::new();
    for field in [0u16, 1, 3, 10] {
        table.extend(field.to_be_bytes());
    }
    table.extend(12u32.to_be_bytes());
    table.extend(subtable);
    table
}

/// [`font`] with more tables.
pub fn font_with(
    family: &str,
    weight: u16,
    italic: bool,
    ascender: i16,
    extra: Vec<([u8; 4], Vec<u8>)>,
) -> Vec<u8> {
    let mut head = vec![0; 54];
    put(&mut head, 18, &1000u16.to_be_bytes());
    put(
        &mut head,
        44,
        &(if italic { 0x02u16 } else { 0 }).to_be_bytes(),
    );
    let mut hhea = vec![0; 36];
    put(&mut hhea, 4, &ascender.to_be_bytes());
    put(&mut hhea, 6, &(-200i16).to_be_bytes());
    let mut os2 = vec![0; 96];
    put(&mut os2, 4, &weight.to_be_bytes());
    put(&mut os2, 62, &u16::from(italic).to_be_bytes());
    let style = if italic { "Italic" } else { "Regular" };
    let postscript = format!("{}-{style}", family.replace(' ', ""));
    let name = name_table(&[(1, family), (2, style), (6, &postscript)]);
    let mut tables = vec![
        (*b"OS/2", os2),
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"name", name),
        (*b"post", vec![0; 32]),
    ];
    let mut maxp = 0x0000_5000u32.to_be_bytes().to_vec();
    maxp.extend(1u16.to_be_bytes());
    let required = [
        (*b"maxp", maxp),
        (*b"cmap", cmap(&[0x41])),
        (*b"hmtx", vec![0; 4]),
        (*b"glyf", vec![0; 4]),
        (*b"loca", vec![0; 4]),
    ];
    for (tag, table) in required {
        if !extra.iter().any(|(extra, _)| *extra == tag) {
            tables.push((tag, table));
        }
    }
    tables.extend(extra);
    tables.sort_by_key(|(tag, _)| *tag);
    let mut out = Vec::new();
    out.extend(0x0001_0000u32.to_be_bytes());
    out.extend((tables.len() as u16).to_be_bytes());
    out.extend([0; 6]);
    let mut data = Vec::new();
    let start = 12 + tables.len() * 16;
    for (tag, bytes) in &tables {
        out.extend(tag);
        out.extend(0u32.to_be_bytes());
        out.extend(((start + data.len()) as u32).to_be_bytes());
        out.extend((bytes.len() as u32).to_be_bytes());
        data.extend(bytes);
        data.resize(data.len().next_multiple_of(4), 0);
    }
    out.extend(data);
    out
}
