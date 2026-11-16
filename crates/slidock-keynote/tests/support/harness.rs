//! Runs the Keynote frontend on bytes: lowering alone, or the whole compile through the passes and the HTML backend.

use serde_json::Value;
use slidock_core::{FrontendRegistry, PassRegistry, Pipeline, Session, SessionOptions};
use slidock_core_api::{
    Analysis, ContainerOrder, Deadline, Frontend, FrontendCtx, FrontendError, Input, Limits,
    LowerOptions, LowerResult, Sink, Unverified, sort_diagnostics,
};
use slidock_html::{Documents, EmitOptions, Provenance as HtmlProvenance, emit_documents};
use slidock_io::{BytesReader, RangeReader};
use slidock_ir::diagnostics::{Diagnostic, Severity, Stage};
use slidock_ir::ids::SlideId;
use slidock_ir::{resolved, source};
use slidock_keynote::KeynoteFrontend;

pub const NAME: &str = "presentation.key";

/// A frontend context over a sink, with no deadline.
pub fn with_ctx<T>(
    limits: &Limits,
    run: impl FnOnce(&mut FrontendCtx<'_>) -> T,
) -> (T, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    let mut sink = Sink::new(&mut diagnostics, Stage::Frontend, "keynote");
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
    with_ctx(&Limits::default(), |ctx| {
        KeynoteFrontend.analyze(&input, ctx)
    })
    .0
}

/// The code of a refusal, `None` when the input is not refused.
pub fn refusal(bytes: &[u8]) -> Option<String> {
    match analyze(bytes) {
        Err(FrontendError::Refused(diagnostic)) => Some(diagnostic.code.code().to_owned()),
        Ok(_) => None,
        Err(other) => panic!("not refused: {other:?}"),
    }
}

pub struct Lowered {
    pub analysis: Analysis,
    pub lowered: LowerResult,
    pub presentation: source::Presentation,
    /// The presentation as canonical JSON, for reading it as the TypeScript tests do.
    pub json: Value,
    pub diagnostics: Vec<Diagnostic>,
}

impl Lowered {
    /// The `index`th slide's elements, as JSON.
    pub fn elements(&self, slide: usize) -> &Vec<Value> {
        self.json["slides"][slide]["elements"].as_array().unwrap()
    }

    pub fn codes(&self) -> Vec<&str> {
        self.diagnostics.iter().map(|d| d.code.code()).collect()
    }

    pub fn features(&self, container: &str) -> Vec<&'static str> {
        self.lowered.features[container]
            .iter()
            .map(|f| f.as_str())
            .collect()
    }
}

pub fn lower(bytes: &[u8]) -> Lowered {
    lower_with(bytes, None, Limits::default())
}

pub fn lower_reader(reader: &dyn RangeReader, only: Option<&[SlideId]>, limits: Limits) -> Lowered {
    lower_reader_by(&KeynoteFrontend, reader, only, limits)
}

/// Lowers through `frontend` on the parity path.
pub fn lower_reader_by(
    frontend: &dyn Frontend,
    reader: &dyn RangeReader,
    only: Option<&[SlideId]>,
    limits: Limits,
) -> Lowered {
    lower_reader_as(frontend, reader, only, limits, false)
}

/// Lowers on the deck path ([`LowerOptions::native_motion`]): the native motion readings apply and their
/// diagnostics are reported.
pub fn lower_native(bytes: &[u8]) -> Lowered {
    lower_native_reader(&BytesReader::new(bytes), None)
}

/// [`lower_native`] over a reader, for some slides only.
pub fn lower_native_reader(reader: &dyn RangeReader, only: Option<&[SlideId]>) -> Lowered {
    lower_reader_as(&KeynoteFrontend, reader, only, Limits::default(), true)
}

fn lower_reader_as(
    frontend: &dyn Frontend,
    reader: &dyn RangeReader,
    only: Option<&[SlideId]>,
    limits: Limits,
    native_motion: bool,
) -> Lowered {
    let input = Input::new(NAME, reader);
    let ((analysis, lowered), diagnostics) = with_ctx(&limits, |ctx| {
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
    let diagnostics = sort_diagnostics(diagnostics, &ContainerOrder::of(Some(&presentation)));
    // Through canonical JSON, so numbers compare as JSON reads them.
    let text =
        slidock_canon::to_canonical_json(&presentation, slidock_canon::Layout::Compact).unwrap();
    let json = serde_json::from_str(&text).unwrap();
    Lowered {
        analysis,
        lowered,
        presentation,
        json,
        diagnostics,
    }
}

pub fn lower_with(bytes: &[u8], only: Option<&[SlideId]>, limits: Limits) -> Lowered {
    lower_reader(&BytesReader::new(bytes), only, limits)
}

/// What a golden file holds: the source presentation as the parity path keeps it (without the native motion
/// data), the frontend's diagnostics and the features it saw. The input's hash is left out: the synthetic zips
/// carry the time they were written.
pub fn golden_bytes(lowered: &Lowered) -> String {
    let mut provenance = serde_json::to_value(&lowered.lowered.provenance).unwrap();
    provenance.as_object_mut().unwrap().remove("sha256");
    let mut presentation = lowered.presentation.clone();
    slidock_ir::effects::strip_native_motion(&mut presentation);
    let value = serde_json::json!({
        "presentation": presentation,
        "diagnostics": lowered.diagnostics,
        "features": lowered.lowered.features,
        "provenance": provenance,
    });
    slidock_canon::to_canonical_json(&value, slidock_canon::Layout::Indented).unwrap()
}

/// The frontends and the standard pipeline over every pass.
pub fn setup() -> (FrontendRegistry, Pipeline) {
    let registry = FrontendRegistry::new(vec![Box::new(KeynoteFrontend)]).unwrap();
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
    pub fn errors(&self) -> Vec<&Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect()
    }
}

pub fn compile(bytes: &[u8], name: &str) -> Compiled {
    let (registry, pipeline) = setup();
    let reader = BytesReader::new(bytes);
    let input = Input::new(name, &reader);
    let mut session = Session::open(&input, SessionOptions::new(&registry, &pipeline)).unwrap();
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

/// A slide with the layout it shows, as canonical JSON.
fn chain(presentation: &source::Presentation, id: &SlideId) -> String {
    let slide = presentation.slides.iter().find(|slide| &slide.id == id);
    let layout = slide
        .and_then(|slide| slide.layout.as_ref())
        .and_then(|layout| presentation.layouts.get(layout));
    let value = serde_json::json!({ "slide": slide, "layout": layout });
    slidock_canon::to_canonical_json(&value, slidock_canon::Layout::Compact).unwrap()
}

/// What every fixture is checked for besides its golden: the slides lowered are the ones analyze lists, lowering twice
/// gives the same bytes, each slide lowered alone is exactly as in the whole presentation, and the compile is
/// error-free and lint-clean.
pub fn check_fixture(name: &str, bytes: &[u8]) {
    let first = lower(bytes);
    let lowered: Vec<(&str, bool)> = first
        .presentation
        .slides
        .iter()
        .map(|slide| (slide.id.as_str(), slide.hidden))
        .collect();
    let analyzed: Vec<(&str, bool)> = first
        .analysis
        .slides
        .iter()
        .map(|slide| (slide.id.as_str(), slide.hidden))
        .collect();
    assert_eq!(lowered, analyzed, "{name}");
    assert_eq!(golden_bytes(&lower(bytes)), golden_bytes(&first), "{name}");

    for slide in &first.presentation.slides {
        let only = [slide.id.clone()];
        let alone = lower_with(bytes, Some(&only), Limits::default());
        let ids: Vec<&SlideId> = alone.presentation.slides.iter().map(|s| &s.id).collect();
        assert_eq!(ids, [&slide.id], "{name}");
        assert_eq!(
            chain(&alone.presentation, &slide.id),
            chain(&first.presentation, &slide.id),
            "{name}: {}",
            slide.id.as_str()
        );
        assert_eq!(
            alone.lowered.features.get(slide.id.as_str()),
            first.lowered.features.get(slide.id.as_str()),
            "{name}: {}",
            slide.id.as_str()
        );
    }

    let out = compile(bytes, &format!("{name}.key"));
    let errors: Vec<&str> = out.errors().iter().map(|d| d.code.code()).collect();
    assert_eq!(errors, Vec::<&str>::new(), "{name}");
    let documents = out.documents.expect("documents");
    assert_eq!(documents.lint(), vec![], "{name}");
}
