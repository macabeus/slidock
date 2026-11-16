//! The parity selector for motion: with `native_motion` off (the default, the parity path) a compile and a session
//! leave out the native motion data a frontend lowers, typed or as JSON; on, they keep it.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

mod support;

use slidock_core::{
    CompileOptions, FrontendRegistry, Phase, Pipeline, Session, SessionOptions, compile,
};
use slidock_core_api::Input;
use slidock_io::BytesReader;
use slidock_ir::effects::{BuildEffect, Dir4, Dir4Params, Easing, TransitionEffect};
use slidock_ir::fixture::{Slide, StepBuilder, TextBox, TransitionBuilder, presentation};
use slidock_ir::source::Presentation;
use support::{FakeFrontend, claim_all, id};

/// One slide with a native step (with timing and a background target), a native transition and a match key.
fn native() -> Presentation {
    presentation(&[Slide::new(
        id("ir-s1"),
        vec![
            TextBox::new(id("ir-s1.title"), [60.0, 40.0, 600.0, 60.0], &["Motion"])
                .match_key("title"),
        ],
    )
    .step(
        StepBuilder::entrance(id("ir-s1.title"), BuildEffect::Fade)
            .background()
            .easing(Easing::Out),
    )
    .transition(
        TransitionBuilder::new(TransitionEffect::Push(Dir4Params { dir: Dir4::Left }))
            .easing(Easing::InOut),
    )])
    .unwrap()
}

fn registry(as_json: bool) -> FrontendRegistry {
    let mut frontend = FakeFrontend::new(native());
    frontend.as_json = as_json;
    FrontendRegistry::new(vec![Box::new(frontend)]).unwrap()
}

/// Whether the presentation still holds each piece of native motion data.
fn held(presentation: &Presentation) -> [bool; 6] {
    let slide = &presentation.slides[0];
    let step = &slide.timeline[0];
    let transition = slide.transition.as_ref().unwrap();
    [
        step.effect.native.is_some(),
        step.timing.is_some(),
        step.target.background,
        transition.effect.native.is_some(),
        transition.easing.is_some(),
        slide.elements[0].match_key().is_some(),
    ]
}

fn compiled(registry: &FrontendRegistry, native_motion: bool) -> Presentation {
    let reader = BytesReader::new(b"FAKE presentation".to_vec());
    let options = CompileOptions {
        native_motion,
        ..CompileOptions::new(registry, Phase::Source)
    };
    let result = compile(&Input::new("presentation.fake", &reader), &options);
    result.source.unwrap()
}

#[test]
fn a_compile_keeps_native_motion_only_when_asked() {
    assert!(!CompileOptions::new(&registry(false), Phase::Source).native_motion);
    for as_json in [false, true] {
        let registry = registry(as_json);
        assert_eq!(
            held(&compiled(&registry, true)),
            [true; 6],
            "json {as_json}"
        );
        let parity = compiled(&registry, false);
        assert_eq!(held(&parity), [false; 6], "json {as_json}");
        // What the IR held before the vocabulary stays.
        assert_eq!(
            parity.slides[0].timeline[0].effect.id.to_string(),
            "ir:fade"
        );
        assert_eq!(
            parity.slides[0]
                .transition
                .as_ref()
                .unwrap()
                .effect
                .id
                .to_string(),
            "ir:push"
        );
    }
}

#[test]
fn a_session_keeps_native_motion_only_when_asked() {
    let registry = registry(false);
    let pipeline = Pipeline::new(vec![claim_all(&[])], Vec::new()).unwrap();
    let reader = BytesReader::new(b"FAKE presentation".to_vec());
    let open = |native_motion| {
        Session::open(
            &Input::new("presentation.fake", &reader),
            SessionOptions {
                native_motion,
                ..SessionOptions::new(&registry, &pipeline)
            },
        )
        .unwrap()
    };
    assert!(!SessionOptions::new(&registry, &pipeline).native_motion);
    let mut on = open(true);
    assert!(on.native_motion());
    assert_eq!(
        held(on.finish().unwrap().source.as_ref().unwrap()),
        [true; 6]
    );
    let mut off = open(false);
    assert!(!off.native_motion());
    assert_eq!(
        held(off.finish().unwrap().source.as_ref().unwrap()),
        [false; 6]
    );
}
