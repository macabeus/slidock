//! The native motion data off the parity path: a slide document's timeline template (every step field), the
//! elements' match keys, the fallback notes (HTML6010) and the deck JSON's steps and transition manifest. The parity
//! path prints none of it, whatever the IR holds. Goldens are under `tests/golden/motion/`; `UPDATE_GOLDEN=1`
//! rewrites them.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::path::{Path, PathBuf};

use serde_json::Value;
use slidock_html::deck::{DeckInput, build_deck};
use slidock_html::timeline::{TIMELINE, timeline_template};
use slidock_html::{
    EmitOptions, Emitted, FontStacks, Motion, emit_documents, emit_documents_shipping,
};
use slidock_ir::color::MappedSlot;
use slidock_ir::diagnostics::codes;
use slidock_ir::effects::{
    AfterEffect, BuildEffect, ColorChangeParams, ColorMix, ColorProperty, ColorValue, Dir, Dir4,
    Dir4Params, Distance, Easing, FlyInParams, Frame, GrowShrinkParams, HexColor, HslShift,
    Keyframes, NativeId, PathParams, PathSeg, PlayParams, RepeatParams, Scale, SetTrack, SetValue,
    Stagger, StaggerAmount, StaggerOrder, Track, TransitionEffect,
};
use slidock_ir::fixture::{StepBuilder, TransitionBuilder};
use slidock_ir::formats::{FileFormat, SourceFormat};
use slidock_ir::paint::ResolvedFill;
use slidock_ir::resolved::{Element, Presentation};
use slidock_ir::timeline::{Delivery, StepFallback, StepKind, TimelineStep, Trigger};
use slidock_ir::units::{Deg, Ms, Px, Ratio, UnitRatio};
use support::{elements_mut, emit, full, id, provenance};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/motion")
}

/// Compares `actual` with the golden file `name`, or rewrites it under `UPDATE_GOLDEN=1`.
fn golden(name: &str, actual: &str) {
    let path = golden_dir().join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (UPDATE_GOLDEN=1 writes it)", path.display()));
    assert_eq!(
        actual, expected,
        "{name} drifted (UPDATE_GOLDEN=1 rewrites it)"
    );
}

fn ratio(x: f64) -> Ratio {
    Ratio::exact(x).unwrap()
}

fn unit(x: f64) -> UnitRatio {
    UnitRatio::exact(x).unwrap()
}

fn pptx(name: &str) -> NativeId {
    NativeId::new(SourceFormat::File(FileFormat::Pptx), name)
}

/// One step per field the template prints: effects with scalar, structured and keyframe parameters, every trigger,
/// paragraph ranges and a background target, stagger in milliseconds and as a fraction, offsets, easings, repeats,
/// auto-reverse, after-effects, and an opaque step without an effect in the vocabulary.
fn steps() -> Vec<TimelineStep> {
    let fly_in = BuildEffect::FlyIn(FlyInParams {
        dir: Dir::Up,
        distance: Distance::OffSlide,
    });
    let path = BuildEffect::Path(PathParams {
        d: vec![
            PathSeg::MoveTo(Px::exact(0.0).unwrap(), Px::exact(0.0).unwrap()),
            PathSeg::CurveTo(
                Px::exact(40.0).unwrap(),
                Px::exact(-20.0).unwrap(),
                Px::exact(80.5).unwrap(),
                Px::exact(20.0).unwrap(),
                Px::exact(120.0).unwrap(),
                Px::exact(0.0).unwrap(),
            ),
        ],
        auto_rotate: true,
        hold: true,
    });
    let behaviors = BuildEffect::Behaviors(vec![Keyframes::Opacity(Track {
        begin_ms: Ms(0),
        duration_ms: Ms(400),
        easing: None,
        frames: vec![
            Frame {
                offset: unit(0.0),
                value: unit(1.0),
            },
            Frame {
                offset: unit(1.0),
                value: unit(0.25),
            },
        ],
    })]);
    vec![
        StepBuilder::entrance(id("ir-s1.body"), fly_in)
            .native_id(pptx("entr.2/4"))
            .delivery(Delivery::ByParagraph)
            .paragraphs(0, 1)
            .stagger(Stagger {
                per_unit: StaggerAmount::Ms(Ms(120)),
                order: StaggerOrder::Forward,
            })
            .offset(0)
            .easing(Easing::Out)
            .build(),
        StepBuilder::emphasis(
            id("ir-s1.title"),
            BuildEffect::Blink(RepeatParams { repeat: 3 }),
        )
        .native_id(pptx("emph.35"))
        .trigger(Trigger::WithPrevious)
        .duration(1000)
        .repeat(slidock_ir::effects::Repeat::UntilNextClick)
        .auto_reverse()
        .after(AfterEffect::Dim(
            slidock_ir::effects::HexColor::parse("#808080").unwrap(),
        ))
        .build(),
        StepBuilder::new(id("ir-s1.arrow"), StepKind::Motion, path)
            .native_id(pptx("path.0"))
            .trigger(Trigger::AfterPrevious)
            .delay(250)
            .easing(Easing::Bezier(
                ratio(0.25),
                ratio(0.1),
                ratio(0.25),
                ratio(1.0),
            ))
            .offset(1250)
            .build(),
        StepBuilder::emphasis(
            id("ir-s1.photo"),
            BuildEffect::GrowShrink(GrowShrinkParams {
                scale: Scale {
                    x: ratio(1.5),
                    y: ratio(0.5),
                },
                hold: false,
            }),
        )
        .native_id(pptx("emph.6"))
        .background()
        .repeat(slidock_ir::effects::Repeat::Count(ratio(2.5)))
        .after(AfterEffect::Hide)
        .build(),
        StepBuilder::entrance(id("ir-s1.group"), behaviors)
            .native_id(pptx("entr.99"))
            .delivery(Delivery::ByWord)
            .stagger(Stagger {
                per_unit: StaggerAmount::Fraction(unit(0.1)),
                order: StaggerOrder::Random(7),
            })
            .easing(Easing::AccelDecel {
                accel: unit(0.2),
                decel: unit(0.3),
            })
            .build(),
        StepBuilder::new(
            id("ir-s1.video"),
            StepKind::Media,
            BuildEffect::Play(PlayParams { from: Ms(1000) }),
        )
        .native_id(NativeId::new(
            SourceFormat::File(FileFormat::Keynote),
            "apple:movie-start",
        ))
        .build(),
        StepBuilder::opaque(
            id("ir-s1.table"),
            StepKind::Exit,
            NativeId::new(SourceFormat::File(FileFormat::Keynote), "apple:shimmer"),
            StepFallback::Dissolve,
        )
        .trigger(Trigger::Click)
        .delivery(Delivery::ByChar)
        .build(),
    ]
}

/// The printer tests' fixture with native motion on slide one: [`steps`], a push transition, and match keys on the
/// title and on a group's nested child.
fn motion() -> Presentation {
    let mut presentation = full();
    let slide = &mut presentation.slides[0];
    slide.timeline = steps();
    slide.transition = Some(
        TransitionBuilder::new(TransitionEffect::Push(Dir4Params { dir: Dir4::Left }))
            .native_id(pptx("push"))
            .duration(700)
            .easing(Easing::InOut)
            .advance(true, Some(3000))
            .build(),
    );
    for element in elements_mut(&mut presentation, 0) {
        match element {
            Element::Shape(shape) if shape.id.as_str() == "ir-s1.title" => {
                shape.match_key = Some("title".to_owned());
            }
            Element::Group(group) => {
                for child in &mut group.children {
                    *child.match_key_mut() = Some(format!("dot:{}", child.id()));
                }
            }
            _ => {}
        }
    }
    presentation
}

fn emit_motion(presentation: &Presentation, motion: Motion) -> Emitted {
    emit_documents_shipping(
        presentation,
        &EmitOptions::new(provenance()),
        &FontStacks::new(),
        motion,
    )
    .unwrap()
}

const SLIDE_ONE: &str = "slides/ir-s1.html";

#[test]
fn the_timeline_template_prints_every_step_field() {
    let emitted = emit_motion(&motion(), Motion::On);
    let html = &emitted.files.text[SLIDE_ONE];
    golden("ir-s1.html", html);
    let head = &html[..html.find("</head>").unwrap()];
    assert!(
        head.contains(&format!("<template {TIMELINE}><ol>\n<li ")),
        "{head}"
    );
    assert_eq!(head.matches("<li ").count(), steps().len());
    for expected in [
        "data-target=\"ir-s1.body\" data-trigger=\"click\" data-kind=\"entrance\" data-effect=\"flyIn\" \
         data-dir=\"u\" data-distance=\"offSlide\" data-native=\"pptx:entr.2/4\" data-fallback=\"dissolve\" \
         data-delivery=\"byParagraph\" data-paragraphs=\"0-1\" data-stagger-ms=\"120\" \
         data-stagger-order=\"forward\" data-offset-ms=\"0\" data-delay-ms=\"0\" data-duration-ms=\"500\" \
         data-easing=\"out\"",
        "data-effect=\"blink\" data-effect-repeat=\"3\"",
        "data-repeat=\"untilNextClick\" data-auto-reverse data-after=\"{&#x22;dim&#x22;:&#x22;#808080&#x22;}\"",
        "data-trigger=\"afterPrevious\" data-kind=\"motion\" data-effect=\"path\" \
         data-d=\"[[&#x22;M&#x22;,0,0],[&#x22;C&#x22;,40,-20,80.5,20,120,0]]\" data-auto-rotate=\"true\" \
         data-hold=\"true\"",
        "data-offset-ms=\"1250\" data-delay-ms=\"250\"",
        "data-easing=\"{&#x22;bezier&#x22;:[0.25,0.1,0.25,1]}\"",
        "data-scale=\"{&#x22;x&#x22;:1.5,&#x22;y&#x22;:0.5}\" data-hold=\"false\"",
        "data-background data-delay-ms",
        "data-repeat=\"{&#x22;count&#x22;:2.5}\"",
        "data-after=\"hide\"",
        "data-effect=\"behaviors\" data-params=\"[{",
        "data-stagger-fraction=\"0.1\" data-stagger-order=\"{&#x22;random&#x22;:7}\"",
        "data-easing=\"{&#x22;accelDecel&#x22;:{&#x22;accel&#x22;:0.2,&#x22;decel&#x22;:0.3}}\"",
        "data-kind=\"media\" data-effect=\"play\" data-from=\"1000\" data-native=\"keynote:apple:movie-start\"",
        "data-target=\"ir-s1.table\" data-trigger=\"click\" data-kind=\"exit\" \
         data-native=\"keynote:apple:shimmer\" data-fallback=\"dissolve\" data-delivery=\"byChar\" \
         data-delay-ms=\"0\" data-duration-ms=\"500\"></li>",
    ] {
        assert!(head.contains(expected), "missing {expected}\n{head}");
    }
    // The body is the fully built slide: no step reaches it.
    let body = &html[html.find("<body>").unwrap()..];
    assert!(!body.contains(TIMELINE) && !body.contains("data-trigger"));
    // The set lints clean with the template and the match keys in it.
    assert_eq!(emitted.files.lint(), Vec::new());
}

#[test]
fn elements_carry_their_match_keys() {
    let html = emit_motion(&motion(), Motion::On).files.text[SLIDE_ONE].clone();
    assert!(
        html.contains("data-el=\"ir-s1.title\" data-kind=\"shape\" data-sd-match=\"title\""),
        "{html}"
    );
    assert!(html.contains(
        "data-el=\"ir-s1.group.dot\" data-kind=\"shape\" data-sd-match=\"dot:ir-s1.group.dot\""
    ));
    assert!(html.contains("data-sd-match=\"dot:ir-s1.group.inner\""));
    assert_eq!(html.matches("data-sd-match=").count(), 3);
}

#[test]
fn steps_and_transitions_without_a_native_effect_note_their_fallback() {
    let emitted = emit_motion(&motion(), Motion::On);
    let notes: Vec<_> = emitted
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == codes::HTML6010)
        .collect();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].node.as_ref().unwrap().as_str(), "ir-s1.table");
    assert_eq!(
        notes[0].message,
        "The keynote:apple:shimmer effect prints only its dissolve fallback"
    );
    // The fixture's own transition and step have no native effect: each prints only its fallback.
    let plain = emit_motion(&full(), Motion::On);
    let codes: Vec<(String, Option<String>)> = plain
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == codes::HTML6010)
        .map(|diagnostic| {
            (
                diagnostic.message.clone(),
                diagnostic.node.as_ref().map(ToString::to_string),
            )
        })
        .collect();
    assert_eq!(
        codes,
        [
            (
                "The keynote:dissolve effect prints only its dissolve fallback".to_owned(),
                Some("ir-s1.body".to_owned())
            ),
            (
                "The pptx:fade effect prints only its dissolve fallback".to_owned(),
                None
            ),
        ]
    );
    // One note per step and transition that prints its fallback, repeated effects included.
    let mut repeated = full();
    let mut again = repeated.slides[0].clone();
    again.id = id("ir-s9");
    again.timeline.extend(repeated.slides[0].timeline.clone());
    repeated.slides.push(again);
    let notes: Vec<_> = emit_motion(&repeated, Motion::On)
        .diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.code == codes::HTML6010)
        .map(|diagnostic| {
            let data = diagnostic.data.unwrap();
            assert!(data.get("count").is_none());
            (
                diagnostic.container.unwrap().to_string(),
                data["effect"].to_js_string(),
            )
        })
        .collect();
    assert_eq!(
        notes,
        [
            ("ir-s1".into(), "keynote:dissolve".into()),
            ("ir-s1".into(), "pptx:fade".into()),
            ("ir-s9".into(), "keynote:dissolve".into()),
            ("ir-s9".into(), "keynote:dissolve".into()),
            ("ir-s9".into(), "pptx:fade".into()),
        ] as [(String, String); 5]
    );
}

/// [`motion`] without its native motion data, as the parity path compiles it.
fn stripped(mut presentation: Presentation) -> Presentation {
    for slide in &mut presentation.slides {
        for step in &mut slide.timeline {
            step.effect.native = None;
            step.timing = None;
            step.target.background = false;
        }
        if let Some(transition) = &mut slide.transition {
            transition.effect.native = None;
            transition.easing = None;
        }
    }
    let mut elements: Vec<&mut Element> = elements_mut(&mut presentation, 0).iter_mut().collect();
    while let Some(element) = elements.pop() {
        *element.match_key_mut() = None;
        if let Element::Group(group) = element {
            elements.extend(group.children.iter_mut());
        }
    }
    presentation
}

#[test]
fn the_parity_path_prints_none_of_it() {
    let rich = motion();
    let parity = emit_documents(&rich, &EmitOptions::new(provenance())).unwrap();
    // The documents of the IR with native motion are those of the IR without it, byte for byte.
    assert_eq!(parity, emit(&stripped(rich.clone())));
    assert_eq!(emit_motion(&rich, Motion::Off), parity);
    let html = &parity.files.text[SLIDE_ONE];
    assert!(!html.contains(TIMELINE) && !html.contains("data-sd-match"));
    assert!(
        parity
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != codes::HTML6010)
    );
    // On, only the slide documents change: the layouts and the manifest stay.
    let on = emit_motion(&rich, Motion::On);
    for (path, text) in &parity.files.text {
        assert_eq!(text == &on.files.text[path], path != SLIDE_ONE, "{path}");
    }
}

#[test]
fn the_template_marker_takes_the_format_prefix() {
    assert_eq!(
        TIMELINE,
        format!("{}timeline", slidock_format::markers::DATA_PREFIX)
    );
}

#[test]
fn a_slide_without_steps_prints_no_template() {
    assert_eq!(timeline_template(&[]).unwrap(), None);
    let mut presentation = motion();
    presentation.slides[0].timeline.clear();
    let html = &emit_motion(&presentation, Motion::On).files.text[SLIDE_ONE];
    assert!(!html.contains(TIMELINE));
}

/// The golden effects of `slidock-ir`, as steps of a kind each can play.
fn golden_builds() -> Vec<BuildEffect> {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../slidock-ir/tests/data/effects.json"),
    )
    .unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    serde_json::from_value(value["builds"].clone()).unwrap()
}

#[test]
fn every_effect_prints_its_parameters() {
    let builds = golden_builds();
    assert_eq!(builds.len(), 54);
    for effect in builds {
        let kind = [
            StepKind::Entrance,
            StepKind::Emphasis,
            StepKind::Motion,
            StepKind::Media,
        ]
        .into_iter()
        .find(|kind| effect.plays_in(*kind))
        .unwrap();
        let params = serde_json::to_value(&effect).unwrap()["params"].clone();
        let step = StepBuilder::new(id("ir-s1.body"), kind, effect.clone()).build();
        let template = timeline_template(&[step]).unwrap().unwrap();
        let html = slidock_html::serialize::fragment(&template);
        assert!(
            html.contains(&format!("data-effect=\"{}\"", effect.name())),
            "{html}"
        );
        // One attribute per parameter (or one for the keyframes), none lost.
        let printed = html.matches("=\"").count();
        let fixed = 9;
        let expected = match &params {
            Value::Object(params) => params.len(),
            Value::Null => 0,
            _ => 1,
        };
        assert_eq!(printed, fixed + expected, "{}: {html}", effect.name());
    }
}

fn deck_slide(presentation: &Presentation) -> Value {
    let emitted = emit_motion(presentation, Motion::On);
    let deck = build_deck(&DeckInput {
        presentation,
        documents: &emitted.files,
        diagnostics: &emitted.diagnostics,
        source: &provenance(),
        fonts: &[],
    })
    .unwrap();
    serde_json::to_value(&deck.slides[0]).unwrap()
}

#[test]
fn the_deck_json_carries_every_step_field_and_the_transition_manifest() {
    let slide = deck_slide(&motion());
    let manifest = serde_json::json!({ "timeline": slide["timeline"], "transition": slide["transition"],
                                        "matchKeys": slide["matchKeys"] });
    golden(
        "deck-slide.json",
        &format!(
            "{}\n",
            slidock_canon::to_canonical_json(&manifest, slidock_canon::Layout::Indented).unwrap()
        ),
    );
    assert_eq!(
        slide["transition"],
        serde_json::json!({
            "native": "pptx:push", "fallback": "push", "durationMs": 700,
            "advance": { "onClick": true, "afterMs": 3000 },
            "effect": "push", "params": { "dir": "l" }, "easing": "in-out"
        })
    );
    assert_eq!(
        slide["timeline"][6],
        serde_json::json!({
            "target": { "element": "ir-s1.table", "paragraphs": null }, "trigger": "click", "kind": "exit",
            "delivery": "byChar", "delayMs": 0, "durationMs": 500, "native": "keynote:apple:shimmer",
            "fallback": "dissolve"
        })
    );
    // The match keys of the slide's own elements, group children included, by element id.
    assert_eq!(
        slide["matchKeys"],
        serde_json::json!({ "ir-s1.title": "title", "ir-s1.group.dot": "dot:ir-s1.group.dot",
                            "ir-s1.group.inner": "dot:ir-s1.group.inner" })
    );
    // A slide without keys prints none.
    assert!(deck_slide(&full()).get("matchKeys").is_none());
}

/// A colour shift track (an HSL turn) of `make`'s property.
fn shift_track(h: f64) -> Track<ColorValue> {
    Track {
        begin_ms: Ms(0),
        duration_ms: Ms(500),
        easing: None,
        frames: vec![
            Frame {
                offset: unit(0.0),
                value: ColorValue::Shift(HslShift::NONE),
            },
            Frame {
                offset: unit(1.0),
                value: ColorValue::Shift(HslShift {
                    h: Deg::exact(h).unwrap(),
                    s: Ratio::ZERO,
                    l: Ratio::ZERO,
                }),
            },
        ],
    }
}

fn switch_track(on: bool) -> Track<bool> {
    Track {
        begin_ms: Ms(0),
        duration_ms: Ms(0),
        easing: None,
        frames: vec![
            Frame {
                offset: unit(0.0),
                value: on,
            },
            Frame {
                offset: unit(1.0),
                value: on,
            },
        ],
    }
}

/// The printer fixture with colour and font steps: the title (one colour, one weight, a solid fill, no line) takes a
/// fill change, a text colour turn and a bold switch, a line colour change it cannot play (the effect turns on a line
/// it lacks) and a text and line turn whose line track is left out (a shift leaves a missing line off); the arrow (a
/// gradient fill, a solid line) a line colour turn and a fill turn it cannot play; the body (text of two colours, no
/// fill) a text colour change and a fill turn it cannot play.
fn painted() -> Presentation {
    let mut presentation = full();
    for element in elements_mut(&mut presentation, 0) {
        if let Element::Shape(shape) = element
            && shape.id.as_str() == "ir-s1.title"
        {
            shape.fill = Some(ResolvedFill::Solid {
                color: support::theme_color(MappedSlot::Accent1, support::accent1()),
            });
        }
    }
    let change = |property| {
        BuildEffect::ColorChange(ColorChangeParams {
            property,
            to: HexColor::rgb(0xe0, 0x44, 0x2e),
        })
    };
    presentation.slides[0].timeline = vec![
        StepBuilder::emphasis(id("ir-s1.title"), change(ColorProperty::Fill))
            .native_id(pptx("emph.1/2"))
            .build(),
        StepBuilder::emphasis(
            id("ir-s1.title"),
            BuildEffect::Behaviors(vec![
                Keyframes::Text(shift_track(120.0)),
                Keyframes::Bold(switch_track(true)),
            ]),
        )
        .native_id(pptx("emph.31"))
        .build(),
        StepBuilder::emphasis(
            id("ir-s1.arrow"),
            BuildEffect::Behaviors(vec![Keyframes::Line(shift_track(180.0))]),
        )
        .native_id(pptx("emph.23"))
        .build(),
        StepBuilder::emphasis(
            id("ir-s1.arrow"),
            BuildEffect::Behaviors(vec![Keyframes::Fill(shift_track(180.0))]),
        )
        .native_id(pptx("emph.21"))
        .build(),
        StepBuilder::emphasis(id("ir-s1.body"), change(ColorProperty::Text))
            .native_id(pptx("emph.3/2"))
            .build(),
        StepBuilder::emphasis(id("ir-s1.title"), change(ColorProperty::Line))
            .native_id(pptx("emph.7/2"))
            .build(),
        StepBuilder::emphasis(
            id("ir-s1.title"),
            BuildEffect::Behaviors(vec![
                Keyframes::Text(shift_track(180.0)),
                Keyframes::Line(shift_track(180.0)),
            ]),
        )
        .native_id(pptx("emph.23"))
        .build(),
        StepBuilder::emphasis(
            id("ir-s1.body"),
            BuildEffect::Behaviors(vec![Keyframes::Fill(shift_track(180.0))]),
        )
        .native_id(pptx("emph.21"))
        .build(),
    ];
    presentation
}

/// The element of a slide document whose `data-el` is `id`: its start tag.
fn start_tag<'h>(html: &'h str, id: &str) -> &'h str {
    let at = html.find(&format!("data-el=\"{id}\"")).unwrap();
    let start = html[..at].rfind('<').unwrap();
    let end = at + html[at..].find('>').unwrap();
    &html[start..=end]
}

/// An element a colour or font step animates declares that paint as a custom property, which
/// its paint reads; nothing else does, and the parity path prints none of it. A step whose target has no single
/// paint to expose plays its fallback, noted (HTML6011, HTML6010); the deck computes the shifts from the paint.
#[test]
fn colour_and_font_steps_hook_only_their_targets() {
    let presentation = painted();
    let on = emit_motion(&presentation, Motion::On);
    let html = &on.files.text[SLIDE_ONE];
    let body = &html[html.find("<body>").unwrap()..];
    let title = start_tag(body, "ir-s1.title");
    assert!(
        title.contains("--sd-fill:var(--sd-color-accent1,#4472c4);--sd-color:")
            && title.contains(";--sd-bold:normal;")
            && title.contains("background-color:var(--sd-fill,var(--sd-color-accent1,#4472c4))"),
        "{title}"
    );
    assert!(body.contains("color:var(--sd-color,"), "{body}");
    assert!(body.contains("font-weight:var(--sd-bold,normal)"), "{body}");
    let arrow = start_tag(body, "ir-s1.arrow");
    assert!(arrow.contains("--sd-line:"), "{arrow}");
    assert!(!arrow.contains("--sd-fill"), "{arrow}");
    assert!(body.contains("stroke:var(--sd-line,"), "{body}");
    let text = start_tag(body, "ir-s1.body");
    assert!(
        !text.contains("--sd-color:") && !text.contains("var(--sd-color,"),
        "{text}"
    );
    // One declaration per hooked paint, on its element alone.
    for (declaration, count) in [
        ("--sd-fill:", 1),
        ("--sd-line:", 1),
        ("--sd-color:", 1),
        ("--sd-bold:", 1),
        ("--sd-italic:", 0),
        ("--sd-underline:", 0),
    ] {
        assert_eq!(body.matches(declaration).count(), count, "{declaration}");
    }
    assert_eq!(on.files.lint(), Vec::new());

    // The parity path prints none of it: its documents are those of the IR without the native effects.
    let off = emit_motion(&presentation, Motion::Off);
    assert_eq!(off, emit(&stripped(presentation.clone())));
    for marker in ["--sd-fill", "--sd-line", "var(--sd-color,", "--sd-bold"] {
        assert!(
            off.files.text.values().all(|text| !text.contains(marker)),
            "{marker}"
        );
    }

    // The unhooked steps play their fallback, noted with the paint and the reason.
    let unhooked: Vec<(String, String, String)> = on
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == codes::HTML6011)
        .map(|diagnostic| {
            let data = diagnostic.data.as_ref().unwrap();
            (
                diagnostic.node.as_ref().unwrap().to_string(),
                data["property"].to_js_string(),
                data["reason"].to_js_string(),
            )
        })
        .collect();
    assert_eq!(
        unhooked,
        [
            (
                "ir-s1.arrow".to_owned(),
                "fill".to_owned(),
                "its fill is not one solid colour".to_owned()
            ),
            (
                "ir-s1.body".to_owned(),
                "text colour".to_owned(),
                "its text has more than one colour".to_owned()
            ),
            (
                "ir-s1.title".to_owned(),
                "line".to_owned(),
                "it has no line, which the effect turns on".to_owned()
            ),
            (
                "ir-s1.body".to_owned(),
                "fill".to_owned(),
                "it has no fill, which the effect turns on".to_owned()
            ),
        ]
    );
    let fallbacks = on
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == codes::HTML6010 && diagnostic.node.is_some())
        .count();
    assert_eq!(fallbacks, 4);

    // The deck: shifts computed from the paint, the unhooked steps without an effect.
    let slide = deck_slide(&presentation);
    let timeline = slide["timeline"].as_array().unwrap();
    assert_eq!(timeline[0]["effect"], "colorChange");
    let text = &timeline[1]["params"][0];
    assert_eq!(text["property"], "text");
    // The title's text is tx2 (#44546a): its own colour, then turned 120°.
    let turned = HslShift {
        h: Deg::exact(120.0).unwrap(),
        s: Ratio::ZERO,
        l: Ratio::ZERO,
    }
    .apply(HexColor::rgb(0x44, 0x54, 0x6a));
    assert_eq!(text["frames"][0]["value"], "#44546a");
    assert_eq!(text["frames"][1]["value"], turned.to_string());
    assert!(
        text["frames"]
            .as_array()
            .unwrap()
            .iter()
            .all(|frame| frame["value"].is_string()),
        "{text}"
    );
    // The arrow's line is tx1, black: turning its hue leaves it black.
    assert_eq!(
        timeline[2]["params"][0]["frames"],
        serde_json::json!([{ "offset": 0.0, "value": "#000000" }, { "offset": 1.0, "value": "#000000" }])
    );
    for i in [3, 4, 5, 7] {
        assert!(timeline[i].get("effect").is_none(), "{}", timeline[i]);
    }
    // The title's missing line is left off: the turn plays its text track alone.
    let kept: Vec<&Value> = timeline[6]["params"]
        .as_array()
        .unwrap()
        .iter()
        .map(|track| &track["property"])
        .collect();
    assert_eq!(kept, [&serde_json::json!("text")]);
    // The timeline template prints the same steps the deck plays.
    let head = &html[..html.find("</head>").unwrap()];
    assert_eq!(head.matches("data-effect=").count(), 4, "{head}");
    assert!(!head.contains("&#x22;h&#x22;"), "{head}");
}

/// A switch (`set`) needs the hook of the property it switches, as a track of it does, and its colour shifts are
/// computed from the paint: the title switches italic and its text back to its own colour; the body switches on a
/// fill it lacks, which plays the fallback (HTML6011).
#[test]
fn switches_hook_their_property_and_resolve_their_colour() {
    let mut presentation = painted();
    let set = |begin: u32, to: SetValue| {
        Keyframes::Set(SetTrack {
            begin_ms: Ms(begin),
            duration_ms: Ms(0),
            to,
        })
    };
    presentation.slides[0].timeline = vec![
        StepBuilder::emphasis(
            id("ir-s1.title"),
            BuildEffect::Behaviors(vec![
                set(0, SetValue::Italic(true)),
                set(250, SetValue::Text(ColorValue::Shift(HslShift::NONE))),
            ]),
        )
        .native_id(pptx("emph.5"))
        .build(),
        StepBuilder::emphasis(
            id("ir-s1.body"),
            BuildEffect::Behaviors(vec![set(
                0,
                SetValue::Fill(ColorValue::Hex(HexColor::rgb(0x33, 0x33, 0x99))),
            )]),
        )
        .native_id(pptx("emph.16"))
        .build(),
    ];
    let on = emit_motion(&presentation, Motion::On);
    let html = &on.files.text[SLIDE_ONE];
    let body = &html[html.find("<body>").unwrap()..];
    let title = start_tag(body, "ir-s1.title");
    assert!(
        title.contains("--sd-italic:normal;") && title.contains("--sd-color:"),
        "{title}"
    );
    assert!(
        body.contains("font-style:var(--sd-italic,normal)"),
        "{body}"
    );
    assert!(!start_tag(body, "ir-s1.body").contains("--sd-fill"));
    let unhooked: Vec<String> = on
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == codes::HTML6011)
        .map(|diagnostic| diagnostic.node.as_ref().unwrap().to_string())
        .collect();
    assert_eq!(unhooked, ["ir-s1.body"]);
    let slide = deck_slide(&presentation);
    let timeline = slide["timeline"].as_array().unwrap();
    assert_eq!(
        timeline[0]["params"],
        serde_json::json!([
            { "property": "set", "beginMs": 0, "durationMs": 0, "to": { "property": "italic", "value": true } },
            { "property": "set", "beginMs": 250, "durationMs": 0, "to": { "property": "text", "value": "#44546a" } },
        ])
    );
    assert!(timeline[1].get("effect").is_none(), "{}", timeline[1]);
}

/// A colour frame a fold left part-way between the element's own colour and a colour (`ColorValue::Mix`) is computed
/// from the paint, like a shift: the deck holds only `#rrggbb`.
#[test]
fn a_colour_mix_resolves_from_the_paint() {
    let mut presentation = painted();
    let toward = HexColor::rgb(0x33, 0x33, 0x99);
    let frame = |offset: f64, value: ColorValue| Frame {
        offset: unit(offset),
        value,
    };
    presentation.slides[0].timeline = vec![
        StepBuilder::emphasis(
            id("ir-s1.title"),
            BuildEffect::Behaviors(vec![Keyframes::Text(Track {
                begin_ms: Ms(0),
                duration_ms: Ms(500),
                easing: None,
                frames: vec![
                    frame(0.0, ColorValue::Shift(HslShift::NONE)),
                    frame(
                        0.5,
                        ColorValue::Mix(ColorMix {
                            shift: HslShift::NONE,
                            toward,
                            by: unit(0.5),
                        }),
                    ),
                    frame(1.0, ColorValue::Hex(toward)),
                ],
            })]),
        )
        .native_id(pptx("emph.19"))
        .build(),
    ];
    let slide = deck_slide(&presentation);
    // The title's text is tx2 (#44546a); halfway to #333399 is #3c4482.
    assert_eq!(
        slide["timeline"][0]["params"][0]["frames"],
        serde_json::json!([
            { "offset": 0.0, "value": "#44546a" },
            { "offset": 0.5, "value": "#3c4482" },
            { "offset": 1.0, "value": "#333399" },
        ])
    );
}
