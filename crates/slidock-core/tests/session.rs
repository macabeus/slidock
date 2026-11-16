//! The two-phase Session: the passes split at the font boundary give the bytes of one compile, the font loop, and
//! the state machine, whose misuse is an error, never a panic.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use serde_json::json;
use slidock_core::{
    CompileOptions, FontNeed, FontSource, FrontendRegistry, Phase, Pipeline, Session, SessionError,
    SessionOptions, VerifyMode, compile,
};
use slidock_core_api::{FrontendError, Input, Pass};
use slidock_fonts::FamilyKey;
use slidock_io::BytesReader;
use slidock_ir::diagnostics::{DiagnosticSite, Stage, codes, make_diagnostic};
use slidock_ir::fonts::FaceStyle;
use slidock_ir::verify::Invariant;
use support::*;

const FAKE: &[u8] = b"FAKE presentation";

fn registry(presentation: slidock_ir::source::Presentation) -> FrontendRegistry {
    FrontendRegistry::new(vec![Box::new(FakeFrontend::new(presentation))]).unwrap()
}

/// claim-all before fonts, these passes after.
fn pipeline(after: Vec<Pass>) -> Pipeline {
    Pipeline::new(vec![claim_all(&[])], after).unwrap()
}

fn open<'a>(options: SessionOptions<'a>, reader: &BytesReader<Vec<u8>>) -> Session<'a> {
    Session::open(&Input::new("presentation.fake", reader), options).unwrap()
}

fn reader() -> BytesReader<Vec<u8>> {
    BytesReader::new(FAKE.to_vec())
}

fn canonical(
    presentation: &Option<slidock_ir::resolved::Presentation>,
    diagnostics: &[slidock_ir::diagnostics::Diagnostic],
) -> String {
    slidock_ir::canonical::to_json(
        &json!({ "presentation": presentation, "diagnostics": diagnostics }),
    )
    .unwrap()
}

/// The passes before fonts and after them.
type Split = (Vec<Pass>, Vec<Pass>);

/// A need, as family and open faces.
type Brief = (String, Vec<(u32, FaceStyle)>);

fn brief(needs: &[FontNeed]) -> Vec<Brief> {
    needs
        .iter()
        .map(|need| {
            assert_eq!(need.key, FamilyKey::of(&need.family));
            let faces = need.faces.iter().map(|f| (f.weight, f.style)).collect();
            (need.family.clone(), faces)
        })
        .collect()
}

fn lato(faces: &[(u32, FaceStyle)]) -> Brief {
    ("Lato".into(), faces.to_vec())
}

/// The full presentation with a bold italic Lato run and a Georgia run on its hidden slide.
fn with_more_faces() -> slidock_ir::source::Presentation {
    let mut presentation = full_json();
    let props = &mut presentation["slides"][1]["elements"][0]["text"]["paragraphs"][0]["inlines"]
        [0]["props"];
    props["weight"] = json!(700);
    props["italic"] = json!(true);
    presentation["slides"][1]["elements"][1]["rows"][1]["cells"][0]["text"]["paragraphs"][0]["inlines"]
        [0]["props"]["font"]["family"] = json!("Georgia");
    typed(presentation)
}

#[test]
fn gives_the_bytes_of_one_compile_in_both_modes() {
    for verify in [VerifyMode::Each, VerifyMode::Boundaries] {
        // A rule broken after the fonts, and one broken before them.
        let cases: [fn() -> Split; 3] = [
            || (vec![claim_all(&[])], vec![identity("identity")]),
            || {
                (
                    vec![claim_all(&[Invariant::TextCascaded])],
                    vec![cascade_liar("ir-s2")],
                )
            },
            || {
                (
                    vec![claim_all(&[Invariant::TextCascaded]), cascade_liar("ir-s1")],
                    vec![identity("identity")],
                )
            },
        ];
        for (index, case) in cases.into_iter().enumerate() {
            let (before, after) = case();
            let names = format!("case {index}");
            let pipeline = Pipeline::new(before, after).unwrap();
            let registry = registry(full());
            let reader = reader();
            let mut session = open(
                SessionOptions {
                    verify,
                    ..SessionOptions::new(&registry, &pipeline)
                },
                &reader,
            );
            let finished = session.finish().unwrap();
            let options = CompileOptions {
                verify,
                ..CompileOptions::new(&registry, Phase::Resolved(pipeline.manager()))
            };
            let compiled = compile(&Input::new("presentation.fake", &reader), &options);
            assert_eq!(
                canonical(&finished.presentation, &finished.diagnostics),
                canonical(&compiled.presentation, &compiled.diagnostics),
                "{verify:?} {names:?}"
            );
            assert_eq!(finished.source, compiled.source);
            assert_eq!(finished.failures.len(), compiled.failures.len());
        }
    }
}

#[test]
fn lists_the_families_and_faces_the_text_uses() {
    let registry = registry(with_more_faces());
    let pipeline = pipeline(vec![identity("identity")]);
    let reader = reader();
    let session = open(SessionOptions::new(&registry, &pipeline), &reader);
    assert_eq!(
        brief(&session.font_needs()),
        [
            regular("Georgia"),
            lato(&[(400, FaceStyle::Normal), (700, FaceStyle::Italic)])
        ]
    );
}

/// A need for the regular face of `family`, used by nothing but text.
fn regular(family: &str) -> Brief {
    (family.into(), vec![(400, FaceStyle::Normal)])
}

#[test]
fn lists_the_families_of_fields_styled_breaks_paragraph_marks_and_list_labels() {
    let mut presentation = full_json();
    let paragraph = &mut presentation["slides"][0]["elements"][1]["text"]["paragraphs"][0];
    let props_of = |family: &str| {
        let mut props = paragraph["inlines"][0]["props"].clone();
        props["font"]["family"] = json!(family);
        props
    };
    let (field, line_break, end) = (props_of("Verdana"), props_of("Arial"), props_of("Tahoma"));
    let inlines = paragraph["inlines"].as_array_mut().unwrap();
    inlines.push(json!({ "kind": "field", "field": "slideNumber", "cached": "1", "props": field }));
    inlines.push(json!({ "kind": "break", "props": line_break }));
    paragraph["endProps"] = end;
    paragraph["props"]["bullet"] = json!({ "kind": "label", "text": "1.",
        "font": { "kind": "family", "family": "Courier New" }, "size": 18,
        "color": { "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 } });
    let registry = registry(typed(presentation));
    let pipeline = pipeline(vec![identity("identity")]);
    let reader = reader();
    let session = open(SessionOptions::new(&registry, &pipeline), &reader);
    assert_eq!(
        brief(&session.font_needs()),
        [
            regular("Arial"),
            regular("Courier New"),
            lato(&[(400, FaceStyle::Normal)]),
            regular("Tahoma"),
            regular("Verdana")
        ]
    );
}

#[test]
fn drops_the_faces_an_added_font_provides() {
    let registry = registry(with_more_faces());
    let pipeline = pipeline(vec![identity("identity")]);
    let reader = reader();
    let mut session = open(SessionOptions::new(&registry, &pipeline), &reader);
    assert_eq!(
        session.add_font(font("Lato", 400, false, 900), FontSource::Google),
        Ok(1)
    );
    let needs = brief(&session.font_needs());
    assert_eq!(needs.len(), 2);
    assert_eq!(needs[1], lato(&[(700, FaceStyle::Italic)]));
    session
        .add_font(font("Lato", 700, true, 900), FontSource::User)
        .unwrap();
    session
        .add_font(font("Georgia", 400, false, 900), FontSource::Local)
        .unwrap();
    assert_eq!(session.font_needs(), Vec::<FontNeed>::new());
    assert_eq!(session.added_fonts().len(), 3);
}

#[test]
fn lists_a_recorded_family_without_faces_until_any_face_of_it_is_added() {
    let mut presentation = full_json();
    presentation["fonts"]["roboto"] = json!({ "request": { "family": "Roboto", "faces": [], "postscriptNames": ["Roboto-Regular"], "embedded": true } });
    let registry = registry(typed(presentation));
    let pipeline = pipeline(vec![identity("identity")]);
    let reader = reader();
    let mut session = open(SessionOptions::new(&registry, &pipeline), &reader);
    let roboto = session
        .font_needs()
        .into_iter()
        .find(|need| need.family == "Roboto")
        .unwrap();
    assert!(roboto.faces.is_empty());
    assert!(roboto.embedded);
    assert_eq!(roboto.postscript_names, ["Roboto-Regular"]);
    session
        .add_font(font("Roboto", 500, false, 900), FontSource::Embedded)
        .unwrap();
    assert!(
        session
            .font_needs()
            .iter()
            .all(|need| need.family != "Roboto")
    );
}

#[test]
fn refuses_bytes_that_are_not_a_font_and_keeps_none_of_them() {
    let registry = registry(full());
    let pipeline = pipeline(vec![identity("identity")]);
    let reader = reader();
    let mut session = open(SessionOptions::new(&registry, &pipeline), &reader);
    let error = session
        .add_font(b"not a font".to_vec(), FontSource::User)
        .unwrap_err();
    assert!(
        matches!(error, SessionError::UnreadableFont(_)),
        "{error:?}"
    );
    assert!(session.added_fonts().is_empty());
    assert!(session.finish().is_ok());
}

#[test]
fn binds_the_passes_after_fonts_to_the_added_fonts_by_source_priority() {
    let seen: Rc<RefCell<Vec<Option<i32>>>> = Rc::default();
    let record = Rc::clone(&seen);
    let binder = presentation_pass("binder", move |presentation, ctx| {
        let ascender = ctx
            .fonts
            .get(&FamilyKey::of("Lato"))
            .and_then(|family| family.faces.first())
            .map(|face| face.metrics.hhea.ascender);
        record.borrow_mut().push(ascender);
        Ok(presentation.clone())
    });
    let registry = registry(full());
    let pipeline = pipeline(vec![binder]);
    let reader = reader();
    let mut session = open(SessionOptions::new(&registry, &pipeline), &reader);
    session.finish().unwrap();
    session
        .add_font(font("Lato", 400, false, 800), FontSource::Embedded)
        .unwrap();
    session
        .add_font(font("Lato", 400, false, 700), FontSource::User)
        .unwrap();
    session.finish().unwrap();
    // The embedded face wins over the dropped one, added later or not.
    assert_eq!(*seen.borrow(), [None, Some(800)]);
    let catalog = session.catalog();
    let face = &catalog.get(&FamilyKey::of("Lato")).unwrap().faces[0];
    assert_eq!(
        face.sha256,
        Some(slidock_ir::ids::Sha256::of(font("Lato", 400, false, 800)))
    );
    // The built-in metrics stay below the added fonts.
    assert!(catalog.get(&FamilyKey::of("Liberation Sans")).is_some());
}

#[test]
fn runs_the_passes_before_fonts_once() {
    let runs = Rc::new(Cell::new(0));
    let count = Rc::clone(&runs);
    let counter = slide_pass_with(
        info("counter", &[], &Invariant::RESOLVED),
        move |view, _| {
            if view.container.id().as_str() == "ir-s1" {
                count.set(count.get() + 1);
            }
            Ok(view.unchanged())
        },
    );
    let pipeline = Pipeline::new(vec![counter], vec![identity("identity")]).unwrap();
    let registry = registry(full());
    let reader = reader();
    let mut session = open(SessionOptions::new(&registry, &pipeline), &reader);
    assert_eq!(runs.get(), 1);
    for _ in 0..3 {
        session.finish().unwrap();
    }
    assert_eq!(runs.get(), 1);
}

#[test]
fn finish_runs_again_with_the_fonts_added_since() {
    let registry = registry(full());
    let pipeline = pipeline(vec![identity("identity")]);
    let reader = reader();
    let mut session = open(SessionOptions::new(&registry, &pipeline), &reader);
    assert!(!session.is_finished());
    let first = session.finish().unwrap();
    assert!(first.presentation.is_some());
    assert!(session.is_finished());
    let again = session.finish().unwrap();
    assert_eq!(again, first);
    session
        .add_font(font("Lato", 400, false, 900), FontSource::User)
        .unwrap();
    assert_eq!(session.font_needs().len(), 0);
    let second = session.finish().unwrap();
    assert_eq!(
        canonical(&second.presentation, &second.diagnostics),
        canonical(&first.presentation, &first.diagnostics)
    );
    assert_eq!(session.resume(), Ok(()));
}

#[test]
fn a_presentation_that_fails_before_fonts_finishes_without_a_presentation() {
    let failing = presentation_pass_with(info("failing", &[], &Invariant::RESOLVED), |_, _| {
        Err(slidock_core_api::PassError::failed("before fonts"))
    });
    let pipeline = Pipeline::new(vec![failing], vec![identity("identity")]).unwrap();
    let registry = registry(full());
    let reader = reader();
    let mut session = open(SessionOptions::new(&registry, &pipeline), &reader);
    assert!(session.before_fonts().is_none());
    assert_eq!(session.font_needs(), Vec::<FontNeed>::new());
    assert!(find(&session.diagnostics(), "IR0907").is_some());
    let finished = session.finish().unwrap();
    assert!(finished.presentation.is_none());
    assert!(finished.source.is_some());
    let failed = find(&finished.diagnostics, "IR0907").unwrap();
    assert_eq!(data(failed, "reason").as_deref(), Some("before fonts"));
    let compiled = compile(
        &Input::new("presentation.fake", &reader),
        &CompileOptions::new(&registry, Phase::Resolved(pipeline.manager())),
    );
    assert_eq!(
        canonical(&finished.presentation, &finished.diagnostics),
        canonical(&compiled.presentation, &compiled.diagnostics)
    );
}

#[test]
fn a_presentation_that_fails_verify_source_finishes_without_one() {
    let mut presentation = full_json();
    presentation["slides"][1]["id"] = json!("ir-s1");
    let frontend = FakeFrontend::new(full()).json();
    // The frontend hands on JSON with a duplicate slide id; analysis still sees the presentation it was built from.
    let frontend = FakeFrontendWith {
        inner: frontend,
        presentation,
    };
    let registry = FrontendRegistry::new(vec![Box::new(frontend)]).unwrap();
    let pipeline = pipeline(vec![identity("identity")]);
    let reader = reader();
    let mut session = open(SessionOptions::new(&registry, &pipeline), &reader);
    assert!(session.before_fonts().is_none());
    let finished = session.finish().unwrap();
    assert!(finished.source.is_none());
    assert!(finished.presentation.is_none());
    assert!(find(&finished.diagnostics, "IR0907").is_some());
}

#[test]
fn refuses_to_open_what_compile_refuses() {
    let registry = registry(full());
    let pipeline = pipeline(vec![identity("identity")]);
    let unread = BytesReader::new(b"nope".to_vec());
    let error = Session::open(
        &Input::new("x.bin", &unread),
        SessionOptions::new(&registry, &pipeline),
    )
    .unwrap_err();
    assert_eq!(error.refusal.unwrap().code, codes::IR0902);
    assert_eq!(error.diagnostics.len(), 1);

    let only = [id("ir-nope")];
    let error = Session::open(
        &Input::new("presentation.fake", &reader()),
        SessionOptions {
            only: Some(&only),
            ..SessionOptions::new(&registry, &pipeline)
        },
    )
    .unwrap_err();
    assert_eq!(error.refusal.unwrap().code, codes::IR0913);

    let crashing =
        FakeFrontend::new(full()).before_lower(|_| Err(FrontendError::Crashed("lower bug".into())));
    let registry = FrontendRegistry::new(vec![Box::new(crashing)]).unwrap();
    let error = Session::open(
        &Input::new("presentation.fake", &reader()),
        SessionOptions::new(&registry, &pipeline),
    )
    .unwrap_err();
    assert!(error.refusal.is_none());
    assert_eq!(
        error
            .diagnostics
            .iter()
            .map(|d| d.code.code())
            .collect::<Vec<_>>(),
        ["IR0906"]
    );
}

#[test]
fn keeps_what_the_frontend_handed_on() {
    let registry = registry(full());
    let pipeline = pipeline(vec![identity("identity")]);
    let reader = reader();
    let only = [id("ir-s1")];
    let session = open(
        SessionOptions {
            only: Some(&only),
            ..SessionOptions::new(&registry, &pipeline)
        },
        &reader,
    );
    assert_eq!(session.provenance().name, "presentation.fake");
    assert_eq!(session.facts().slides.len(), 2);
    assert_eq!(session.analysis().slides.len(), 2);
    assert_eq!(session.before_fonts().unwrap().slides.len(), 1);
    assert!(session.embedded_fonts().is_empty());
    assert!(session.features().contains_key("presentation"));
    assert!(
        session
            .media(&slidock_ir::ids::Sha256::of(b"photo"))
            .is_none()
    );
    let frontend = session
        .diagnostics()
        .into_iter()
        .find(|d| d.stage == Stage::Frontend)
        .unwrap();
    assert_eq!(frontend.emitter, "pptx");
    let expected = make_diagnostic(
        codes::IR0905,
        Stage::Frontend,
        "pptx",
        DiagnosticSite::new()
            .data("stage", "note")
            .data("reason", "frontend diagnostics are stamped"),
    );
    assert_eq!(frontend, expected);
}

/// A fake frontend whose `lower` hands on `presentation` as JSON instead of its own.
struct FakeFrontendWith {
    inner: FakeFrontend,
    presentation: serde_json::Value,
}

impl slidock_core_api::Frontend for FakeFrontendWith {
    fn format(&self) -> slidock_ir::formats::SourceFormat {
        self.inner.format()
    }

    fn version(&self) -> &str {
        self.inner.version()
    }

    fn sniff(&self, head: &[u8], hints: &slidock_core_api::SniffHints<'_>) -> bool {
        self.inner.sniff(head, hints)
    }

    fn analyze(
        &self,
        input: &Input<'_>,
        ctx: &mut slidock_core_api::FrontendCtx<'_>,
    ) -> Result<slidock_core_api::Analysis, FrontendError> {
        self.inner.analyze(input, ctx)
    }

    fn lower(
        &self,
        input: &Input<'_>,
        ctx: &mut slidock_core_api::FrontendCtx<'_>,
        options: &slidock_core_api::LowerOptions<'_>,
    ) -> Result<slidock_core_api::LowerResult, FrontendError> {
        let mut lowered = self.inner.lower(input, ctx, options)?;
        lowered.presentation = slidock_core_api::Unverified::Json(self.presentation.clone());
        Ok(lowered)
    }
}
