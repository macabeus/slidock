//! Runs the ODP frontend on bytes: lowering alone, or the whole compile through the passes and the HTML backend.
#![allow(dead_code, reason = "each test binary uses part of the support")]

pub mod fixtures;
pub mod odp;

use serde_json::Value;
use slidock_core::{FrontendRegistry, PassRegistry, Pipeline, Session, SessionOptions};
use slidock_core_api::{
    Analysis, ContainerOrder, Deadline, Frontend, FrontendCtx, FrontendError, Input, Limits,
    LowerOptions, LowerResult, Sink, Unverified, sort_diagnostics,
};
use slidock_html::{Documents, EmitOptions, Provenance as HtmlProvenance, emit_documents};
use slidock_io::BytesReader;
use slidock_ir::diagnostics::{Diagnostic, Severity, Stage};
use slidock_ir::ids::SlideId;
use slidock_ir::{resolved, source};
use slidock_odp::OdpFrontend;

pub const NAME: &str = "presentation.odp";

/// A frontend context over a sink, with no deadline.
pub fn with_ctx<T>(
    limits: &Limits,
    run: impl FnOnce(&mut FrontendCtx<'_>) -> T,
) -> (T, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    let mut sink = Sink::new(&mut diagnostics, Stage::Frontend, "odp");
    let mut ctx = FrontendCtx {
        limits,
        diag: &mut sink,
        deadline: Deadline::never(),
        spent: slidock_core_api::Output::default(),
    };
    let out = run(&mut ctx);
    (out, diagnostics)
}

pub fn analyze(bytes: &[u8]) -> Result<Analysis, FrontendError> {
    let reader = BytesReader::new(bytes);
    let input = Input::new(NAME, &reader);
    with_ctx(&Limits::default(), |ctx| OdpFrontend.analyze(&input, ctx)).0
}

/// The refusal of an input, which must be refused.
pub fn refusal(bytes: &[u8]) -> Diagnostic {
    match analyze(bytes) {
        Err(FrontendError::Refused(diagnostic)) => *diagnostic,
        other => panic!("not refused: {other:?}"),
    }
}

pub struct Lowered {
    pub analysis: Analysis,
    pub lowered: LowerResult,
    pub presentation: source::Presentation,
    /// The presentation as JSON, for reading it as the TypeScript tests do.
    pub json: Value,
    pub diagnostics: Vec<Diagnostic>,
}

impl Lowered {
    /// The `index`th slide's elements, as JSON.
    pub fn elements(&self, slide: usize) -> &Vec<Value> {
        self.json["slides"][slide]["elements"].as_array().unwrap()
    }

    /// The element named `name` on a slide.
    pub fn named(&self, slide: usize, name: &str) -> &Value {
        by_name(self.elements(slide), name)
    }

    pub fn codes(&self) -> Vec<(String, Option<String>)> {
        self.diagnostics
            .iter()
            .map(|d| {
                (
                    d.code.code().to_owned(),
                    d.container.as_ref().map(|c| c.as_str().to_owned()),
                )
            })
            .collect()
    }

    pub fn features(&self, container: &str) -> Vec<&'static str> {
        self.lowered.features[container]
            .iter()
            .map(|f| f.as_str())
            .collect()
    }
}

/// A value as JSON with JavaScript's numbers: integral values are integers, as canonical JSON prints them, so they
/// compare with `json!` literals.
pub fn js<T: serde::Serialize + ?Sized>(value: &T) -> Value {
    let text = slidock_canon::to_canonical_json(value, slidock_canon::Layout::Compact).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// Whether `actual` holds `pattern` as Vitest's `toMatchObject` reads it: every member of an object pattern matches
/// the actual's, arrays match element by element, anything else is equal.
pub fn holds(actual: &Value, pattern: &Value) -> bool {
    match (actual, pattern) {
        (Value::Object(a), Value::Object(p)) => p
            .iter()
            .all(|(key, want)| a.get(key).is_some_and(|got| holds(got, want))),
        (Value::Array(a), Value::Array(p)) => {
            a.len() == p.len() && a.iter().zip(p).all(|(x, y)| holds(x, y))
        }
        _ => js(actual) == js(pattern),
    }
}

/// Asserts that `actual` holds `pattern` (`toMatchObject`).
#[track_caller]
pub fn assert_holds(actual: &Value, pattern: Value) {
    assert!(
        holds(actual, &pattern),
        "\n{actual:#}\ndoes not hold\n{pattern:#}"
    );
}

pub fn by_name<'v>(elements: &'v [Value], name: &str) -> &'v Value {
    elements
        .iter()
        .find(|element| element["name"] == name)
        .unwrap_or_else(|| panic!("no {name}"))
}

pub fn lower(bytes: &[u8]) -> Lowered {
    lower_with(bytes, None, Limits::default())
}

pub fn lower_with(bytes: &[u8], only: Option<&[SlideId]>, limits: Limits) -> Lowered {
    lower_by(false, bytes, only, limits)
}

/// Lowers with native motion: transitions and build animations in the effect vocabulary.
pub fn lower_motion(bytes: &[u8]) -> Lowered {
    lower_by(true, bytes, None, Limits::default())
}

pub fn lower_by(
    native_motion: bool,
    bytes: &[u8],
    only: Option<&[SlideId]>,
    limits: Limits,
) -> Lowered {
    let frontend = OdpFrontend;
    let reader = BytesReader::new(bytes);
    let input = Input::new(NAME, &reader);
    let ((analysis, lowered), mut diagnostics) = with_ctx(&limits, |ctx| {
        let analysis = frontend.analyze(&input, ctx).unwrap();
        let facts = analysis.facts();
        let lowered = frontend
            .lower(
                &input,
                ctx,
                &LowerOptions {
                    facts: &facts,
                    only,
                    native_motion,
                },
            )
            .unwrap();
        (analysis, lowered)
    });
    let Unverified::Typed(presentation) = &lowered.presentation else {
        panic!("a typed presentation");
    };
    let presentation = (**presentation).clone();
    diagnostics = sort_diagnostics(diagnostics, &ContainerOrder::of(Some(&presentation)));
    let json = js(&presentation);
    Lowered {
        analysis,
        lowered,
        presentation,
        json,
        diagnostics,
    }
}

/// What a golden file holds: the source presentation, the frontend's diagnostics and the features it saw. The
/// input's hash is left out (the zips carry the time they were written), and its size is the golden's: the test
/// zips' compressed sizes depend on the deflate encoder that wrote them.
pub fn golden_bytes(lowered: &Lowered, golden_bytes: Option<u64>) -> String {
    let mut provenance = serde_json::to_value(&lowered.lowered.provenance).unwrap();
    let object = provenance.as_object_mut().unwrap();
    object.remove("sha256");
    if let Some(bytes) = golden_bytes {
        object.insert("bytes".to_owned(), Value::from(bytes));
    }
    let value = serde_json::json!({
        "presentation": lowered.presentation,
        "diagnostics": lowered.diagnostics,
        "features": lowered.lowered.features,
        "provenance": provenance,
    });
    slidock_canon::to_canonical_json(&value, slidock_canon::Layout::Indented).unwrap()
}

/// The frontends and the standard pipeline over every pass.
pub fn setup() -> (FrontendRegistry, Pipeline) {
    let registry = FrontendRegistry::new(vec![Box::new(OdpFrontend)]).unwrap();
    let mut passes = PassRegistry::new();
    for pass in slidock_passes::passes() {
        passes.register(pass).unwrap();
    }
    (registry, Pipeline::standard(passes).unwrap())
}

/// A whole compile: the resolved presentation, its documents and every diagnostic.
pub struct Compiled {
    pub presentation: Option<resolved::Presentation>,
    pub documents: Option<Documents>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Compiled {
    pub fn errors(&self) -> Vec<&str> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| d.code.code())
            .collect()
    }
}

pub fn compile(bytes: &[u8]) -> Compiled {
    compile_by(false, bytes)
}

/// A whole compile with native motion: the session reads the motion natively and keeps its data.
pub fn compile_motion(bytes: &[u8]) -> Compiled {
    compile_by(true, bytes)
}

fn compile_by(native_motion: bool, bytes: &[u8]) -> Compiled {
    let (registry, pipeline) = setup();
    let reader = BytesReader::new(bytes);
    let input = Input::new(NAME, &reader);
    let mut options = SessionOptions::new(&registry, &pipeline);
    options.native_motion = native_motion;
    let mut session = Session::open(&input, options).unwrap();
    let finished = session.finish().unwrap();
    let mut diagnostics = finished.diagnostics.clone();
    let documents = finished.presentation.as_ref().map(|presentation| {
        let provenance = session.provenance();
        let emit = EmitOptions::new(HtmlProvenance {
            format: provenance.format.as_str().to_owned(),
            name: provenance.name.clone(),
            sha256: provenance.sha256.to_hex(),
            bytes: provenance.bytes,
        });
        let emitted = emit_documents(presentation, &emit).unwrap();
        diagnostics.extend(emitted.diagnostics);
        emitted.files
    });
    Compiled {
        presentation: finished.presentation,
        documents,
        diagnostics,
    }
}
