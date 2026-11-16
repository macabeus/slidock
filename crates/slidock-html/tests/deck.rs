//! The deck JSON (`slidock-deck@1`) of a printed presentation, on the printer tests' fixture: slides with their
//! numbering, titles, notes, build steps and transitions, the media and faces, the diagnostics summary, the content
//! id and the canonical bytes.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::Value;
use slidock_html::deck::{
    CodeCount, DECK_FORMAT, Deck, DeckAdvance, DeckBuildEffect, DeckDiagnostic, DeckFont,
    DeckFormat, DeckInput, DeckStep, DeckTransition, DeckTransitionEffect, DiagnosticSummary,
    FontFileInfo, FontSource, PlayerStamp, StepTarget, build_deck, deck_id, notes_paragraphs,
};
use slidock_html::{Documents, HtmlError, notes_text};
use slidock_ir::diagnostics::{
    Collect, Diagnostic, DiagnosticSink, DiagnosticSite, Severity, Stage, codes,
};
use slidock_ir::effects::StepTiming;
use slidock_ir::fonts::{FaceRequest, FaceStyle, FontFile};
use slidock_ir::media::PlanAction;
use slidock_ir::presentation::Failed;
use slidock_ir::resolved::{Body, Element, Presentation, Shape};
use slidock_ir::text::{
    Anchor, Inline, Paragraph, PartialParaProps, PartialRunProps, ResolvedAutofit,
    ResolvedRunProps, TextBody,
};
use slidock_ir::timeline::{Delivery, StepFallback, StepKind, TransitionFallback, Trigger};
use support::{
    elements_mut, emit, family, frame, full, group, id, para_props, paragraph, provenance, rect,
    run, run_props, sha, shape, text_body, tx1,
};

fn player() -> PlayerStamp {
    PlayerStamp {
        version: "0.1.0".to_owned(),
        sha256: "ab".repeat(32),
        presenter: Some("cd".repeat(32)),
    }
}

fn deck_with(
    presentation: &Presentation,
    documents: &Documents,
    diagnostics: &[Diagnostic],
    fonts: &[FontFileInfo],
) -> Deck {
    build_deck(&DeckInput {
        presentation,
        documents,
        diagnostics,
        source: &provenance(),
        fonts,
    })
    .unwrap()
}

fn deck_of(presentation: &Presentation) -> Deck {
    let emitted = emit(presentation);
    deck_with(presentation, &emitted.files, &emitted.diagnostics, &[])
}

#[test]
fn lists_the_slides_in_show_order_numbering_only_the_shown_ones() {
    let mut presentation = full();
    let mut third = presentation.slides[0].clone();
    third.id = id("ir-s3");
    third.notes = None;
    third.transition = None;
    third.timeline.clear();
    presentation.slides.push(third);
    let deck = deck_of(&presentation);
    assert_eq!(deck.format, DeckFormat::V1);
    assert_eq!(deck.title, "full");
    assert_eq!((deck.size.w, deck.size.h), (720.0, 405.0));
    let slides: Vec<(&str, bool, Option<u32>, Option<&str>)> = deck
        .slides
        .iter()
        .map(|s| (s.id.as_str(), s.hidden, s.number, s.layout.as_deref()))
        .collect();
    assert_eq!(
        slides,
        [
            ("ir-s1", false, Some(1), Some("ir-layout")),
            ("ir-s2", true, None, Some("ir-layout")),
            ("ir-s3", false, Some(2), Some("ir-layout")),
        ]
    );
    assert_eq!(deck.slides[1].title, "Back to the start");
    assert_eq!(deck.slides[0].notes, ["Speaker notes"]);
    assert!(deck.slides[1].notes.is_empty());
    assert_eq!(deck.slides[0].failed, None);
}

#[test]
fn carries_build_steps_and_transitions_as_the_ir_holds_them() {
    let deck = deck_of(&full());
    assert_eq!(
        deck.slides[0].timeline,
        [DeckStep {
            target: StepTarget {
                element: "ir-s1.body".to_owned(),
                paragraphs: Some((0, 1)),
                background: false,
            },
            trigger: Trigger::Click,
            kind: StepKind::Entrance,
            delivery: Delivery::ByParagraph,
            delay_ms: 0,
            duration_ms: 500,
            native: "keynote:dissolve".to_owned(),
            fallback: StepFallback::Dissolve,
            effect: DeckBuildEffect::None {},
            timing: StepTiming::default(),
        }]
    );
    assert_eq!(
        deck.slides[0].transition,
        Some(DeckTransition {
            native: "pptx:fade".to_owned(),
            fallback: TransitionFallback::Dissolve,
            duration_ms: 500,
            advance: Some(DeckAdvance {
                on_click: true,
                after_ms: None,
            }),
            effect: DeckTransitionEffect::None {},
            easing: None,
        })
    );
    assert_eq!(deck.slides[1].transition, None);
    let json = deck.to_json().unwrap();
    assert!(json.contains(r#""timeline":[{"delayMs":0,"delivery":"byParagraph","durationMs":500,"fallback":"dissolve","kind":"entrance","native":"keynote:dissolve","target":{"element":"ir-s1.body","paragraphs":[0,1]},"trigger":"click"}]"#), "{json}");
}

#[test]
fn keeps_a_failed_slides_code_and_the_title_it_kept() {
    let mut presentation = full();
    presentation.slides[1].body = Body::Failed {
        failed: Failed::new(codes::IR0001, None),
        background: None,
    };
    presentation.slides[1].failed_a11y = Some(slidock_ir::resolved::FailedA11y {
        title: Some("Kept".to_owned()),
        reading_order: None,
    });
    let deck = deck_of(&presentation);
    assert_eq!(deck.slides[1].failed.as_deref(), Some("IR0001"));
    assert_eq!(deck.slides[1].title, "Kept");
}

#[test]
fn lists_the_media_files_the_documents_reference_with_a_copys_size() {
    let mut presentation = full();
    let emitted = emit(&presentation);
    let deck = deck_with(&presentation, &emitted.files, &[], &[]);
    let paths: Vec<&str> = deck.media.iter().map(|m| m.path.as_str()).collect();
    let expected: Vec<&str> = emitted.files.media.keys().map(String::as_str).collect();
    assert_eq!(paths, expected);
    assert!(!paths.is_empty());
    let poster = sha("poster.png").to_hex();
    let entry = deck.media.iter().find(|m| m.sha256 == poster).unwrap();
    assert_eq!(entry.path, format!("media/{poster}.png"));
    assert_eq!(
        (entry.mime.as_str(), entry.bytes),
        ("image/png", Some(4096))
    );
    // A transcode's size is not known until it is written.
    presentation
        .media
        .get_mut(&sha("poster.png"))
        .unwrap()
        .plan
        .action = PlanAction::Transcode;
    let deck = deck_with(&presentation, &emitted.files, &[], &[]);
    let entry = deck.media.iter().find(|m| m.sha256 == poster).unwrap();
    assert_eq!(entry.bytes, None);
}

#[test]
fn lists_each_requested_face_once_with_the_source_of_its_file() {
    let mut presentation = full();
    let face = |weight, style| FaceRequest { weight, style };
    let sans = presentation.fonts.get_mut("liberationsans").unwrap();
    sans.request.faces = vec![
        face(700, FaceStyle::Normal),
        face(400, FaceStyle::Italic),
        face(400, FaceStyle::Normal),
    ];
    sans.binding.file = Some(FontFile {
        sha256: sha("sans.ttf"),
        postscript_name: "LiberationSans".to_owned(),
    });
    let mono = presentation.fonts.get_mut("liberationmono").unwrap();
    mono.request.faces = vec![face(400, FaceStyle::Normal), face(700, FaceStyle::Normal)];
    // A family name that carries a weight draws every face at it, and bold at 700.
    mono.binding.weight = Some(500);
    let emitted = emit(&presentation);
    let files = [FontFileInfo {
        sha256: sha("sans.ttf"),
        source: FontSource::Google,
        licence: Some("OFL".to_owned()),
    }];
    let deck = deck_with(&presentation, &emitted.files, &[], &files);
    let font = |family: &str, weight, style, source, licence: Option<&str>| DeckFont {
        family: family.to_owned(),
        weight,
        style,
        source,
        licence: licence.map(str::to_owned),
    };
    assert_eq!(
        deck.fonts,
        [
            font(
                "Liberation Mono",
                500,
                FaceStyle::Normal,
                FontSource::System,
                None
            ),
            font(
                "Liberation Mono",
                700,
                FaceStyle::Normal,
                FontSource::System,
                None
            ),
            font(
                "Liberation Sans",
                400,
                FaceStyle::Normal,
                FontSource::Google,
                Some("OFL")
            ),
            font(
                "Liberation Sans",
                400,
                FaceStyle::Italic,
                FontSource::Google,
                Some("OFL")
            ),
            font(
                "Liberation Sans",
                700,
                FaceStyle::Normal,
                FontSource::Google,
                Some("OFL")
            ),
        ]
    );
    // Without the host's word on the file, nothing is known to ship.
    let deck = deck_with(&presentation, &emitted.files, &[], &[]);
    assert!(
        deck.fonts
            .iter()
            .all(|f| f.source == FontSource::System && f.licence.is_none())
    );
}

fn report(collect: &mut Collect, code: slidock_ir::diagnostics::DiagnosticCode, container: &str) {
    collect.report(code, DiagnosticSite::new().container(id(container)));
}

#[test]
fn counts_the_diagnostics_by_severity_and_code() {
    let mut collect = Collect::new(Stage::Pass, "test");
    report(&mut collect, codes::MID5401, "ir-s1");
    report(&mut collect, codes::IR0001, "ir-s2");
    report(&mut collect, codes::MID5401, "ir-s2");
    let summary = DiagnosticSummary::of(&collect.diagnostics);
    assert_eq!(
        (
            summary.total,
            summary.error,
            summary.lossy,
            summary.decline,
            summary.warning,
            summary.info
        ),
        (3, 1, 2, 0, 0, 0)
    );
    assert_eq!(
        summary.codes,
        [
            CodeCount {
                code: "IR0001".to_owned(),
                severity: Severity::Error,
                count: 1
            },
            CodeCount {
                code: "MID5401".to_owned(),
                severity: Severity::Lossy,
                count: 2
            },
        ]
    );
    let shown = DeckDiagnostic::from(&collect.diagnostics[1]);
    assert_eq!(
        (
            shown.code.as_str(),
            shown.severity,
            shown.container.as_deref(),
            shown.node
        ),
        ("IR0001", Severity::Error, Some("ir-s2"), None)
    );
    let presentation = full();
    let emitted = emit(&presentation);
    let deck = deck_with(&presentation, &emitted.files, &collect.diagnostics, &[]);
    assert_eq!(deck.diagnostics, summary);
}

fn notes(paragraphs: &[&[&str]]) -> TextBody {
    let inline = |text: &str| match text {
        "\n" => Inline::Break { props: None },
        "\t" => Inline::Tab {},
        text => Inline::Run {
            text: text.to_owned(),
            props: PartialRunProps::default(),
            line_height: None,
            leading_skew: None,
        },
    };
    TextBody {
        body: Default::default(),
        list_style: None,
        paragraphs: paragraphs
            .iter()
            .map(|inlines| Paragraph {
                level: 0,
                props: PartialParaProps::default(),
                inlines: inlines.iter().map(|text| inline(text)).collect(),
                end_props: None,
            })
            .collect(),
        flips_with_frame: false,
        rotation: None,
    }
}

#[test]
fn splits_notes_where_their_paragraphs_end_trimming_the_tail_as_the_manifest_does() {
    let cases: [&[&[&str]]; 5] = [
        &[&["One"], &["Two", "\n", "lines"], &["a", "\t", "b"]],
        &[&["Kept "], &[" "], &[]],
        &[&[], &["After a blank"]],
        &[&[" "], &["\t"]],
        &[],
    ];
    let expected: [&[&str]; 5] = [
        &["One", "Two\nlines", "a\tb"],
        &["Kept"],
        &["", "After a blank"],
        &[],
        &[],
    ];
    for (case, expected) in cases.iter().zip(expected) {
        let body = notes(case);
        let paragraphs = notes_paragraphs(Some(&body));
        assert_eq!(paragraphs, expected, "{case:?}");
        assert_eq!(paragraphs.join("\n"), notes_text(Some(&body)), "{case:?}");
    }
    assert!(notes_paragraphs(None).is_empty());
}

#[test]
fn ids_the_content_whatever_the_player() {
    let presentation = full();
    let emitted = emit(&presentation);
    let deck = deck_of(&presentation);
    assert_eq!(deck.id.len(), 64);
    assert_eq!(deck.id, deck_id(&deck, &emitted.files).unwrap());
    // The builder leaves the stamp to the bundle, which records the player it embeds.
    assert_eq!(deck.player, None);
    assert!(deck.to_json().unwrap().contains(r#""player":null"#));
    let stamped = Deck {
        player: Some(player()),
        ..deck.clone()
    };
    assert_eq!(deck_id(&stamped, &emitted.files).unwrap(), deck.id);
    assert_ne!(stamped.to_json().unwrap(), deck.to_json().unwrap());
    // A document's bytes are content the JSON does not hold.
    let mut documents = emitted.files.clone();
    documents
        .text
        .get_mut("slides/ir-s1.html")
        .unwrap()
        .push(' ');
    let changed = deck_with(&presentation, &documents, &emitted.diagnostics, &[]);
    assert_ne!(changed.id, deck.id);
    // And so is every field of the JSON.
    let mut retitled = presentation.clone();
    retitled.slides[0].notes = None;
    assert_ne!(deck_of(&retitled).id, deck.id);
}

#[test]
fn prints_canonical_json_that_reads_back_and_refuses_another_format() {
    let deck = deck_of(&full());
    let json = deck.to_json().unwrap();
    assert!(json.starts_with(r#"{"diagnostics":"#), "{json}");
    assert!(json.contains(&format!(r#""format":"{DECK_FORMAT}""#)));
    assert_eq!(json, deck_of(&full()).to_json().unwrap());
    assert_eq!(Deck::from_json(&json).unwrap(), deck);
    let other = json.replace(DECK_FORMAT, "slidock-deck@2");
    assert!(matches!(Deck::from_json(&other), Err(HtmlError::Deck(_))));
}

/// Adds `field` to every object at `pointer` in a deck JSON value.
fn add_field(value: &mut Value, pointer: &str, field: &str, added: Value) {
    let target = value.pointer_mut(pointer).unwrap();
    let objects: Vec<&mut Value> = match target {
        Value::Array(items) => items.iter_mut().collect(),
        object => vec![object],
    };
    for object in objects {
        object
            .as_object_mut()
            .unwrap()
            .insert(field.to_owned(), added.clone());
    }
}

#[test]
fn reads_a_v1_deck_that_carries_fields_it_does_not_know() {
    let mut presentation = full();
    presentation.slides[0].transition.as_mut().unwrap().advance =
        Some(slidock_ir::timeline::Advance {
            on_click: false,
            after_ms: Some(slidock_ir::units::Ms(2000)),
        });
    let deck = deck_of(&presentation);
    assert!(!deck.media.is_empty() && !deck.slides[0].timeline.is_empty());
    let mut value: Value = serde_json::from_str(&deck.to_json().unwrap()).unwrap();
    value["player"] = serde_json::to_value(player()).unwrap();
    // Additive fields at every level a later v1 may grow.
    for (pointer, field) in [
        ("", "timeline"),
        ("/player", "build"),
        ("/size", "aspect"),
        ("/source", "modified"),
        ("/slides", "match"),
        ("/slides/0/timeline", "curve"),
        ("/slides/0/timeline/0/target", "range"),
        ("/slides/0/transition", "options"),
        ("/slides/0/transition/advance", "loop"),
        ("/media", "width"),
        ("/diagnostics", "dropped"),
    ] {
        add_field(&mut value, pointer, field, Value::from("ease-in"));
    }
    let fonts = serde_json::to_value(vec![DeckFont {
        family: "Roboto".to_owned(),
        weight: 400,
        style: FaceStyle::Normal,
        source: FontSource::Google,
        licence: None,
    }])
    .unwrap();
    value["fonts"] = fonts;
    add_field(&mut value, "/fonts", "unicodeRange", Value::from("U+0-FF"));
    value["diagnostics"]["codes"] =
        serde_json::json!([{"code": "MID5401", "severity": "lossy", "count": 1, "first": "ir-s1"}]);
    let read = Deck::from_json(&value.to_string()).unwrap();
    assert_eq!(read.slides, deck.slides);
    assert_eq!(read.media, deck.media);
    assert_eq!(read.player, Some(player()));
    assert_eq!(read.fonts[0].family, "Roboto");
    assert_eq!(read.diagnostics.codes[0].count, 1);
    // An effect a reader does not know reads as none: the step plays its fallback.
    let mut unknown_effect = value.clone();
    unknown_effect["slides"][0]["timeline"][0]["effect"] = Value::from("teleport");
    unknown_effect["slides"][0]["timeline"][0]["params"] = serde_json::json!({ "speed": 2 });
    let read = Deck::from_json(&unknown_effect.to_string()).unwrap();
    assert_eq!(read.slides[0].timeline[0].effect, DeckBuildEffect::None {});
    // A value a reader does not know is not additive.
    value["slides"][0]["timeline"][0]["trigger"] = Value::from("onHover");
    assert!(matches!(
        Deck::from_json(&value.to_string()),
        Err(HtmlError::Deck(_))
    ));
}

/// Gives each fixture family the faces a compile could request of it, `serif` named with a weight.
fn requested_faces(presentation: &mut Presentation) {
    let face = |weight, style| FaceRequest { weight, style };
    let normal = FaceStyle::Normal;
    let italic = FaceStyle::Italic;
    let fonts = &mut presentation.fonts;
    fonts.get_mut("liberationsans").unwrap().request.faces =
        vec![face(400, normal), face(400, italic), face(700, normal)];
    fonts.get_mut("liberationmono").unwrap().request.faces =
        vec![face(400, normal), face(400, italic), face(700, normal)];
    let serif = fonts.get_mut("liberationserif").unwrap();
    serif.request.faces = vec![face(400, normal), face(700, normal)];
    serif.binding.weight = Some(500);
}

fn text_shape(element: &str, props: ResolvedRunProps) -> Shape {
    Shape {
        text: Some(text_body(
            Anchor::Top,
            ResolvedAutofit::None {},
            vec![paragraph(para_props(1.2), vec![run("Text", props)])],
        )),
        ..shape(element, frame(10.0, 10.0, 100.0, 40.0), rect(100.0, 40.0))
    }
}

fn mono(weight: u32, italic: bool) -> ResolvedRunProps {
    ResolvedRunProps {
        weight,
        italic,
        ..run_props(family("Liberation Mono"), 12.0, tx1())
    }
}

fn sans_italic() -> ResolvedRunProps {
    ResolvedRunProps {
        italic: true,
        ..run_props(family("Liberation Sans"), 12.0, tx1())
    }
}

fn serif(weight: u32) -> ResolvedRunProps {
    ResolvedRunProps {
        weight,
        ..run_props(family("Liberation Serif"), 12.0, tx1())
    }
}

/// The faces of a slide, by family, weight and style.
fn slide_faces(deck: &Deck, slide: usize) -> Vec<(&str, u32, FaceStyle)> {
    deck.slides[slide]
        .fonts
        .iter()
        .map(|&index| {
            let face = &deck.fonts[index as usize];
            (face.family.as_str(), face.weight, face.style)
        })
        .collect()
}

#[test]
fn lists_the_faces_each_slide_draws_with_its_shown_layouts_but_not_its_placeholders() {
    let mut presentation = full();
    requested_faces(&mut presentation);
    let layout = presentation.layouts.get_mut("ir-layout").unwrap();
    let Body::Built(built) = &mut layout.body else {
        panic!("the fixture's layout is built")
    };
    // A layout's own text draws on the slides that show it; a placeholder's prompt never does.
    built.elements.push(Element::Shape(Box::new(text_shape(
        "ir-layout.footer",
        sans_italic(),
    ))));
    let Element::Shape(prompt) = &mut built.elements[0] else {
        panic!("the layout's title placeholder")
    };
    prompt.text = text_shape("ir-layout.title", mono(700, false)).text;
    // A group's children and a Foreign fallback's elements draw.
    let mut fallback = elements_mut(&mut presentation, 0)
        .iter()
        .find_map(|element| match element {
            Element::Foreign(foreign) => Some(foreign.clone()),
            _ => None,
        })
        .unwrap();
    fallback.id = id("ir-s2.chart");
    fallback.fallback.as_mut().unwrap().elements = Some(vec![Element::Shape(Box::new(
        text_shape("ir-s2.chart.label", serif(400)),
    ))]);
    // A paragraph mark draws only in an empty paragraph, a break draws nothing, and an empty run's span takes part
    // in its line.
    let mut boxed_text = text_shape("ir-s2.box.text", mono(400, true));
    let body = boxed_text.text.as_mut().unwrap();
    body.paragraphs[0].inlines.push(Inline::Break {
        props: Some(mono(700, false)),
    });
    body.paragraphs[0]
        .inlines
        .push(run("more", mono(400, true)));
    body.paragraphs[0].end_props = Some(mono(700, false));
    body.paragraphs.push(Paragraph {
        end_props: Some(serif(700)),
        ..paragraph(para_props(1.2), vec![run("", mono(400, false))])
    });
    let boxed = group(
        "ir-s2.box",
        frame(400.0, 300.0, 100.0, 40.0),
        vec![Element::Shape(Box::new(boxed_text))],
    );
    let second = elements_mut(&mut presentation, 1);
    second.push(Element::Group(Box::new(boxed)));
    second.push(Element::Foreign(fallback));

    let deck = deck_of(&presentation);
    let (normal, italic) = (FaceStyle::Normal, FaceStyle::Italic);
    assert_eq!(
        deck.fonts
            .iter()
            .map(|f| (f.family.as_str(), f.weight, f.style))
            .collect::<Vec<_>>(),
        [
            ("Liberation Mono", 400, normal),
            ("Liberation Mono", 400, italic),
            ("Liberation Mono", 700, normal),
            ("Liberation Sans", 400, normal),
            ("Liberation Sans", 400, italic),
            ("Liberation Sans", 700, normal),
            ("Liberation Serif", 500, normal),
            ("Liberation Serif", 700, normal),
        ]
    );
    // Slide one: the title in Serif (drawn at the binding's 500), the body's Sans runs, bold field, Mono label and
    // italic run, and the layout's Sans italic footer; not the layout's Mono 700 prompt.
    assert_eq!(
        slide_faces(&deck, 0),
        [
            ("Liberation Mono", 400, normal),
            ("Liberation Mono", 400, italic),
            ("Liberation Sans", 400, normal),
            ("Liberation Sans", 400, italic),
            ("Liberation Sans", 700, normal),
            ("Liberation Serif", 500, normal),
        ]
    );
    // Slide two hides its layout: its note and table in Sans, its group's Mono italic, empty run and empty
    // paragraph's Serif 700 mark, its fallback's Serif.
    assert_eq!(
        slide_faces(&deck, 1),
        [
            ("Liberation Mono", 400, normal),
            ("Liberation Mono", 400, italic),
            ("Liberation Sans", 400, normal),
            ("Liberation Sans", 700, normal),
            ("Liberation Serif", 500, normal),
            ("Liberation Serif", 700, normal),
        ]
    );
    assert!(deck.to_json().unwrap().contains(r#""fonts":[0,1,3,5,6,7]"#));
    // Faces the slides draw but no request lists are not the deck's to load.
    let mut unrequested = presentation.clone();
    for entry in unrequested.fonts.values_mut() {
        entry.request.faces.clear();
    }
    let deck = deck_of(&unrequested);
    assert!(deck.slides.iter().all(|slide| slide.fonts.is_empty()));
}

#[test]
fn escapes_every_less_than_sign_inside_the_script_element() {
    let mut presentation = full();
    if let Body::Built(built) = &mut presentation.slides[0].body {
        built.title = "a </script><!-- b".to_owned();
    }
    let deck = deck_of(&presentation);
    let script = deck.to_script_json().unwrap();
    assert!(!script.contains('<'), "{script}");
    assert!(script.contains(r"a \u003c/script>\u003c!-- b"));
    assert_eq!(Deck::from_json(&script).unwrap(), deck);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&script).unwrap(),
        serde_json::from_str::<serde_json::Value>(&deck.to_json().unwrap()).unwrap()
    );
}

#[test]
fn prints_native_effects_and_timing_flat_in_steps_and_transitions() {
    use slidock_ir::effects::{
        Dir, Dir4, Dir4Params, Distance, Easing, FlyInParams, NativeId, NativeStep,
        NativeTransition, TransitionEffect,
    };
    use slidock_ir::formats::FileFormat;
    use slidock_ir::units::Ms;

    let mut presentation = full();
    let slide = &mut presentation.slides[0];
    slide.timeline[0].effect.native = Some(Box::new(NativeStep {
        id: NativeId::new(FileFormat::Pptx.into(), "entr.2/4"),
        effect: slidock_ir::effects::BuildEffect::FlyIn(FlyInParams {
            dir: Dir::Up,
            distance: Distance::OffSlide,
        }),
    }));
    slide.timeline[0].target.background = true;
    slide.timeline[0].timing = Some(Box::new(StepTiming {
        offset_ms: Some(Ms(250)),
        easing: Some(Easing::Out),
        ..StepTiming::default()
    }));
    let transition = slide.transition.as_mut().unwrap();
    transition.effect.native = Some(Box::new(NativeTransition {
        id: NativeId::new(FileFormat::Pptx.into(), "push"),
        effect: TransitionEffect::Push(Dir4Params { dir: Dir4::Left }),
    }));
    transition.easing = Some(Easing::InOut);
    let deck = deck_of(&presentation);
    let json = serde_json::to_value(&deck.slides[0]).unwrap();
    assert_eq!(
        json["timeline"][0],
        serde_json::json!({
            "target": { "element": "ir-s1.body", "paragraphs": [0, 1], "background": true },
            "trigger": "click", "kind": "entrance", "delivery": "byParagraph", "delayMs": 0, "durationMs": 500,
            "native": "pptx:entr.2/4", "fallback": "dissolve",
            "effect": "flyIn", "params": { "dir": "u", "distance": "offSlide" },
            "offsetMs": 250, "easing": "out"
        })
    );
    assert_eq!(
        json["transition"],
        serde_json::json!({
            "native": "pptx:push", "fallback": "dissolve", "durationMs": 500,
            "advance": { "onClick": true, "afterMs": null },
            "effect": "push", "params": { "dir": "l" }, "easing": "in-out"
        })
    );
    // The deck reads its own JSON back.
    let read = Deck::from_json(&serde_json::to_string(&deck).unwrap()).unwrap();
    assert_eq!(read.slides, deck.slides);
    // Without native data the step and the transition print as they always did.
    let plain = serde_json::to_value(&deck_of(&full()).slides[0]).unwrap();
    assert!(
        plain["timeline"][0].get("effect").is_none()
            && plain["timeline"][0].get("offsetMs").is_none()
    );
    assert!(plain["timeline"][0]["target"].get("background").is_none());
    assert!(
        plain["transition"].get("params").is_none() && plain["transition"].get("easing").is_none()
    );
}
