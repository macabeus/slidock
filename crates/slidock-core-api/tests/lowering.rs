//! The scaffolding every frontend lowers containers with: ids, failure isolation, reporting.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use std::cell::Cell;

use slidock_core_api::failure::{Failable, describe_error};
use slidock_core_api::lowering::{
    ContainerReporter, FailureCodes, IdScope, LowerError, TakenId, isolate_container,
    sorted_features,
};
use slidock_core_api::{Clock, Deadline, FrontendCtx, FrontendError, Limits};
use slidock_ir::diagnostics::{
    DataValue, DiagnosticCode, DiagnosticData, DiagnosticSink, DiagnosticSite, Stage, codes,
    make_diagnostic,
};
use slidock_ir::features::FeatureId;
use slidock_ir::formats::IdPrefix;
use slidock_ir::ids::{IdPart, NodeId};
use slidock_ir::origin::Origin;
use slidock_ir::presentation::Failed;
use slidock_ir::source::Slide;

fn id(text: &str) -> NodeId {
    NodeId::new(text).unwrap()
}

fn skeleton() -> Slide {
    serde_json::from_value(serde_json::json!({
        "id": "pp-256",
        "origin": { "kind": "source", "part": "ppt/slides/slide1.xml" },
        "hidden": false,
        "showLayout": true,
        "elements": [],
        "timeline": []
    }))
    .unwrap()
}

/// A sink that records what it is given, before any stamping.
#[derive(Default)]
struct Calls(Vec<(DiagnosticCode, DiagnosticSite)>);

impl DiagnosticSink for Calls {
    fn report(&mut self, code: DiagnosticCode, site: DiagnosticSite) {
        self.0.push((code, site));
    }
}

fn site_data(site: &DiagnosticSite) -> Vec<(String, DataValue)> {
    site.data
        .iter()
        .filter_map(|(key, value)| value.clone().map(|value| (key.to_string(), value)))
        .collect()
}

fn s(value: &str) -> DataValue {
    DataValue::Str(value.to_owned())
}

/// A failure, the code it records and its data.
type Case = (
    &'static str,
    LowerError,
    DiagnosticCode,
    Vec<(String, DataValue)>,
);

fn codes_for_pptx() -> FailureCodes {
    FailureCodes::new(codes::PX2007)
        .part(codes::PX2002)
        .limit(codes::PX2901)
        .data("part", "ppt/slides/slide1.xml")
}

struct Now(Cell<f64>);

impl Clock for Now {
    fn now_ms(&self) -> f64 {
        self.0.get()
    }
}

fn with_ctx<T>(
    clock: Option<&dyn Clock>,
    run: impl FnOnce(&mut FrontendCtx<'_>) -> T,
) -> (T, Calls) {
    let limits = Limits::default();
    let mut calls = Calls::default();
    let result = {
        let mut ctx = FrontendCtx {
            limits: &limits,
            diag: &mut calls,
            deadline: Deadline::start(clock, 5),
            spent: slidock_core_api::Output::default(),
        };
        run(&mut ctx)
    };
    (result, calls)
}

#[test]
fn gives_a_repeated_id_the_next_free_suffix_and_says_so() {
    let mut ids = IdScope::new(IdPrefix::Pp);
    let parts = [IdPart::Int(256), IdPart::Int(3)];
    assert_eq!(
        ids.take(&parts).unwrap(),
        TakenId {
            id: id("pp-256.3"),
            duplicate: false
        }
    );
    assert_eq!(
        ids.take(&parts).unwrap(),
        TakenId {
            id: id("pp-256.3.2"),
            duplicate: true
        }
    );
    assert_eq!(
        ids.take(&parts).unwrap(),
        TakenId {
            id: id("pp-256.3.3"),
            duplicate: true
        }
    );
    // A native id that is itself a suffixed one takes the next free suffix of its own.
    assert_eq!(
        ids.take(&[IdPart::Int(256), IdPart::Int(3), IdPart::Int(2)])
            .unwrap(),
        TakenId {
            id: id("pp-256.3.2.2"),
            duplicate: true
        }
    );
    assert!(ids.take(&[]).is_err());
}

#[test]
fn returns_what_lowering_returns() {
    let slide = skeleton();
    let (result, calls) = with_ctx(None, |ctx| {
        isolate_container(&skeleton(), ctx, &codes_for_pptx(), |_| Ok(skeleton()))
    });
    assert_eq!(result.unwrap(), slide);
    assert!(calls.0.is_empty());
}

#[test]
fn fails_the_container_alone_keeping_its_id_and_place() {
    let cases: Vec<Case> = vec![
        (
            "a part it cannot read",
            LowerError::Part {
                part: "ppt/media/x.xml".into(),
                reason: "missing".into(),
            },
            codes::PX2002,
            vec![
                ("part".into(), s("ppt/media/x.xml")),
                ("reason".into(), s("missing")),
            ],
        ),
        (
            "a limit",
            LowerError::Limit("too many elements".into()),
            codes::PX2901,
            vec![
                ("part".into(), s("ppt/slides/slide1.xml")),
                ("reason".into(), s("too many elements")),
            ],
        ),
        (
            "anything else",
            LowerError::Other("x is undefined".into()),
            codes::PX2007,
            vec![
                ("part".into(), s("ppt/slides/slide1.xml")),
                ("reason".into(), s("x is undefined")),
            ],
        ),
        (
            "an error that names its code",
            LowerError::Coded {
                code: codes::KN1005,
                data: [
                    ("page".to_owned(), DataValue::Num(2.0)),
                    ("w".to_owned(), DataValue::Num(0.0)),
                ]
                .into_iter()
                .collect::<DiagnosticData>(),
            },
            codes::KN1005,
            vec![
                ("page".into(), DataValue::Num(2.0)),
                ("w".into(), DataValue::Num(0.0)),
            ],
        ),
    ];
    for (name, error, code, data) in cases {
        let (result, calls) = with_ctx(None, |ctx| {
            isolate_container(&skeleton(), ctx, &codes_for_pptx(), move |_| Err(error))
        });
        let failed = result.unwrap();
        assert_eq!(failed.id, id("pp-256"), "{name}");
        assert!(failed.elements.is_empty(), "{name}");
        assert_eq!(failed.failed, Some(Failed::new(code, None)), "{name}");
        assert_eq!(failed.origin, skeleton().origin, "{name}");
        assert_eq!(calls.0.len(), 1, "{name}");
        assert_eq!(calls.0[0].0, code, "{name}");
        assert_eq!(calls.0[0].1.container, Some(id("pp-256")), "{name}");
        assert_eq!(site_data(&calls.0[0].1), data, "{name}");
    }
}

#[test]
fn falls_back_to_the_other_code_for_a_limit_or_a_part_when_the_frontend_has_none() {
    let codes = FailureCodes::new(codes::KN1004).data("page", 1u32);
    let (_, calls) = with_ctx(None, |ctx| {
        isolate_container(&skeleton(), ctx, &codes, |_| {
            Err(LowerError::Limit("x".into()))
        })
    });
    assert_eq!(calls.0[0].0, codes::KN1004);
    assert_eq!(
        site_data(&calls.0[0].1),
        [
            ("page".into(), DataValue::Num(1.0)),
            ("reason".into(), s("x"))
        ]
    );
    let (_, calls) = with_ctx(None, |ctx| {
        isolate_container(&skeleton(), ctx, &codes, |_| {
            Err(LowerError::Part {
                part: "a.xml".into(),
                reason: "gone".into(),
            })
        })
    });
    assert_eq!(
        site_data(&calls.0[0].1),
        [
            ("page".into(), DataValue::Num(1.0)),
            ("reason".into(), s("a.xml: gone"))
        ]
    );
}

#[test]
fn clips_the_reason_to_200_code_units() {
    let long = "é".repeat(150) + &"😀".repeat(100);
    let (_, calls) = with_ctx(None, |ctx| {
        isolate_container(&skeleton(), ctx, &codes_for_pptx(), move |_| {
            Err(LowerError::Other(long))
        })
    });
    let reason = site_data(&calls.0[0].1).pop().unwrap().1.to_js_string();
    assert_eq!(slidock_canon::utf16::len(&reason), 200);
    assert_eq!(describe_error(&"x".repeat(201)).len(), 200);
    // A surrogate pair across the limit is dropped whole.
    assert_eq!(
        slidock_canon::utf16::len(&describe_error(&("x".repeat(199) + "😀"))),
        199
    );
}

#[test]
fn ends_the_whole_lowering_on_a_refusal_or_a_passed_deadline() {
    let refusal = make_diagnostic(
        codes::PX2001,
        Stage::Frontend,
        "pptx",
        DiagnosticSite::new(),
    );
    let refused = refusal.clone();
    let (result, calls) = with_ctx(None, |ctx| {
        isolate_container(&skeleton(), ctx, &codes_for_pptx(), move |_| {
            Err(LowerError::Refused(Box::new(refused)))
        })
    });
    assert_eq!(result.unwrap_err(), FrontendError::refused(refusal));
    assert!(calls.0.is_empty());

    let clock = Now(Cell::new(0.0));
    let (result, calls) = with_ctx(Some(&clock), |ctx| {
        isolate_container(&skeleton(), ctx, &codes_for_pptx(), |_| {
            clock.0.set(10.0);
            Err(LowerError::Other("interrupted".into()))
        })
    });
    assert_eq!(result.unwrap_err(), FrontendError::Stopped);
    assert!(calls.0.is_empty());

    let (result, _) = with_ctx(Some(&clock), |ctx| {
        clock.0.set(20.0);
        isolate_container(&skeleton(), ctx, &codes_for_pptx(), |_| Ok(skeleton()))
    });
    assert_eq!(result.unwrap_err(), FrontendError::Stopped);

    let (result, _) = with_ctx(None, |ctx| {
        isolate_container(&skeleton(), ctx, &codes_for_pptx(), |_| {
            Err(LowerError::Stopped)
        })
    });
    assert_eq!(result.unwrap_err(), FrontendError::Stopped);
}

#[test]
fn stamps_the_container_on_every_report_and_collects_features() {
    let mut calls = Calls::default();
    let mut reporter = ContainerReporter::new(&mut calls, id("pp-256"));
    reporter.report(
        codes::PX2107,
        DiagnosticSite::new().data("modifier", "gamma"),
    );
    reporter.seen.insert(FeatureId::TextRun);
    reporter.seen.insert(FeatureId::ElementShape);
    reporter.seen.insert(FeatureId::TextRun);
    assert_eq!(
        reporter.features(),
        [FeatureId::ElementShape, FeatureId::TextRun]
    );
    drop(reporter);
    assert_eq!(calls.0.len(), 1);
    assert_eq!(calls.0[0].1.container, Some(id("pp-256")));
    assert_eq!(site_data(&calls.0[0].1), [("modifier".into(), s("gamma"))]);
    assert_eq!(
        sorted_features([
            FeatureId::TextRun,
            FeatureId::ElementShape,
            FeatureId::TextRun
        ]),
        [FeatureId::ElementShape, FeatureId::TextRun]
    );
}

#[test]
fn a_failed_skeleton_keeps_links_and_background_unless_bare() {
    let mut slide = skeleton();
    slide.layout = Some(id("pp-l1"));
    slide.title = Some("Title".into());
    let failed = slide.failed_skeleton(Failed::new(codes::IR0908, Some("thrower".into())), false);
    assert_eq!(
        (failed.layout.clone(), failed.title.clone()),
        (Some(id("pp-l1")), None)
    );
    assert_eq!(failed.origin, slide.origin);
    let bare = slide.failed_skeleton(Failed::new(codes::IR0908, Some("thrower".into())), true);
    assert_eq!(bare.layout, None);
    assert_eq!(bare.origin, Origin::synth("thrower"));
    assert_eq!((bare.hidden, bare.show_layout), (false, true));
    let verify = slide.failed_skeleton(Failed::new(codes::IR0001, None), true);
    assert_eq!(verify.origin, Origin::synth("verify"));
}
