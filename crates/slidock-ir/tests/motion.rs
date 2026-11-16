//! The motion fixtures (`slidock_ir::fixture`), the elements' match keys, and the native motion data the parity
//! path strips. The fixture deck is committed as `tests/data/motion.presentation-ir.json` for hosts and players to
//! compile; `UPDATE_GOLDEN=1` rewrites it.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code"
)]

use std::path::Path;

use serde_json::{Value, json};
use slidock_ir::canonical::{to_json, to_json_pretty};
use slidock_ir::effects::{
    BuildEffect, Dir, Dir4, Dir4Params, Distance, Easing, FlyInParams, MorphBy, MorphParams,
    NativeId, SpinParams, Stagger, StaggerAmount, StaggerOrder, TransitionEffect,
    strip_native_motion,
};
use slidock_ir::fixture::{
    DURATION_MS, Slide, StepBuilder, TextBox, TransitionBuilder, presentation, presentation_json,
};
use slidock_ir::formats::{FileFormat, SourceFormat};
use slidock_ir::ids::NodeId;
use slidock_ir::source::Presentation;
use slidock_ir::timeline::{Delivery, StepFallback, StepKind, Trigger};
use slidock_ir::units::{Deg, Ms};
use slidock_ir::validate::validate;
use slidock_ir::verify::{VerifyOptions, verify_source};

/// A presentation's JSON as the canonical text of the typed presentation it reads as.
fn canonical(json: &Value) -> String {
    let typed: Presentation = slidock_ir::parse::from_value(json.clone()).unwrap();
    to_json(&typed).unwrap()
}

fn id(text: &str) -> NodeId {
    NodeId::new(text).unwrap()
}

/// Three slides: builds by paragraph and an emphasis on the first, a morph that pairs a box across the first two,
/// and a push with an exit on the third.
fn slides() -> Vec<Slide> {
    let card = |x: f64| {
        TextBox::new(
            id(&format!("ir-s{}.card", if x < 300.0 { 1 } else { 2 })),
            [x, 260.0, 160.0, 90.0],
            &["Card"],
        )
        .fill([1.0, 0.8, 0.0])
        .match_key("card")
    };
    vec![
        Slide::new(
            id("ir-s1"),
            vec![
                TextBox::new(id("ir-s1.title"), [60.0, 40.0, 600.0, 60.0], &["Motion"]),
                TextBox::new(
                    id("ir-s1.body"),
                    [60.0, 120.0, 600.0, 120.0],
                    &["One", "Two", "Three"],
                ),
                card(60.0),
            ],
        )
        .step(StepBuilder::entrance(id("ir-s1.title"), BuildEffect::Fade))
        .step(
            StepBuilder::entrance(
                id("ir-s1.body"),
                BuildEffect::FlyIn(FlyInParams {
                    dir: Dir::Up,
                    distance: Distance::OffSlide,
                }),
            )
            .delivery(Delivery::ByParagraph)
            .paragraphs(0, 2)
            .stagger(Stagger {
                per_unit: StaggerAmount::Ms(Ms(150)),
                order: StaggerOrder::Forward,
            })
            .easing(Easing::Out),
        )
        .step(
            StepBuilder::emphasis(
                id("ir-s1.card"),
                BuildEffect::Spin(SpinParams {
                    deg: Deg::exact(360.0).unwrap(),
                    cw: true,
                    hold: false,
                }),
            )
            .trigger(Trigger::AfterPrevious)
            .delay(200)
            .duration(1000),
        ),
        Slide::new(
            id("ir-s2"),
            vec![
                TextBox::new(id("ir-s2.title"), [60.0, 40.0, 600.0, 60.0], &["Morph"]),
                card(480.0),
            ],
        )
        .transition(
            TransitionBuilder::new(TransitionEffect::Morph(MorphParams {
                by: MorphBy::Object,
            }))
            .duration(1000),
        ),
        Slide::new(
            id("ir-s3"),
            vec![TextBox::new(
                id("ir-s3.title"),
                [60.0, 40.0, 600.0, 60.0],
                &["Push"],
            )],
        )
        .transition(
            TransitionBuilder::new(TransitionEffect::Push(Dir4Params { dir: Dir4::Left }))
                .easing(Easing::InOut)
                .advance(true, Some(3000)),
        )
        .step(StepBuilder::opaque(
            id("ir-s3.title"),
            StepKind::Exit,
            NativeId::new(SourceFormat::File(FileFormat::Keynote), "apple:shimmer"),
            StepFallback::Dissolve,
        )),
    ]
}

#[test]
fn the_fixture_deck_verifies_and_matches_its_golden() {
    let presentation = presentation(&slides()).unwrap();
    let issues = validate(&presentation);
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(
        verify_source(&presentation, &VerifyOptions::default()),
        Vec::new()
    );
    let text = format!("{}\n", to_json_pretty(&presentation).unwrap());
    // The builder's JSON reads back as itself.
    let json = presentation_json(&slides()).unwrap();
    assert_eq!(canonical(&json), to_json(&presentation).unwrap());
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/motion.presentation-ir.json");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        text,
        "motion.presentation-ir.json drifted (UPDATE_GOLDEN=1 rewrites it)"
    );
}

#[test]
fn builders_default_to_a_click_step_of_the_effects_own_fallback() {
    let step = StepBuilder::exit(id("ir-s1.title"), BuildEffect::Fade).build();
    assert_eq!(step.trigger, Trigger::Click);
    assert_eq!((step.delay_ms, step.duration_ms), (Ms(0), Ms(DURATION_MS)));
    assert_eq!(step.effect.id.to_string(), "ir:fade");
    assert_eq!(
        step.effect.fallback,
        BuildEffect::Fade.fallback(StepKind::Exit)
    );
    assert_eq!(step.timing, None);
    let native = step.effect.native.as_ref().unwrap();
    assert_eq!(
        (native.id.to_string(), native.effect.clone()),
        ("ir:fade".to_owned(), BuildEffect::Fade)
    );
    let named = StepBuilder::exit(id("ir-s1.title"), BuildEffect::Fade)
        .native_id(NativeId::new(
            SourceFormat::File(FileFormat::Pptx),
            "exit.10",
        ))
        .build();
    assert_eq!(named.effect.id.to_string(), "pptx:exit.10");
    assert_eq!(named.effect.native.unwrap().id.to_string(), "pptx:exit.10");
    let transition = TransitionBuilder::new(TransitionEffect::Dissolve).build();
    assert_eq!(transition.effect.id.to_string(), "ir:dissolve");
    assert!(transition.advance.unwrap().on_click);
}

/// The fixture deck's JSON with a group (a matched child inside) on slide one and a matched element on a layout.
fn with_groups(mut json: Value) -> Value {
    let child = presentation_json(&[Slide::new(
        id("ir-s1"),
        vec![TextBox::new(id("ir-s1.group.dot"), [0.0, 0.0, 20.0, 20.0], &["."]).match_key("dot")],
    )])
    .unwrap()["slides"][0]["elements"][0]
        .clone();
    let group = json!({ "id": "ir-s1.group", "origin": { "kind": "agent" }, "kind": "group", "matchKey": "group",
                        "children": [child] });
    json["slides"][0]["elements"]
        .as_array_mut()
        .unwrap()
        .push(group);
    json
}

fn keys(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            for (key, inner) in object {
                out.push(key.clone());
                keys(inner, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| keys(item, out)),
        _ => {}
    }
}

#[test]
fn the_parity_path_strips_every_native_motion_field() {
    let mut rich = with_groups(presentation_json(&slides()).unwrap());
    // A layout's and a master's elements carry keys too.
    let element = |id: &str, key: &str| {
        let mut element = presentation_json(&[Slide::new(
            NodeId::new(id).unwrap(),
            vec![
                TextBox::new(NodeId::new(id).unwrap(), [0.0, 0.0, 20.0, 20.0], &["."])
                    .match_key(key),
            ],
        )])
        .unwrap()["slides"][0]["elements"][0]
            .clone();
        element["id"] = json!(id);
        element
    };
    rich["masters"] = json!({ "ir-m1": { "id": "ir-m1", "origin": { "kind": "agent" },
                                         "elements": [element("ir-m1.logo", "logo")] } });
    rich["layouts"] = json!({ "ir-l1": { "id": "ir-l1", "origin": { "kind": "agent" }, "showMaster": true,
                                         "elements": [element("ir-l1.band", "band")] } });
    let typed: Presentation = slidock_ir::parse::from_value(rich.clone()).unwrap();
    let mut stripped = typed.clone();
    strip_native_motion(&mut stripped);
    let stripped_json: Value = serde_json::from_str(&to_json(&stripped).unwrap()).unwrap();
    let mut names = Vec::new();
    keys(&stripped_json, &mut names);
    for gone in ["native", "timing", "matchKey", "easing", "background"] {
        assert!(!names.iter().any(|name| name == gone), "{gone} left");
    }
    let mut before = Vec::new();
    keys(&rich, &mut before);
    assert_eq!(before.iter().filter(|name| *name == "matchKey").count(), 6);
    // What the IR held before the vocabulary stays: ids, fallbacks, triggers, paragraphs, durations, advance.
    let step = &stripped_json["slides"][0]["timeline"][1];
    assert_eq!(
        step,
        &json!({ "target": { "element": "ir-s1.body", "paragraphs": [0, 2] }, "trigger": "click", "delayMs": 0,
                 "durationMs": 500, "kind": "entrance", "effect": { "id": "ir:flyIn", "fallback": "dissolve" },
                 "delivery": "byParagraph" })
    );
    assert_eq!(
        stripped_json["slides"][2]["transition"],
        json!({ "effect": { "id": "ir:push", "fallback": "push" }, "durationMs": 500,
                "advance": { "onClick": true, "afterMs": 3000 } })
    );
    // Stripping what holds none changes nothing.
    let mut again = stripped.clone();
    strip_native_motion(&mut again);
    assert_eq!(again, stripped);
}

#[test]
fn match_keys_round_trip_within_their_bounds() {
    let rich = with_groups(presentation_json(&slides()).unwrap());
    let typed: Presentation = slidock_ir::parse::from_value(rich.clone()).unwrap();
    assert_eq!(to_json(&typed).unwrap(), canonical(&rich));
    let card = &typed.slides[0].elements[2];
    assert_eq!(card.match_key(), Some("card"));
    for (key, issue) in [
        ("", Some("IR0001")),
        (&"k".repeat(256)[..], None),
        (&"k".repeat(257)[..], Some("IR0008")),
    ] {
        let mut changed = typed.clone();
        *changed.slides[0].elements[2].match_key_mut() = Some(key.to_owned());
        let issues = validate(&changed);
        match issue {
            None => assert!(issues.is_empty(), "{issues:?}"),
            Some(code) => {
                assert_eq!(issues.len(), 1, "{issues:?}");
                assert_eq!(issues[0].code.code(), code);
                assert!(
                    issues[0].path.to_string().ends_with("matchKey"),
                    "{}",
                    issues[0].path
                );
            }
        }
    }
}

/// The readers' bounds are not the validator's: a timeline longer than a reader lowers still validates, as the IR's
/// schema has it, so the parity path refuses nothing it accepted.
#[test]
fn the_timeline_length_is_a_readers_bound() {
    let mut slide = Slide::new(
        id("ir-s1"),
        vec![TextBox::new(
            id("ir-s1.title"),
            [0.0, 0.0, 100.0, 40.0],
            &["T"],
        )],
    );
    for _ in 0..=slidock_ir::effects::limits::STEPS_PER_SLIDE {
        slide = slide.step(StepBuilder::entrance(id("ir-s1.title"), BuildEffect::Fade));
    }
    let presentation = presentation(&[slide]).unwrap();
    assert!(presentation.slides[0].timeline.len() > slidock_ir::effects::limits::STEPS_PER_SLIDE);
    assert_eq!(validate(&presentation), []);
}
