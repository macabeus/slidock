//! The pass manager: construction checks, both verify modes, failure isolation, visit-order independence, dumps
//! and replay, timings and traces.
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
    ConfigError, PassManager, VerifyMode, replay_failure, verify_source_presentation,
};
use slidock_core_api::{OwnedContainer, PassError, Unverified};
use slidock_ir::diagnostics::{Diagnostic, DiagnosticSite, codes};
use slidock_ir::source::Presentation;
use slidock_ir::text::PartialRunProps;
use slidock_ir::verify::{Invariant, cascaded_run};
use support::*;

use VerifyMode::{Boundaries, Each};

/// The resolved presentation in the source phase's types, whose fields tests read directly.
fn resolved(result: &slidock_core::RunResult) -> Presentation {
    result.presentation.as_ref().unwrap().to_source().unwrap()
}

fn slide<'a>(presentation: &'a Presentation, id: &str) -> &'a slidock_ir::source::Slide {
    source_slide(presentation, id)
}

fn source_slide<'a>(presentation: &'a Presentation, id: &str) -> &'a slidock_ir::source::Slide {
    presentation
        .slides
        .iter()
        .find(|s| s.id.as_str() == id)
        .unwrap()
}

fn failed_code(
    failed: Option<&slidock_ir::presentation::Failed>,
) -> Option<(String, Option<String>)> {
    failed.map(|f| (f.code.as_str().to_owned(), f.pass.clone()))
}

fn thrower(target: &'static str) -> slidock_core_api::Pass {
    slide_pass("thrower", move |view, _| {
        if view.container.id().as_str() == target {
            return Err(PassError::failed("boom"));
        }
        Ok(view.unchanged())
    })
}

#[test]
fn accepts_an_order_where_every_requirement_is_provided_earlier() {
    assert!(PassManager::new(vec![claim_all(&[])]).is_ok());
    let ordered = vec![
        claim_all(&[Invariant::A11yDone]),
        slide_pass_with(
            info("a11y", &[Invariant::TextCascaded], &[Invariant::A11yDone]),
            |v, _| Ok(v.unchanged()),
        ),
    ];
    assert_eq!(PassManager::new(ordered).unwrap().passes().len(), 2);
}

#[test]
fn refuses_a_pass_whose_requirement_no_earlier_pass_provides() {
    let early = slide_pass_with(info("early", &[Invariant::TextCascaded], &[]), |v, _| {
        Ok(v.unchanged())
    });
    let error = PassManager::new(vec![early, claim_all(&[])]).unwrap_err();
    assert_eq!(
        error,
        ConfigError::Unprovided {
            pass: "early".into(),
            invariant: Invariant::TextCascaded
        }
    );
    assert_eq!(
        error.to_string(),
        "Pass early requires text.cascaded, which no earlier pass provides"
    );
}

#[test]
fn refuses_a_list_that_leaves_a_resolved_invariant_unprovided_and_duplicate_names() {
    let error = PassManager::new(vec![claim_all(&[Invariant::FontsBound])]).unwrap_err();
    assert_eq!(
        error.to_string(),
        "verify-resolved requires fonts.bound, which no pass provides"
    );
    let error = PassManager::new(vec![claim_all(&[]), identity("claim-all")]).unwrap_err();
    assert_eq!(error.to_string(), "Pass claim-all appears twice");
}

#[test]
fn resolves_a_presentation_whose_passes_hold_every_invariant_in_both_modes() {
    for presentation in [minimal(), full()] {
        let each = run(
            vec![claim_all(&[]), identity("identity")],
            &presentation,
            Each,
        );
        assert_eq!(errors(&each.diagnostics), Vec::<Diagnostic>::new());
        assert!(each.presentation.is_some());
        let boundaries = run(
            vec![claim_all(&[]), identity("identity")],
            &presentation,
            Boundaries,
        );
        assert_eq!(bytes(&boundaries), bytes(&each));
    }
}

#[test]
fn names_the_pass_and_container_when_a_pass_declares_an_invariant_it_does_not_provide() {
    let passes = vec![claim_all(&[Invariant::TextCascaded]), cascade_liar("ir-s2")];
    let result = run(passes, &full(), Each);
    let violation = find(&result.diagnostics, "IR0909").unwrap();
    assert_eq!(violation.container, Some(id("ir-s2")));
    assert_eq!(data(violation, "pass").as_deref(), Some("cascade-liar"));
    assert_eq!(
        data(violation, "invariant").as_deref(),
        Some("text.cascaded")
    );
    assert_eq!(data(violation, "rule").as_deref(), Some("IR0103"));
    let presentation = &resolved(&result);
    let s2 = slide(presentation, "ir-s2");
    assert_eq!(
        failed_code(s2.failed.as_ref()),
        Some(("IR0909".into(), Some("cascade-liar".into())))
    );
    assert!(s2.elements.is_empty());
    assert!(slide(presentation, "ir-s1").failed.is_none());
}

#[test]
fn gives_byte_identical_results_in_each_and_boundaries_mode_with_a_rule_breaking_pass() {
    let each = run(
        vec![claim_all(&[Invariant::TextCascaded]), cascade_liar("ir-s2")],
        &full(),
        Each,
    );
    let boundaries = run(
        vec![claim_all(&[Invariant::TextCascaded]), cascade_liar("ir-s2")],
        &full(),
        Boundaries,
    );
    assert_eq!(bytes(&boundaries), bytes(&each));
    assert!(find(&boundaries.diagnostics, "IR0909").is_some());
}

#[test]
fn fails_only_the_slide_a_slide_scope_pass_errs_on() {
    for verify in [Each, Boundaries] {
        let result = run(vec![claim_all(&[]), thrower("ir-s2")], &full(), verify);
        let errors = errors(&result.diagnostics);
        assert_eq!(codes(&errors), [("IR0908".into(), Some("ir-s2".into()))]);
        let last = result.diagnostics.last().unwrap();
        assert_eq!(data(last, "pass").as_deref(), Some("thrower"));
        assert_eq!(data(last, "error").as_deref(), Some("boom"));
        let presentation = &resolved(&result);
        let s2 = slide(presentation, "ir-s2");
        assert_eq!(
            failed_code(s2.failed.as_ref()),
            Some(("IR0908".into(), Some("thrower".into())))
        );
        assert!(s2.elements.is_empty());
        assert!(!slide(presentation, "ir-s1").elements.is_empty());
    }
}

#[test]
fn fails_an_erring_layout_and_the_slides_that_show_it_and_still_compiles() {
    let presentation = typed(copy_slide(&full_json(), "ir-s2", "ir-s3", false));
    let thrower = slide_pass("thrower", |view, _| {
        if view.container.kind() == "layout" {
            return Err(PassError::failed("layout boom"));
        }
        Ok(view.unchanged())
    });
    let result = run(vec![claim_all(&[]), thrower], &presentation, Each);
    let resolved = &resolved(&result);
    let layout = &resolved.layouts[&id("ir-layout")];
    assert_eq!(
        failed_code(layout.failed.as_ref()).map(|f| f.0),
        Some("IR0908".into())
    );
    assert!(layout.elements.is_empty());
    for slide_id in ["ir-s1", "ir-s2"] {
        assert_eq!(
            failed_code(slide(resolved, slide_id).failed.as_ref()),
            Some(("IR0908".into(), Some("thrower".into())))
        );
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|d| d.container.as_ref().is_some_and(|c| c.as_str() == slide_id))
            .unwrap();
        assert_eq!(data(diagnostic, "via").as_deref(), Some("ir-layout"));
    }
    assert!(slide(resolved, "ir-s3").failed.is_none());
}

#[test]
fn skips_failed_containers_in_later_passes() {
    let seen: Rc<RefCell<Vec<String>>> = Rc::default();
    let record = Rc::clone(&seen);
    let watcher = slide_pass("watcher", move |view, _| {
        record.borrow_mut().push(view.container.id().to_string());
        Ok(view.unchanged())
    });
    run(
        vec![claim_all(&[]), thrower("ir-s2"), watcher],
        &full(),
        Each,
    );
    let seen = seen.borrow();
    assert!(!seen.contains(&"ir-s2".to_owned()));
    assert!(seen.contains(&"ir-s1".to_owned()));
}

/// A slide pass that reports a warning on every container it runs on, and errs on `target`.
fn noter(target: &'static str) -> slidock_core_api::Pass {
    slide_pass("noter", move |view, ctx| {
        ctx.diag.report(
            codes::IR0012,
            DiagnosticSite::new()
                .container(view.container.id().clone())
                .data("field", "noted"),
        );
        if view.container.id().as_str() == target {
            return Err(PassError::failed("boom"));
        }
        Ok(view.unchanged())
    })
}

/// `[code, container]` of the diagnostics `emitter` reported.
fn emitted(result: &slidock_core::RunResult, emitter: &str) -> Vec<(String, Option<String>)> {
    let own: Vec<Diagnostic> = result
        .diagnostics
        .iter()
        .filter(|d| d.emitter == emitter)
        .cloned()
        .collect();
    codes(&own)
}

#[test]
fn keeps_a_slide_pass_diagnostics_for_the_containers_it_succeeds_on_and_drops_them_where_it_errs() {
    let noted = |container: &str| ("IR0012".to_owned(), Some(container.to_owned()));
    let failed = |container: &str| ("IR0908".to_owned(), Some(container.to_owned()));
    for verify in [Each, Boundaries] {
        let clean = run(vec![claim_all(&[]), noter("")], &full(), verify);
        assert_eq!(
            emitted(&clean, "noter"),
            [
                noted("ir-master"),
                noted("ir-layout"),
                noted("ir-s1"),
                noted("ir-s2")
            ],
            "{verify:?}"
        );
        let erring = run(vec![claim_all(&[]), noter("ir-s2")], &full(), verify);
        assert_eq!(
            emitted(&erring, "noter"),
            [
                noted("ir-master"),
                noted("ir-layout"),
                noted("ir-s1"),
                failed("ir-s2")
            ],
            "{verify:?}"
        );
    }
}

#[test]
fn keeps_a_presentation_pass_diagnostics_when_it_succeeds_and_drops_them_when_it_errs() {
    let reporter = |fails: bool| {
        presentation_pass("reporter", move |presentation, ctx| {
            ctx.diag
                .report(codes::IR0012, DiagnosticSite::new().data("field", "noted"));
            if fails {
                return Err(PassError::failed("presentation boom"));
            }
            Ok(presentation.clone())
        })
    };
    for verify in [Each, Boundaries] {
        let kept = run(vec![claim_all(&[]), reporter(false)], &full(), verify);
        assert!(kept.presentation.is_some());
        assert_eq!(
            emitted(&kept, "reporter"),
            [("IR0012".to_owned(), None)],
            "{verify:?}"
        );
        let dropped = run(vec![claim_all(&[]), reporter(true)], &full(), verify);
        assert!(dropped.presentation.is_none());
        assert_eq!(
            emitted(&dropped, "reporter"),
            [("IR0907".to_owned(), None)],
            "{verify:?}"
        );
    }
}

#[test]
fn does_not_fail_again_a_container_an_earlier_pass_failed_when_what_it_shows_fails() {
    let kind_thrower = |name: &'static str, kind: &'static str| {
        slide_pass(name, move |view, _| {
            if view.container.kind() == kind {
                return Err(PassError::failed("boom"));
            }
            Ok(view.unchanged())
        })
    };
    for verify in [Each, Boundaries] {
        let passes = vec![
            claim_all(&[]),
            kind_thrower("layout-thrower", "layout"),
            kind_thrower("master-thrower", "master"),
        ];
        let result = run(passes, &full(), verify);
        let resolved = &resolved(&result);
        assert_eq!(
            failed_code(resolved.masters[&id("ir-master")].failed.as_ref()),
            Some(("IR0908".into(), Some("master-thrower".into())))
        );
        assert_eq!(
            failed_code(resolved.layouts[&id("ir-layout")].failed.as_ref()),
            Some(("IR0908".into(), Some("layout-thrower".into())))
        );
        for slide_id in ["ir-s1", "ir-s2"] {
            assert_eq!(
                failed_code(slide(resolved, slide_id).failed.as_ref()),
                Some(("IR0908".into(), Some("layout-thrower".into())))
            );
        }
        let via: Vec<(String, String)> = result
            .diagnostics
            .iter()
            .filter_map(|d| Some((d.container.as_ref()?.to_string(), data(d, "via")?)))
            .collect();
        assert_eq!(
            via,
            [
                ("ir-s1".to_owned(), "ir-layout".to_owned()),
                ("ir-s2".to_owned(), "ir-layout".to_owned())
            ],
            "{verify:?}"
        );
    }
}

/// Each container a pass saw, with its view's show position.
type Positions = Rc<RefCell<Vec<(String, Option<usize>)>>>;

#[test]
fn gives_slide_views_their_one_based_show_position_and_other_containers_none() {
    let seen: Positions = Rc::default();
    let record = Rc::clone(&seen);
    let watcher = slide_pass("watcher", move |view, _| {
        record
            .borrow_mut()
            .push((view.container.id().to_string(), view.show_index));
        Ok(view.unchanged())
    });
    run(vec![claim_all(&[]), watcher], &full(), Each);
    assert_eq!(
        *seen.borrow(),
        [
            ("ir-master".to_owned(), None),
            ("ir-layout".to_owned(), None),
            ("ir-s1".to_owned(), Some(1)),
            ("ir-s2".to_owned(), Some(2))
        ]
    );
}

#[test]
fn fails_the_compile_on_a_presentation_scope_error_or_a_presentation_level_rule() {
    let failing = presentation_pass("presentation-thrower", |_, _| {
        Err(PassError::failed("presentation boom"))
    });
    let thrown = run(vec![claim_all(&[]), failing], &full(), Each);
    assert!(thrown.presentation.is_none());
    let failed = find(&thrown.diagnostics, "IR0907").unwrap();
    assert_eq!(
        data(failed, "pass").as_deref(),
        Some("presentation-thrower")
    );
    assert_eq!(data(failed, "reason").as_deref(), Some("presentation boom"));

    let duplicating = presentation_pass("duplicator", |presentation, _| {
        let mut out = presentation.clone();
        let first = out.slides[0].elements.clone();
        for slide in &mut out.slides {
            slide.elements.clone_from(&first);
        }
        Ok(out)
    });
    let duplicated = run(vec![claim_all(&[]), duplicating], &full(), Each);
    assert!(duplicated.presentation.is_none());
    assert_eq!(
        data(find(&duplicated.diagnostics, "IR0907").unwrap(), "pass").as_deref(),
        Some("duplicator")
    );
}

#[test]
fn fails_the_compile_when_a_presentation_pass_reshapes_the_containers() {
    let dropper = presentation_pass("dropper", |presentation, _| {
        let mut out = presentation.clone();
        out.slides.pop();
        Ok(out)
    });
    let result = run(vec![claim_all(&[]), dropper], &full(), Each);
    assert!(result.presentation.is_none());
    assert_eq!(
        data(find(&result.diagnostics, "IR0907").unwrap(), "reason").as_deref(),
        Some("a presentation-scope pass added, removed or reordered containers")
    );
}

#[test]
fn keeps_failed_containers_out_of_a_presentation_pass_reach() {
    let clearer = presentation_pass("clearer", |presentation, _| {
        let mut out = presentation.clone();
        for slide in &mut out.slides {
            slide.title = Some("cleared".into());
        }
        Ok(out)
    });
    let result = run(
        vec![claim_all(&[]), thrower("ir-s2"), clearer],
        &full(),
        Each,
    );
    let resolved = resolved(&result);
    assert_eq!(slide(&resolved, "ir-s1").title.as_deref(), Some("cleared"));
    assert_eq!(slide(&resolved, "ir-s2").title, None);
    assert!(slide(&resolved, "ir-s2").failed.is_some());
}

#[test]
fn fails_the_slide_whose_pass_returns_a_different_container_and_attributes_guard_failures() {
    let swapper = slide_pass("swapper", |view, _| {
        let mut out = view.unchanged();
        if view.container.id().as_str() == "ir-s2"
            && let OwnedContainer::Slide(slide) = &mut out
        {
            slide.id = id("ir-other");
        }
        Ok(out)
    });
    let swapped = run(vec![claim_all(&[]), swapper], &full(), Each);
    assert_eq!(
        data(find(&swapped.diagnostics, "IR0908").unwrap(), "error").as_deref(),
        Some("returned a different container")
    );

    let kind_swapper = slide_pass("kind-swapper", |view, _| match view.unchanged() {
        OwnedContainer::Layout(layout) => {
            let master = view.master.unwrap().clone();
            Ok(OwnedContainer::Master(slidock_ir::source::Master {
                id: layout.id,
                ..master
            }))
        }
        other => Ok(other),
    });
    let swapped = run(vec![claim_all(&[]), kind_swapper], &full(), Each);
    let diagnostic = find(&swapped.diagnostics, "IR0908").unwrap();
    assert_eq!(diagnostic.container, Some(id("ir-layout")));
    assert_eq!(
        data(diagnostic, "error").as_deref(),
        Some("returned a different container")
    );

    let guarded = slide_pass("guarded", |view, _| {
        if view.container.id().as_str() == "ir-s1" {
            cascaded_run(Some(&PartialRunProps::default()), &id("ir-s1.title"))?;
        }
        Ok(view.unchanged())
    });
    let result = run(vec![claim_all(&[]), guarded], &full(), Boundaries);
    let violation = find(&result.diagnostics, "IR0909").unwrap();
    assert_eq!(violation.container, Some(id("ir-s1")));
    assert_eq!(violation.node, Some(id("ir-s1.title")));
    assert_eq!(data(violation, "pass").as_deref(), Some("guarded"));
    assert_eq!(
        data(violation, "invariant").as_deref(),
        Some("text.cascaded")
    );
    assert_eq!(data(violation, "rule").as_deref(), Some("IR0103"));
}

#[test]
fn gives_the_same_output_whatever_order_containers_are_visited_in() {
    let presentation = typed(copy_slide(&full_json(), "ir-s2", "ir-s3", false));
    let passes = manager(vec![
        claim_all(&[Invariant::TextCascaded]),
        cascade_liar("ir-s1"),
    ]);
    let forward = run_with(&passes, &presentation, Each, Extra::default());
    let reversed = run_with(
        &passes,
        &presentation,
        Each,
        Extra {
            visit_order: Some(&reverse),
            ..Extra::default()
        },
    );
    assert_eq!(bytes(&reversed), bytes(&forward));
    assert!(find(&forward.diagnostics, "IR0909").is_some());
}

#[test]
fn dumps_what_a_failed_pass_saw_keyed_by_the_hash_in_its_diagnostic_and_replays_it() {
    let presentation = full();
    let result = run(
        vec![claim_all(&[Invariant::TextCascaded]), cascade_liar("ir-s2")],
        &presentation,
        Each,
    );
    let dump = &result.failures[0];
    assert_eq!(
        (
            dump.pass.as_str(),
            dump.version,
            dump.scope.as_str(),
            dump.container.clone()
        ),
        ("cascade-liar", 1, "slide", Some(id("ir-s2")))
    );
    let diagnostic = find(&result.diagnostics, "IR0909").unwrap();
    let expected = slidock_canon::sha256_canonical(
        &json!({ "pass": "cascade-liar", "version": 1, "view": dump.view }),
    )
    .unwrap();
    assert_eq!(data(diagnostic, "input"), Some(expected));

    let reread: slidock_core::FailureDump =
        serde_json::from_str(&slidock_ir::canonical::to_json(dump).unwrap()).unwrap();
    let canonical =
        |dump: &slidock_core::FailureDump| slidock_ir::canonical::to_json(dump).unwrap();
    assert_eq!(canonical(&reread), canonical(dump));
    let shared = Shared::of(&presentation);
    let replayed = replay_failure(&reread, &cascade_liar("ir-s2"), &shared.env()).unwrap();
    let rules: Vec<(String, Option<String>)> = replayed
        .iter()
        .map(|d| (d.code.code().to_owned(), data(d, "rule")))
        .collect();
    assert!(
        rules.contains(&("IR0909".into(), Some("IR0103".into()))),
        "{rules:?}"
    );
    let fixed = slide_pass_with(
        info("cascade-liar", &[], &[Invariant::TextCascaded]),
        |v, _| Ok(v.unchanged()),
    );
    assert_eq!(
        replay_failure(&reread, &fixed, &shared.env()).unwrap(),
        Vec::<Diagnostic>::new()
    );
    let wrong = replay_failure(&reread, &identity("other"), &shared.env()).unwrap_err();
    assert_eq!(
        wrong.to_string(),
        "The dump is for slide pass cascade-liar, not slide pass other"
    );
}

#[test]
fn replays_a_presentation_scope_failure() {
    let failing = || {
        presentation_pass("presentation-thrower", |_, _| {
            Err(PassError::failed("presentation boom"))
        })
    };
    let presentation = minimal();
    let result = run(vec![claim_all(&[]), failing()], &presentation, Each);
    let dump = &result.failures[0];
    assert_eq!(
        (dump.scope.as_str(), dump.container.clone()),
        ("presentation", None)
    );
    let shared = Shared::of(&presentation);
    let replayed = replay_failure(dump, &failing(), &shared.env()).unwrap();
    assert_eq!(codes(&replayed), [("IR0908".into(), None)]);
    assert_eq!(
        data(&replayed[0], "error").as_deref(),
        Some("presentation boom")
    );
}

#[test]
fn reports_timings_only_with_a_clock() {
    let clock = Ticks::new(0.5);
    let passes = manager(vec![claim_all(&[]), identity("identity")]);
    let timed = run_with(
        &passes,
        &full(),
        Each,
        Extra {
            clock: Some(&clock),
            ..Extra::default()
        },
    );
    let timings = timed.timings.unwrap();
    assert_eq!(
        timings.iter().map(|t| t.pass.as_str()).collect::<Vec<_>>(),
        ["claim-all", "identity"]
    );
    assert_eq!(timings[0].ms, 0.5);
    let containers: Vec<&str> = timings[1]
        .containers
        .as_ref()
        .unwrap()
        .keys()
        .map(|k| k.as_str())
        .collect();
    assert_eq!(containers, ["ir-layout", "ir-master", "ir-s1", "ir-s2"]);
    assert!(
        timings[1]
            .containers
            .as_ref()
            .unwrap()
            .values()
            .all(|ms| *ms == 0.5)
    );
    assert_eq!(timings[1].ms, 2.0);
    assert!(run(vec![claim_all(&[])], &full(), Each).timings.is_none());
}

#[test]
fn keeps_the_presentation_after_each_pass_only_when_asked() {
    let received: Rc<RefCell<Vec<(String, Presentation)>>> = Rc::default();
    let recorder = |name: &'static str| {
        let received = Rc::clone(&received);
        presentation_pass(name, move |presentation, _| {
            received
                .borrow_mut()
                .push((name.to_owned(), presentation.clone()));
            Ok(presentation.clone())
        })
    };
    let passes = manager(vec![claim_all(&[]), recorder("first"), recorder("second")]);
    let traced = run_with(
        &passes,
        &full(),
        Each,
        Extra {
            trace: true,
            ..Extra::default()
        },
    );
    let trace = traced.trace.unwrap();
    assert_eq!(
        trace.iter().map(|s| s.pass.as_str()).collect::<Vec<_>>(),
        ["claim-all", "first", "second"]
    );
    {
        let received = received.borrow();
        // Each pass received what the one before it handed on.
        assert_eq!(trace[0].presentation, received[0].1);
        assert_eq!(trace[1].presentation, received[1].1);
    }
    assert!(
        run_with(&passes, &full(), Each, Extra::default())
            .trace
            .is_none()
    );
}

#[test]
fn records_each_pass_once_when_boundaries_runs_again_under_each() {
    let passes = manager(vec![
        claim_all(&[Invariant::TextCascaded]),
        cascade_liar("ir-s2"),
    ]);
    let traced = run_with(
        &passes,
        &full(),
        Boundaries,
        Extra {
            trace: true,
            ..Extra::default()
        },
    );
    assert!(slide(&resolved(&traced), "ir-s2").failed.is_some());
    let trace = traced.trace.unwrap();
    assert_eq!(
        trace.iter().map(|s| s.pass.as_str()).collect::<Vec<_>>(),
        ["claim-all", "cascade-liar"]
    );
    let failed = source_slide(&trace[1].presentation, "ir-s2")
        .failed
        .as_ref();
    assert_eq!(failed_code(failed).map(|f| f.0), Some("IR0909".into()));
}

/// A pass that points ir-s2 at a layout that does not exist.
fn dangler() -> slidock_core_api::Pass {
    slide_pass("dangler", |view, _| {
        let mut out = view.unchanged();
        if let OwnedContainer::Slide(slide) = &mut out
            && slide.id.as_str() == "ir-s2"
        {
            slide.layout = Some(id("ir-gone"));
        }
        Ok(out)
    })
}

#[test]
fn fails_the_compile_when_a_container_breaks_a_rule_even_as_a_failed_container() {
    // The slide's layout link is kept in its failed skeleton, so failing it cannot clear the dangling link.
    for verify in [Each, Boundaries] {
        let result = run(vec![claim_all(&[]), dangler()], &full(), verify);
        assert!(result.presentation.is_none());
        let failed = find(&result.diagnostics, "IR0907").unwrap();
        assert_eq!(
            data(failed, "reason").as_deref(),
            Some("ir-s2 cannot be represented even as a failed container")
        );
    }
}

#[test]
fn verify_resolved_fails_a_container_a_pass_left_incomplete() {
    // Without each-mode checks, only verify-resolved sees the stripped sizes.
    let passes = vec![claim_all(&[]), cascade_liar("ir-s2")];
    let result = run(passes, &full(), Boundaries);
    let violation = find(&result.diagnostics, "IR0909").unwrap();
    // The run under each attributes it to the pass.
    assert_eq!(data(violation, "pass").as_deref(), Some("cascade-liar"));

    let shared = Shared::of(&full());
    let options = slidock_core::RunOptions::new(Each, shared.env());
    let manager = manager(vec![claim_all(&[]), cascade_liar("ir-s2")]);
    let first = manager.run(&Unverified::from(full()), &options);
    assert_eq!(bytes(&first), bytes(&result));
}

#[test]
fn verify_source_passes_a_valid_source_presentation_through() {
    let verified = verify_source_presentation(&Unverified::Json(full_json()), None);
    assert_eq!(verified.presentation.unwrap().slides.len(), 2);
    assert_eq!(errors(&verified.diagnostics), Vec::<Diagnostic>::new());
    let typed = verify_source_presentation(&Unverified::from(full()), None);
    assert_eq!(typed.presentation, Some(full()));
}

#[test]
fn verify_source_fails_a_slide_with_a_schema_error_and_keeps_the_rest() {
    let mut input = full_json();
    input["slides"][1]["elements"][0]["frame"]["w"] = json!(0.0001);
    let verified = verify_source_presentation(&Unverified::Json(input), None);
    assert_eq!(
        codes(&errors(&verified.diagnostics)),
        [("IR0007".into(), Some("ir-s2".into()))]
    );
    let presentation = verified.presentation.unwrap();
    let s2 = source_slide(&presentation, "ir-s2");
    assert_eq!(
        failed_code(s2.failed.as_ref()).map(|f| f.0),
        Some("IR0007".into())
    );
    assert!(s2.elements.is_empty());
    assert!(source_slide(&presentation, "ir-s1").failed.is_none());
}

#[test]
fn verify_source_fails_a_typed_presentation_out_of_bounds() {
    let mut input = full();
    input.slides[1].title = Some("x".repeat(300));
    let verified = verify_source_presentation(&Unverified::from(input), None);
    assert_eq!(
        codes(&errors(&verified.diagnostics)),
        [("IR0008".into(), Some("ir-s2".into()))]
    );
    let presentation = verified.presentation.unwrap();
    assert!(source_slide(&presentation, "ir-s2").failed.is_some());
    assert!(source_slide(&presentation, "ir-s2").title.is_none());
}

#[test]
fn verify_source_fails_a_layout_and_its_slides_and_a_slide_whose_kept_fields_are_invalid_goes_bare()
{
    let mut input = full_json();
    input["layouts"]["ir-layout"]["bogus"] = json!(true);
    input["slides"][0]["background"] = json!({ "kind": "nonsense" });
    let verified = verify_source_presentation(&Unverified::Json(input), None);
    let presentation = verified.presentation.unwrap();
    let layout = &presentation.layouts[&id("ir-layout")];
    assert_eq!(
        failed_code(layout.failed.as_ref()),
        Some(("IR0001".into(), None))
    );
    assert!(layout.elements.is_empty());
    let s1 = source_slide(&presentation, "ir-s1");
    assert_eq!(
        failed_code(s1.failed.as_ref()),
        Some(("IR0001".into(), None))
    );
    assert!(s1.background.is_none());
    assert_eq!(s1.origin, slidock_ir::origin::Origin::synth("verify"));
    let s2 = source_slide(&presentation, "ir-s2");
    assert_eq!(
        failed_code(s2.failed.as_ref()),
        Some(("IR0001".into(), None))
    );
    let via = verified
        .diagnostics
        .iter()
        .find(|d| d.container == Some(id("ir-s2")))
        .unwrap();
    assert_eq!(data(via, "via").as_deref(), Some("ir-layout"));
}

#[test]
fn verify_source_fails_a_master_with_an_invalid_theme_bare_and_drops_the_theme() {
    let mut input = full_json();
    input["themes"]["ir-theme"]["colors"]["dk1"] = json!({ "kind": "rgb" });
    let verified = verify_source_presentation(&Unverified::Json(input), None);
    let presentation = verified.presentation.unwrap();
    assert!(presentation.themes.is_empty());
    let master = &presentation.masters[&id("ir-master")];
    assert_eq!(
        failed_code(master.failed.as_ref()).map(|f| f.0),
        Some("IR0001".into())
    );
    assert!(master.elements.is_empty());
    assert!(master.theme.is_none());
    assert!(source_slide(&presentation, "ir-s1").failed.is_some());
}

#[test]
fn verify_source_fails_the_presentation_on_a_presentation_level_rule() {
    let mut input = full_json();
    input["slides"][1]["id"] = json!("ir-s1");
    let verified = verify_source_presentation(&Unverified::Json(input), None);
    assert!(verified.presentation.is_none());
    assert!(find(&verified.diagnostics, "IR0907").is_some());
}

#[test]
fn verify_source_fails_the_presentation_when_it_is_not_even_json_of_one() {
    let verified = verify_source_presentation(&Unverified::Json(json!([1, 2])), None);
    assert!(verified.presentation.is_none());
    let failed = find(&verified.diagnostics, "IR0907").unwrap();
    assert_eq!(data(failed, "pass").as_deref(), Some("verify-source"));
}

fn reverse(refs: &mut Vec<slidock_ir::source::Container<'_>>) {
    refs.reverse();
}

/// The passes over `presentation` under `limits`.
fn run_limited(
    passes: &PassManager,
    presentation: &Presentation,
    verify: VerifyMode,
    limits: slidock_core_api::Limits,
) -> slidock_core::RunResult {
    let shared = Shared {
        limits,
        ..Shared::of(presentation)
    };
    let options = slidock_core::RunOptions {
        verify,
        env: shared.env(),
        clock: None,
        deadline: slidock_core_api::Deadline::never(),
        visit_order: None,
        trace: false,
    };
    passes.run(&Unverified::from(presentation.clone()), &options)
}

#[test]
fn leaves_out_the_slides_whose_output_passes_what_the_presentation_has_left() {
    use slidock_core_api::Output;
    let presentation = typed(copy_slide(&full_json(), "ir-s2", "ir-s3", false));
    let count = |c: slidock_ir::source::Container<'_>| Output::of(c);
    let mut kept = Output::default();
    for container in presentation.containers() {
        if container.id().as_str() != "ir-s2" && container.id().as_str() != "ir-s3" {
            kept = kept.plus(&count(container));
        }
    }
    let s2 = count(slidock_ir::source::Container::Slide(slide(
        &presentation,
        "ir-s2",
    )));
    assert!(s2.elements > 0 && s2.text_runs > 0);
    // Room for everything but one copy of ir-s2's elements: ir-s2 is charged before ir-s3, so ir-s3 is left out.
    let mut limits = slidock_core_api::Limits::default();
    limits.output.elements = kept.elements + s2.elements;
    let passes = manager(vec![identity("expand"), claim_all(&[])]);
    for verify in [Each, Boundaries] {
        let result = run_limited(&passes, &presentation, verify, limits);
        let over: Vec<_> = codes(&result.diagnostics)
            .into_iter()
            .filter(|(code, _)| code == "IR0914")
            .collect();
        assert_eq!(over, [("IR0914".to_owned(), Some("ir-s3".to_owned()))]);
        let diagnostic = find(&result.diagnostics, "IR0914").unwrap();
        assert_eq!(
            diagnostic.message,
            format!(
                "The container is left out: with what expand made of it, the presentation's elements pass its \
                 budget of {}",
                limits.output.elements
            )
        );
        let out = resolved(&result);
        assert!(slide(&out, "ir-s3").failed.is_some());
        assert!(slide(&out, "ir-s3").elements.is_empty());
        assert!(slide(&out, "ir-s2").failed.is_none());
        assert_eq!(errors(&result.diagnostics).len(), 1);
    }
    // A run within the budget leaves out nothing.
    limits.output.elements += s2.elements;
    let result = run_limited(&passes, &presentation, Each, limits);
    assert!(find(&result.diagnostics, "IR0914").is_none());
    // So does a budget on runs, segments or stops passed by a presentation pass.
    let mut runs = slidock_core_api::Limits::default();
    runs.output.text_runs = kept.text_runs;
    let result = run_limited(&manager(vec![claim_all(&[])]), &presentation, Each, runs);
    let left: Vec<_> = codes(&result.diagnostics)
        .into_iter()
        .filter(|(code, _)| code == "IR0914")
        .map(|(_, container)| container.unwrap())
        .collect();
    assert_eq!(left, ["ir-s2", "ir-s3"]);
    assert!(
        find(&result.diagnostics, "IR0914")
            .unwrap()
            .message
            .contains("the presentation's text runs pass its budget")
    );
}

#[test]
fn stops_the_passes_once_the_compile_deadline_passes() {
    let presentation = full();
    let passes = manager(vec![identity("first"), claim_all(&[]), identity("last")]);
    let shared = Shared::of(&presentation);
    let run = |deadline| {
        let options = slidock_core::RunOptions {
            deadline,
            ..slidock_core::RunOptions::new(Each, shared.env())
        };
        passes.run(&Unverified::from(presentation.clone()), &options)
    };
    let within = run(slidock_core_api::Deadline::never());
    assert!(within.presentation.is_some());
    assert!(find(&within.diagnostics, "IR0905").is_none());
    // Every check reads the clock, which moves 1 ms a read: the deadline passes after a few containers.
    let clock = Ticks::new(1.0);
    let stopped = run(slidock_core_api::Deadline::start(Some(&clock), 4));
    assert!(stopped.presentation.is_none());
    let timeouts: Vec<&str> = stopped
        .diagnostics
        .iter()
        .filter(|d| d.code == codes::IR0905)
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(timeouts, ["passes stopped: over the compile's 4 ms limit"]);
}
