//! Hostile packages: whatever the bytes, the frontend refuses the input or lowers it with diagnostics, within seconds,
//! and never panics; a read graph's every decoded object dumps, and its style chains end. A container that fails as
//! a crash (KN1011) counts as a failure.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::disallowed_types,
    reason = "test code; the time limit is the test's own"
)]

mod support;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

use serde_json::json;
use slidock_core_api::{Frontend, FrontendError, Input, Limits, LowerOptions};
use slidock_io::BytesReader;
use slidock_ir::diagnostics::{Collect, DiagnosticCode, Stage, codes};
use slidock_keynote::graph::MAX_INDEX_BYTES;
use slidock_keynote::iwa::{dump, unframe_iwa};
use slidock_keynote::proto::{kn, tsd, tsp, tswp};
use slidock_keynote::styles::{CharacterStyle, ParagraphStyle, ShapeStyle, style_chain};
use slidock_keynote::{DOCUMENT_PATH, KeynoteFrontend, open_document, report_index};
use support::fixtures::{basic_presentation, shapes_presentation, text_presentation};
use support::harness::{lower, lower_native, with_ctx};
use support::keynote::{Presentation, geometry as box_at};
use support::{
    PresentationBuilder, Seeded, concat, frame_iwa, fuzz_iterations, mutate, r, style, zip,
};

/// How long one input may take.
const PER_INPUT: Duration = Duration::from_secs(5);

fn geometry(x: f32, y: f32, w: f32, h: f32) -> tsd::GeometryArchive {
    tsd::GeometryArchive {
        position: Some(tsp::Point {
            x: Some(x),
            y: Some(y),
        }),
        size: Some(tsp::Size {
            width: Some(w),
            height: Some(h),
        }),
        ..Default::default()
    }
}

/// A text box on a slide under a template, with a style chain and a nested slide.
fn text_deck() -> Vec<(String, Vec<u8>)> {
    let mut presentation = PresentationBuilder::new();
    let body = presentation.paragraph_style(
        tswp::CharacterStylePropertiesArchive {
            font_size: Some(40.0),
            font_name: Some("HelveticaNeue".into()),
            ..Default::default()
        },
        None,
    );
    let title = presentation.paragraph_style(
        tswp::CharacterStylePropertiesArchive {
            bold: Some(true),
            ..Default::default()
        },
        Some(body),
    );
    let storage = presentation.storage("Hello, wörld 😀\nsecond line", Some(title));
    let text_box = tswp::ShapeInfoArchive {
        super_: Some(tsd::ShapeArchive {
            super_: Some(tsd::DrawableArchive {
                geometry: Some(geometry(10.0, 20.0, 300.0, 100.0)),
                ..Default::default()
            }),
            ..Default::default()
        }),
        owned_storage: Some(storage),
        is_text_box: Some(true),
        ..Default::default()
    };
    let text_box = presentation.add_in("TSWP.ShapeInfoArchive", &text_box, "Index/Slide.iwa");
    let template = presentation.template("Title", vec![]);
    let first = presentation.slide(
        kn::SlideArchive {
            drawables_z_order: vec![text_box],
            template_slide: Some(template),
            ..Default::default()
        },
        Default::default(),
        None,
    );
    presentation.slide(Default::default(), Default::default(), Some(&first));
    presentation.files()
}

/// Shapes, a group, an image with its data file, and a hidden slide.
fn shapes_deck() -> Vec<(String, Vec<u8>)> {
    let mut presentation = PresentationBuilder::new();
    let fill = tsd::ShapeStyleArchive {
        super_: Some(style(None)),
        shape_properties: Some(tsd::ShapeStylePropertiesArchive {
            opacity: Some(0.5),
            ..Default::default()
        }),
        ..Default::default()
    };
    let fill = presentation.add("TSD.ShapeStyleArchive", &fill);
    let shape = tsd::ShapeArchive {
        super_: Some(tsd::DrawableArchive {
            geometry: Some(geometry(0.0, 0.0, 50.0, 50.0)),
            ..Default::default()
        }),
        style: Some(fill),
        ..Default::default()
    };
    let shape = presentation.add_in("TSD.ShapeArchive", &shape, "Index/Slide.iwa");
    let group = tsd::GroupArchive {
        super_: Some(tsd::DrawableArchive {
            geometry: Some(geometry(5.0, 5.0, 60.0, 60.0)),
            ..Default::default()
        }),
        children: vec![shape],
        ..Default::default()
    };
    let group = presentation.add_in("TSD.GroupArchive", &group, "Index/Slide.iwa");
    let data = presentation.data("image.png", b"\x89PNG\r\n\x1a\n0000".to_vec());
    let image = tsd::ImageArchive {
        super_: Some(tsd::DrawableArchive {
            geometry: Some(geometry(100.0, 100.0, 200.0, 150.0)),
            ..Default::default()
        }),
        data: Some(data),
        ..Default::default()
    };
    let image = presentation.add_in("TSD.ImageArchive", &image, "Index/Slide.iwa");
    presentation.slide(
        kn::SlideArchive {
            drawables_z_order: vec![group, image],
            ..Default::default()
        },
        kn::SlideNodeArchive {
            is_skipped: Some(true),
            ..Default::default()
        },
        None,
    );
    presentation.files()
}

/// Motion the deck path reads: transitions on shown, skipped and last slides (a Magic Move among them), builds with
/// directions, text deliveries, orders, triggers and curves.
fn motion_deck() -> Vec<u8> {
    let mut p = Presentation::new();
    let transition = |effect: &str, extra: serde_json::Value| {
        json!({ "transition": { "attributes": {
            "animationAttributes": support::keynote::merge(
                json!({ "animation_type": "Transition", "effect": effect, "duration": 1.0, "direction": 23 }),
                extra,
            ),
            "custom_timing_curve": 5,
        } } })
    };
    let text = p.storage("one two three", None, json!({}));
    let text = p.text_box(text, box_at(0.0, 0.0, 300.0, 100.0), None);
    let build = p.add(
        "KN.BuildArchive",
        json!({ "drawable": text, "delivery": "By Paragraph", "attributes": {
            "animationAttributes": { "animation_type": "In", "effect": "apple:dissolve character", "duration": 1.0,
                                     "direction": 41, "random_number_seed": 7 },
            "custom_textDelivery": 1, "custom_deliveryOption": 5, "eventTrigger": 2, "action_acceleration": 3 } }),
    );
    let chunk = p.add(
        "KN.BuildChunkArchive",
        json!({ "build": build, "duration": 1.0, "referent": true, "delay": 0.5 }),
    );
    let slide = |p: &mut Presentation, extra: serde_json::Value, skipped: bool| {
        p.slide(extra, json!({ "isSkipped": skipped }), None)
    };
    let mut first = transition(
        "apple:magic-move-implied-motion-path",
        json!({ "is_automatic": true }),
    );
    first["drawables_z_order"] = json!([text]);
    first["builds"] = json!([build]);
    first["buildChunks"] = json!([chunk]);
    slide(&mut p, first, false);
    slide(&mut p, transition("apple:slide", json!({})), true);
    slide(
        &mut p,
        transition("apple:wipe", json!({ "delay": 2.0 })),
        false,
    );
    slide(&mut p, transition("apple:reveal", json!({})), false);
    p.bytes()
}

fn bases() -> Vec<Vec<u8>> {
    vec![
        zip(&text_deck(), 0),
        zip(&shapes_deck(), 0),
        zip(&text_deck(), 6),
        basic_presentation(),
        shapes_presentation(),
        text_presentation(),
        motion_deck(),
    ]
}

/// Analyzes and lowers the input on the parity path and on the deck path (`native_motion`); the failure when it
/// neither refuses nor lowers, or a container crashed (KN1011).
fn lowers(bytes: &[u8]) -> Result<(), String> {
    lowers_as(bytes, false)?;
    lowers_as(bytes, true)
}

fn lowers_as(bytes: &[u8], native_motion: bool) -> Result<(), String> {
    let reader = BytesReader::new(bytes);
    let input = Input::new("presentation.key", &reader);
    let (outcome, diagnostics) = with_ctx(&Limits::default(), |ctx| {
        let analysis = KeynoteFrontend.analyze(&input, ctx)?;
        let facts = analysis.facts();
        KeynoteFrontend.lower(
            &input,
            ctx,
            &LowerOptions {
                facts: &facts,
                only: None,
                native_motion,
            },
        )?;
        Ok::<(), FrontendError>(())
    });
    match outcome {
        Ok(()) | Err(FrontendError::Refused(_)) => {}
        Err(other) => return Err(other.to_string()),
    }
    if let Some(crashed) = diagnostics
        .iter()
        .find(|d| d.code == codes::KN1011 || d.code == codes::IR0906)
    {
        return Err(format!(
            "{} on {:?}: {}",
            crashed.code.code(),
            crashed.container,
            crashed.message
        ));
    }
    Ok(())
}

/// Opens the input, refused or read; a read graph is walked whole. `None` when it survives.
fn survive(bytes: &[u8]) -> Option<String> {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let reader = BytesReader::new(bytes);
        let input = Input::new("presentation.key", &reader);
        match open_document(&input, &Limits::default()) {
            Ok(document) => {
                let mut diagnostics = Collect::new(Stage::Frontend, "keynote");
                report_index(&document.graph, &mut diagnostics);
                for archive in document.graph.archives() {
                    dump(archive).map_err(|e| format!("{}: {e}", archive.type_name()))?;
                }
                for object in document.graph.objects() {
                    let reference = r(object.id.0);
                    style_chain::<CharacterStyle>(&document.graph, Some(&reference));
                    style_chain::<ParagraphStyle>(&document.graph, Some(&reference));
                    style_chain::<ShapeStyle>(&document.graph, Some(&reference));
                }
                Ok(())
            }
            Err(FrontendError::Refused(_)) => Ok(()),
            Err(other) => Err(other.to_string()),
        }?;
        let started = Instant::now();
        lowers(bytes)?;
        let elapsed = started.elapsed();
        if elapsed > PER_INPUT {
            return Err(format!("took {} ms", elapsed.as_millis()));
        }
        Ok(())
    }));
    match outcome {
        Ok(Ok(())) => None,
        Ok(Err(failure)) => Some(failure),
        Err(panic) => Some(
            panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "a panic".into()),
        ),
    }
}

fn refusal(bytes: &[u8]) -> Option<DiagnosticCode> {
    let reader = BytesReader::new(bytes);
    let input = Input::new("presentation.key", &reader);
    match open_document(&input, &Limits::default()) {
        Ok(_) => None,
        Err(FrontendError::Refused(diagnostic)) => Some(diagnostic.code),
        Err(other) => panic!("{other}"),
    }
}

#[test]
fn the_bases_open() {
    for base in bases() {
        assert_eq!(survive(&base), None);
        assert_eq!(refusal(&base), None);
    }
    // The motion base reaches the deck path's readings: placement, delivery, the last slide's transition.
    let motion = lower_native(&motion_deck());
    let slides = motion.json["slides"].as_array().unwrap();
    assert_eq!(
        slides[2]["transition"]["effect"]["native"]["effect"],
        json!("magicMove")
    );
    assert_eq!(
        slides[3]["transition"]["effect"]["native"]["effect"],
        json!("wipe")
    );
    assert_eq!(slides[0]["timeline"][0]["delivery"], json!("byParagraph"));
    assert!(motion.codes().contains(&"KN1409"));
}

#[test]
fn survives_structure_blind_mutations_of_every_base() {
    let bases = bases();
    let mut random = Seeded::new(0x4b4e);
    let mut failures = Vec::new();
    let mut refused = 0;
    for i in 0..fuzz_iterations() {
        let mut bytes = bases[i % bases.len()].clone();
        let rounds = 1 + random.below(3);
        for _ in 0..rounds {
            bytes = mutate(&bytes, &mut random);
        }
        if let Some(failure) = survive(&bytes) {
            failures.push(format!("iteration {i}: {failure}"));
        }
        refused += usize::from(refusal(&bytes).is_some());
    }
    assert_eq!(failures, Vec::<String>::new());
    // The mutations reach past the zip: some inputs still open.
    assert!(refused < fuzz_iterations(), "{refused}");
}

#[test]
fn survives_mutations_inside_the_object_streams_of_each_index_file() {
    let base = text_deck();
    let index: Vec<usize> = (0..base.len())
        .filter(|&i| base[i].0.ends_with(".iwa"))
        .collect();
    let mut random = Seeded::new(0x1a7);
    let mut failures = Vec::new();
    for i in 0..fuzz_iterations() {
        let at = index[i % index.len()];
        let (path, framed) = &base[at];
        let stream = unframe_iwa(framed, path, MAX_INDEX_BYTES).unwrap();
        let mut files = base.clone();
        files[at].1 = frame_iwa(&mutate(&stream, &mut random));
        if let Some(failure) = survive(&zip(&files, 0)) {
            failures.push(format!("{path}: {failure}"));
        }
    }
    assert_eq!(failures, Vec::<String>::new());
}

#[test]
fn refuses_a_chunk_that_declares_more_than_the_index_budget_before_decompressing_it() {
    // A Snappy block declaring 2^32 - 1 bytes and holding none.
    let bomb = concat(&[&[0, 5, 0, 0], &[0xff, 0xff, 0xff, 0xff, 0x0f]]);
    assert_eq!(
        refusal(&zip(&[(DOCUMENT_PATH, bomb)], 0)),
        Some(codes::KN1004)
    );
}

#[test]
fn refuses_an_object_header_that_decodes_past_the_index_budget() {
    // One header of 1,500,000 empty MessageInfos: 3 MB of stream, 288 MB once decoded.
    let header = |field: [u8; 2]| {
        let mut info = vec![0x08, 1];
        for _ in 0..1_500_000 {
            info.extend(field);
        }
        let stream = concat(&[&support::varint(info.len() as u64), &info]);
        zip(&[(DOCUMENT_PATH, frame_iwa(&stream))], 0)
    };
    assert_eq!(refusal(&header([0x12, 0])), Some(codes::KN1004));
    // The same bytes as a field the schema leaves out: a header with no object, read whole.
    assert_eq!(refusal(&header([0x78, 0])), Some(codes::KN1003));
}

#[test]
fn refuses_a_presentation_that_is_not_a_package_of_a_current_keynote() {
    assert_eq!(
        refusal(&zip(&[("Index.zip", b"z")], 0)),
        Some(codes::KN1001)
    );
    assert_eq!(refusal(b"not a zip"), Some(codes::KN1003));
}

#[test]
fn survives_mutations_inside_the_object_streams_of_the_shapes_fixture() {
    let base = support::unzip_stored(&shapes_presentation());
    let index: Vec<usize> = (0..base.len())
        .filter(|&i| base[i].0.ends_with(".iwa"))
        .collect();
    let mut random = Seeded::new(0x1a7);
    let mut failures = Vec::new();
    for i in 0..fuzz_iterations() {
        let at = index[i % index.len()];
        let (path, framed) = &base[at];
        let stream = unframe_iwa(framed, path, MAX_INDEX_BYTES).unwrap();
        let mut files = base.clone();
        files[at].1 = frame_iwa(&mutate(&stream, &mut random));
        if let Some(failure) = survive(&zip(&files, 0)) {
            failures.push(format!("{path}: {failure}"));
        }
    }
    assert_eq!(failures, Vec::<String>::new());
}

#[test]
fn survives_cycles_in_the_slide_tree_groups_that_hold_themselves_and_styles_that_are_their_own_parent()
 {
    let mut p = Presentation::new();
    let style = p.add(
        "TSWP.ParagraphStyleArchive",
        json!({ "super": { "parent": { "identifier": "1000" } } }),
    );
    let group = p.add(
        "TSD.GroupArchive",
        json!({ "super": { "geometry": box_at(0.0, 0.0, 10.0, 10.0) }, "children": [{ "identifier": "1001" }] }),
    );
    let text = p.storage("x", Some(style), json!({}));
    let text_box = p.text_box(text, box_at(0.0, 0.0, 9.0, 9.0), None);
    let first = p.slide(
        json!({ "drawables_z_order": [group, text_box] }),
        json!({}),
        None,
    );
    // A slide indented under the first that lists the first among its own indented slides.
    p.slide(json!({}), json!({ "children": [first] }), Some(&first));
    let bytes = p.bytes();
    assert_eq!(survive(&bytes), None);
    assert_eq!(lower(&bytes).presentation.slides.len(), 2);
}
