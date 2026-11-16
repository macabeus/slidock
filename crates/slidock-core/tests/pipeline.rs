//! `compile` and `analyze` over fake frontends: sniffing, limits, `only`, refusals, crashes, timeouts, Foreign
//! diagnostics, stats and traces, and the pass order of a pipeline.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;
use slidock_core::{
    AFTER_FONTS, BEFORE_FONTS, CompileOptions, CompileResult, FrontendChoice, FrontendRegistry,
    PassManager, PassRegistry, Phase, Pipeline, PipelineError, VerifyMode, analyze, compile,
};
use slidock_core_api::{Clock, FrontendError, Input, Limits, PresentationFacts};
use slidock_io::BytesReader;
use slidock_ir::diagnostics::{DiagnosticSite, Stage, codes, make_diagnostic};
use slidock_ir::formats::{FileFormat, SourceFormat};
use slidock_ir::verify::Invariant;
use support::*;

const FAKE: &[u8] = b"FAKE presentation";
const PPTX: SourceFormat = SourceFormat::File(FileFormat::Pptx);

fn registry(frontend: FakeFrontend) -> FrontendRegistry {
    FrontendRegistry::new(vec![Box::new(frontend)]).unwrap()
}

fn claim() -> PassManager {
    manager(vec![claim_all(&[])])
}

/// A registry of one frontend and the `claim-all` passes.
struct Setup {
    registry: FrontendRegistry,
    passes: PassManager,
}

impl Setup {
    fn new(frontend: FakeFrontend) -> Self {
        Setup {
            registry: registry(frontend),
            passes: claim(),
        }
    }

    fn options(&self) -> CompileOptions<'_> {
        CompileOptions::new(&self.registry, Phase::Resolved(&self.passes))
    }
}

fn compile_bytes(options: &CompileOptions<'_>, bytes: &[u8], name: &str) -> CompileResult {
    let reader = BytesReader::new(bytes.to_vec());
    compile(&Input::new(name, &reader), options)
}

/// Compiles `bytes` named `name` in the resolved phase with `claim-all`, after `configure` adjusts the options.
fn compile_with(
    frontend: FakeFrontend,
    bytes: &[u8],
    name: &str,
    configure: impl FnOnce(&mut CompileOptions<'_>),
) -> CompileResult {
    let setup = Setup::new(frontend);
    let mut options = setup.options();
    configure(&mut options);
    compile_bytes(&options, bytes, name)
}

fn compile_fake(
    frontend: FakeFrontend,
    configure: impl FnOnce(&mut CompileOptions<'_>),
) -> CompileResult {
    compile_with(frontend, FAKE, "presentation.fake", configure)
}

fn source_phase(options: &mut CompileOptions<'_>) {
    options.phase = Phase::Source;
}

fn diagnostic_codes(result: &CompileResult) -> Vec<&str> {
    result.diagnostics.iter().map(|d| d.code.code()).collect()
}

#[test]
fn sniffs_the_frontend_lowers_and_resolves() {
    let result = compile_fake(FakeFrontend::new(full()), |_| {});
    assert!(result.presentation.is_some());
    assert_eq!(result.provenance.as_ref().unwrap().format, PPTX);
    assert!(result.refusal.is_none());
    let frontend = result
        .diagnostics
        .iter()
        .find(|d| d.stage == Stage::Frontend)
        .unwrap();
    assert_eq!(frontend.emitter, "pptx");
}

#[test]
fn reads_a_json_presentation_as_a_typed_one() {
    let typed = compile_fake(FakeFrontend::new(full()), |_| {});
    let json = compile_fake(FakeFrontend::new(full()).json(), |_| {});
    let canonical = |result: &CompileResult| {
        slidock_ir::canonical::to_json(
            &json!({ "presentation": result.presentation, "diagnostics": result.diagnostics }),
        )
        .unwrap()
    };
    assert_eq!(canonical(&json), canonical(&typed));
}

#[test]
fn stops_after_verify_source_in_the_source_phase() {
    let result = compile_fake(FakeFrontend::new(full()), source_phase);
    assert!(result.presentation.is_none());
    assert_eq!(result.source.unwrap().slides.len(), 2);
}

#[test]
fn refuses_inputs_no_frontend_reads_and_too_large_ones_before_reading_them() {
    let unread = compile_with(FakeFrontend::new(full()), b"nope", "x.bin", |_| {});
    let refusal = unread.refusal.unwrap();
    assert_eq!(refusal.code, codes::IR0902);
    assert_eq!(data(&refusal, "name").as_deref(), Some("x.bin"));
    assert_eq!(refusal.message, "x.bin is not an input any frontend reads");
    let too_large = compile_fake(FakeFrontend::new(full()), |options| {
        options.limits = Limits {
            input_bytes: 3,
            ..Limits::default()
        };
    });
    assert_eq!(too_large.refusal.unwrap().code, codes::IR0903);
}

#[test]
fn gives_zip_entry_names_to_sniffing() {
    let names: Rc<RefCell<Option<Vec<String>>>> = Rc::default();
    let seen = Rc::clone(&names);
    let frontend = FakeFrontend::new(minimal()).with_sniff(move |_, hints| {
        *seen.borrow_mut() = hints.zip_names.map(<[String]>::to_vec);
        true
    });
    let zip = stored_zip(&[("ppt/presentation.xml", b"<p/>")]);
    compile_with(frontend, &zip, "deck.pptx", source_phase);
    assert_eq!(
        names.borrow().clone(),
        Some(vec!["ppt/presentation.xml".to_owned()])
    );

    // A broken zip sniffs on its head alone.
    let seen = Rc::clone(&names);
    let frontend = FakeFrontend::new(minimal()).with_sniff(move |head, hints| {
        *seen.borrow_mut() = hints.zip_names.map(<[String]>::to_vec);
        head.starts_with(b"PK")
    });
    let result = compile_with(frontend, &zip[..20], "deck.pptx", source_phase);
    assert_eq!(*names.borrow(), None);
    assert!(result.source.is_some());
}

#[test]
fn honours_a_forced_frontend() {
    let result = compile_with(FakeFrontend::new(minimal()), b"anything", "x", |options| {
        options.frontend = FrontendChoice::Format(PPTX);
        options.phase = Phase::Source;
    });
    assert!(result.source.is_some());
    let missing = compile_fake(FakeFrontend::new(minimal()), |options| {
        options.frontend = FrontendChoice::Format(SourceFormat::File(FileFormat::Odp));
    });
    assert_eq!(missing.refusal.unwrap().code, codes::IR0902);
}

#[test]
fn refuses_presentations_over_the_slide_limit_before_lowering() {
    let frontend = FakeFrontend::new(full());
    let calls = Rc::clone(&frontend.calls);
    let result = compile_fake(frontend, |options| {
        options.limits = Limits {
            slides: 1,
            ..Limits::default()
        };
    });
    let refusal = result.refusal.unwrap();
    assert_eq!(refusal.code, codes::IR0904);
    assert_eq!(
        (data(&refusal, "count"), data(&refusal, "limit")),
        (Some("2".into()), Some("1".into()))
    );
    assert!(calls.borrow().is_empty());
}

#[test]
fn passes_presentation_wide_facts_and_the_slide_selection_to_lower() {
    let frontend = FakeFrontend::new(full());
    let calls = Rc::clone(&frontend.calls);
    let only = [id("ir-s2")];
    let setup = Setup::new(frontend);
    let options = CompileOptions {
        only: Some(&only),
        ..setup.options()
    };
    let result = compile_bytes(&options, FAKE, "presentation.fake");
    let facts: PresentationFacts = serde_json::from_value(json!({
        "size": { "w": 720, "h": 405 },
        "slides": [{ "id": "ir-s1", "hidden": false }, { "id": "ir-s2", "hidden": true }],
        "mediaBytes": 0
    }))
    .unwrap();
    assert_eq!(
        *calls.borrow(),
        [LowerCall {
            facts,
            only: Some(vec![id("ir-s2")])
        }]
    );
    let presentation = result.presentation.unwrap().to_source().unwrap();
    assert_eq!(
        presentation
            .slides
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["ir-s2"]
    );
    // Its link to ir-s1, outside the selection, resolves against the whole show order.
    assert!(presentation.slides[0].failed.is_none());
}

#[test]
fn refuses_an_unknown_slide_in_only_and_only_on_a_presentation_over_its_media_budget() {
    let unknown = [id("ir-nope")];
    let setup = Setup::new(FakeFrontend::new(full()));
    let result = compile_bytes(
        &CompileOptions {
            only: Some(&unknown),
            ..setup.options()
        },
        FAKE,
        "p",
    );
    assert_eq!(result.refusal.unwrap().code, codes::IR0913);
    let heavy = || {
        FakeFrontend::new(full()).with_analyze(|presentation, _| {
            Ok(slidock_core_api::Analysis {
                media_bytes: 3 * 1024 * 1024 * 1024,
                ..analysis_of(presentation)
            })
        })
    };
    let only = [id("ir-s1")];
    let setup = Setup::new(heavy());
    let result = compile_bytes(
        &CompileOptions {
            only: Some(&only),
            ..setup.options()
        },
        FAKE,
        "p",
    );
    assert_eq!(result.refusal.unwrap().code, codes::IR0912);
    assert!(compile_fake(heavy(), |_| {}).refusal.is_none());
}

#[test]
fn turns_refusals_crashes_and_timeouts_into_diagnostics() {
    let refusal = make_diagnostic(
        codes::IR0902,
        Stage::Frontend,
        "pptx",
        DiagnosticSite::new().data("name", "x"),
    );
    let refused = refusal.clone();
    let refusing = FakeFrontend::new(minimal())
        .before_lower(move |_| Err(FrontendError::refused(refused.clone())));
    assert_eq!(compile_fake(refusing, |_| {}).refusal, Some(refusal));

    let crashing = FakeFrontend::new(minimal())
        .with_analyze(|_, _| Err(FrontendError::Crashed("parser bug".into())));
    let crashed = compile_fake(crashing, |_| {});
    assert_eq!(diagnostic_codes(&crashed), ["IR0906"]);
    let d = &crashed.diagnostics[0];
    assert_eq!(
        (data(d, "format"), data(d, "stage"), data(d, "error")),
        (
            Some("pptx".into()),
            Some("analyze".into()),
            Some("parser bug".into())
        )
    );
    assert!(crashed.refusal.is_none());

    let clock = Rc::new(Ticks::new(0.0));
    let ticks = Rc::clone(&clock);
    let slow = FakeFrontend::new(minimal()).before_lower(move |ctx| {
        ticks.advance(30.0);
        ctx.deadline.check()?;
        Err(FrontendError::Crashed("unreachable".into()))
    });
    let setup = Setup::new(slow);
    let options = CompileOptions {
        limits: Limits {
            lower_ms: 5,
            ..Limits::default()
        },
        clock: Some(&*clock as &dyn Clock),
        ..setup.options()
    };
    let timed_out = compile_bytes(&options, FAKE, "p");
    assert_eq!(diagnostic_codes(&timed_out), ["IR0905"]);
    assert_eq!(
        data(&timed_out.diagnostics[0], "stage").as_deref(),
        Some("lower")
    );
    assert_eq!(
        data(&timed_out.diagnostics[0], "reason").as_deref(),
        Some("over its 5 ms limit")
    );

    let clock = Rc::new(Ticks::new(0.0));
    let ticks = Rc::clone(&clock);
    let late = FakeFrontend::new(minimal()).with_analyze(move |presentation, _| {
        ticks.advance(30.0);
        Ok(analysis_of(presentation))
    });
    let setup = Setup::new(late);
    let options = CompileOptions {
        limits: Limits {
            analyze_ms: 5,
            ..Limits::default()
        },
        clock: Some(&*clock as &dyn Clock),
        ..setup.options()
    };
    let overdue = compile_bytes(&options, FAKE, "p");
    assert_eq!(diagnostic_codes(&overdue), ["IR0905"]);
    assert_eq!(
        data(&overdue.diagnostics[0], "stage").as_deref(),
        Some("analyze")
    );
}

#[test]
fn refuses_on_an_analysis_that_refuses() {
    let refusal = make_diagnostic(
        codes::PX2001,
        Stage::Frontend,
        "pptx",
        DiagnosticSite::new(),
    );
    let refused = refusal.clone();
    let frontend = FakeFrontend::new(minimal()).with_analyze(move |presentation, _| {
        Ok(slidock_core_api::Analysis {
            refusal: Some(refused.clone()),
            ..analysis_of(presentation)
        })
    });
    let result = compile_fake(frontend, |_| {});
    assert_eq!(result.refusal, Some(refusal));
    assert!(result.provenance.is_none());
}

#[test]
fn diagnoses_every_foreign_node_declined_without_a_fallback_lossy_with_one() {
    let mut presentation = full_json();
    let mut chart = presentation["slides"][0]["elements"][5].clone();
    chart["id"] = json!("ir-s1.chart");
    chart["type"] = json!("chart");
    chart.as_object_mut().unwrap().shift_remove("fallback");
    // A fallback without media or elements prints nothing: declined too.
    let mut empty = chart.clone();
    empty["id"] = json!("ir-s1.empty");
    empty["fallback"] = json!({ "elements": [] });
    let mut pictured = chart.clone();
    pictured["id"] = json!("ir-s1.pictured");
    pictured["fallback"] = json!({ "media": photo_sha() });
    let elements = presentation["slides"][0]["elements"]
        .as_array_mut()
        .unwrap();
    elements.extend([chart, empty, pictured]);
    let result = compile_fake(FakeFrontend::new(typed(presentation)), source_phase);
    let foreign: Vec<(String, &str, String, &str)> = result
        .diagnostics
        .iter()
        .filter(|d| d.emitter == "foreign")
        .map(|d| {
            (
                d.code.code().to_owned(),
                d.severity.as_str(),
                d.node.as_ref().unwrap().to_string(),
                d.feature.unwrap().as_str(),
            )
        })
        .collect();
    assert_eq!(
        foreign,
        [
            (
                "IR0910".into(),
                "decline",
                "ir-s1.chart".into(),
                "element.chart"
            ),
            (
                "IR0910".into(),
                "decline",
                "ir-s1.empty".into(),
                "element.chart"
            ),
            (
                "IR0911".into(),
                "lossy",
                "ir-s1.pictured".into(),
                "element.chart"
            ),
            (
                "IR0911".into(),
                "lossy",
                "ir-s1.table".into(),
                "element.table"
            ),
        ]
    );
}

#[test]
fn reports_pass_timings_in_stats_only_with_a_clock() {
    let clock = Ticks::new(1.0);
    let setup = Setup::new(FakeFrontend::new(full()));
    let timed = compile_bytes(
        &CompileOptions {
            clock: Some(&clock),
            ..setup.options()
        },
        FAKE,
        "p",
    );
    let passes: Vec<String> = timed
        .stats
        .unwrap()
        .passes
        .into_iter()
        .map(|p| p.pass)
        .collect();
    assert_eq!(passes, ["claim-all"]);
    assert!(
        compile_fake(FakeFrontend::new(full()), |_| {})
            .stats
            .is_none()
    );
}

#[test]
fn keeps_per_pass_presentations_only_with_trace() {
    let traced = compile_fake(FakeFrontend::new(full()), |options| options.trace = true);
    let trace = traced.trace.unwrap();
    assert_eq!(
        trace.iter().map(|s| s.pass.as_str()).collect::<Vec<_>>(),
        ["claim-all"]
    );
    assert_eq!(trace[0].presentation.slides.len(), 2);
    assert!(
        compile_fake(FakeFrontend::new(full()), |_| {})
            .trace
            .is_none()
    );
}

#[test]
fn sorts_every_diagnostic_of_a_compile_in_output_order() {
    let result = compile_fake(FakeFrontend::new(full()), |options| {
        options.verify = VerifyMode::Each
    });
    let sorted = slidock_core_api::sort_diagnostics(
        result.diagnostics.clone(),
        &slidock_core_api::ContainerOrder::of_resolved(result.presentation.as_ref().unwrap()),
    );
    assert_eq!(result.diagnostics, sorted);
}

#[test]
fn analyzes_without_lowering() {
    let frontend = FakeFrontend::new(full());
    let calls = Rc::clone(&frontend.calls);
    let registry = registry(frontend);
    let limits = Limits::default();
    let reader = BytesReader::new(FAKE.to_vec());
    let options = slidock_core::InputOptions {
        registry: &registry,
        frontend: FrontendChoice::Auto,
        limits: &limits,
        clock: None,
        deadline: slidock_core_api::Deadline::never(),
    };
    let result = analyze(&Input::new("presentation.fake", &reader), &options);
    assert_eq!(result.analysis.unwrap().slides.len(), 2);
    assert!(calls.borrow().is_empty());
    let unread = analyze(
        &Input::new("x.bin", &BytesReader::new(b"nope".to_vec())),
        &options,
    );
    assert!(unread.analysis.is_none());
    assert_eq!(unread.refusal.unwrap().code, codes::IR0902);
    assert_eq!(unread.diagnostics.len(), 1);
}

fn named(name: &str) -> slidock_core_api::Pass {
    let provides: Vec<Invariant> = match name {
        "number-lists" => Invariant::RESOLVED.to_vec(),
        _ => Vec::new(),
    };
    slide_pass_with(info(name, &[], &provides), |view, _| Ok(view.unchanged()))
}

#[test]
fn builds_the_standard_order_from_registered_passes() {
    let mut registry = PassRegistry::new();
    for name in AFTER_FONTS.iter().rev().chain(BEFORE_FONTS.iter().rev()) {
        registry.register(named(name)).unwrap();
    }
    let pipeline = Pipeline::standard(registry).unwrap();
    let names: Vec<&str> = pipeline
        .manager()
        .passes()
        .iter()
        .map(|p| p.name())
        .collect();
    assert_eq!(
        names,
        [BEFORE_FONTS.as_slice(), AFTER_FONTS.as_slice()].concat()
    );
    assert_eq!(pipeline.font_boundary(), 6);

    let mut registry = PassRegistry::new();
    for name in BEFORE_FONTS.iter().chain(&AFTER_FONTS[..6]) {
        registry.register(named(name)).unwrap();
    }
    let pipeline = Pipeline::standard(registry).unwrap();
    assert_eq!(pipeline.manager().passes().len(), 12);
}

#[test]
fn refuses_a_registry_that_misses_or_adds_a_pass() {
    let mut registry = PassRegistry::new();
    for name in BEFORE_FONTS.iter().chain(&AFTER_FONTS[..5]) {
        registry.register(named(name)).unwrap();
    }
    assert_eq!(
        Pipeline::standard(registry).unwrap_err(),
        PipelineError::Missing("a11y".into())
    );

    let mut registry = PassRegistry::new();
    for name in BEFORE_FONTS.iter().chain(&AFTER_FONTS) {
        registry.register(named(name)).unwrap();
    }
    registry.register(named("extra")).unwrap();
    assert_eq!(
        Pipeline::standard(registry).unwrap_err(),
        PipelineError::Unknown("extra".into())
    );

    let mut registry = PassRegistry::new();
    registry.register(named("a11y")).unwrap();
    assert_eq!(
        registry.register(named("a11y")).unwrap_err(),
        PipelineError::Duplicate("a11y".into())
    );

    // The manager's order checks hold across the font boundary.
    let early = slide_pass_with(info("early", &[Invariant::FontsBound], &[]), |v, _| {
        Ok(v.unchanged())
    });
    let error = Pipeline::new(vec![early], vec![claim_all(&[])]).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Pass early requires fonts.bound, which no earlier pass provides"
    );
}
