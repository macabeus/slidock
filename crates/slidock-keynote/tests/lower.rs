//! Lowering a Keynote presentation into the IR: slide order, layouts and placeholder binding, builds and transitions,
//! media, isolation and the reads it costs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::{Value, json};
use slidock_core_api::{Frontend, Input, Limits, LowerOptions, SniffHints};
use slidock_io::{BytesReader, CountingReader};
use slidock_keynote::KeynoteFrontend;
use support::harness::{NAME, lower, lower_with, with_ctx};
use support::keynote::{Presentation, geometry, geometry_with, id_of, movie, r};
use support::{IwaRecord, Type, iwa_file, zip};

const PNG: [u8; 24] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52, 0, 0, 0,
    1, 0, 0, 0, 1,
];
const GIF: &[u8] = b"GIF89a\x01\x00\x01\x00\x00\x00\x00";
const PDF: &[u8] = b"%PDF-1.4\n";

fn png_with(tail: &[u8]) -> Vec<u8> {
    [&PNG[..], tail].concat()
}

fn kn(reference: &Value) -> String {
    format!("kn-{}", id_of(reference))
}

fn shape(p: &mut Presentation) -> Value {
    p.add(
        "TSD.ShapeArchive",
        json!({ "super": { "geometry": geometry(0.0, 0.0, 10.0, 10.0) } }),
    )
}

fn kinds(elements: &[Value]) -> Vec<&str> {
    elements
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect()
}

#[test]
fn orders_slides_by_the_slide_tree_depth_first_with_skipped_slides_hidden_keyed_by_their_nodes() {
    let mut p = Presentation::new();
    let first = p.slide(json!({}), json!({}), None);
    p.slide(json!({}), json!({ "isSkipped": true }), Some(&first));
    p.slide(json!({}), json!({}), Some(&first));
    let last = p.slide(json!({}), json!({ "isSkipped": true }), None);
    let lowered = lower(&p.bytes_sized(1024.0, 768.0));
    assert_eq!(lowered.json["size"], json!({ "w": 1024, "h": 768 }));
    // Keynote draws the regular face where a family lacks a bold one.
    assert_eq!(
        lowered.json["defaults"]["text"]["0"]["run"]["synthesizeBold"],
        false
    );
    // Text taller than its box runs below it.
    assert_eq!(lowered.json["defaults"]["body"]["overflow"], "below");
    let hidden: Vec<bool> = lowered.analysis.slides.iter().map(|s| s.hidden).collect();
    assert_eq!(hidden, [false, true, false, true]);
    assert_eq!(lowered.analysis.slides[0].id.as_str(), kn(&first));
    assert_eq!(lowered.analysis.slides[3].id.as_str(), kn(&last));
    assert_eq!(lowered.lowered.provenance.format.as_str(), "keynote");
    assert_eq!(lowered.lowered.provenance.name, NAME);
}

#[test]
fn lowers_templates_as_layouts_without_their_tagged_prototypes_and_binds_placeholders_they_hold() {
    let mut p = Presentation::new();
    let bar = shape(&mut p);
    let prototype = p.storage("Prototype", None, json!({}));
    let tagged = p.text_box(prototype, geometry(10.0, 10.0, 500.0, 100.0), None);
    let placeholder = |p: &mut Presentation, kind: i32, storage: Option<Value>, at: Value| {
        let mut info = json!({ "super": { "super": { "geometry": at } } });
        if let Some(storage) = storage {
            info["owned_storage"] = storage;
        }
        p.add(
            "KN.PlaceholderArchive",
            json!({ "super": info, "kind": kind }),
        )
    };
    let title = placeholder(&mut p, 2, None, geometry(0.0, 0.0, 10.0, 10.0));
    let body = placeholder(&mut p, 3, None, geometry(0.0, 0.0, 10.0, 10.0));
    let template = p.template(
        "Title",
        vec![bar.clone(), tagged.clone(), title.clone()],
        json!({ "sage_tag_to_info_map": [{ "tag": "Text", "info": tagged }], "titlePlaceholder": title,
                "bodyPlaceholder": body }),
    );
    let title_text = p.storage("Presentation title", None, json!({}));
    let slide_title = placeholder(
        &mut p,
        2,
        Some(title_text),
        geometry(100.0, 100.0, 800.0, 200.0),
    );
    let body_text = p.storage("Body", None, json!({}));
    let slide_body = placeholder(&mut p, 3, Some(body_text), geometry(0.0, 300.0, 10.0, 10.0));
    p.slide(
        json!({ "template_slide": template, "drawables_z_order": [slide_title, slide_body] }),
        json!({}),
        None,
    );
    let lowered = lower(&p.bytes());
    let layout = &lowered.json["layouts"][kn(&template)];
    assert_eq!(layout["name"], "Title");
    let elements: Vec<(String, Value)> = layout["elements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["id"].as_str().unwrap().to_owned(),
                e["placeholder"]["role"].clone(),
            )
        })
        .collect();
    assert_eq!(
        elements,
        [(kn(&bar), Value::Null), (kn(&title), json!("title"))]
    );
    let slide = &lowered.json["slides"][0];
    assert_eq!(slide["layout"], layout["id"]);
    // The body placeholder the template names is missing from the layout, so the slide's binds by role later.
    let placeholders: Vec<&Value> = slide["elements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| &e["placeholder"])
        .collect();
    assert_eq!(
        placeholders,
        [
            &json!({ "role": "title", "target": kn(&title) }),
            &json!({ "role": "body" })
        ]
    );
}

#[test]
fn reads_transitions_and_orders_build_chunks_into_steps_reporting_what_does_not_resolve() {
    let mut p = Presentation::new();
    let a = shape(&mut p);
    let text = p.storage("one\ntwo", None, json!({}));
    let b = p.text_box(text, geometry(0.0, 0.0, 100.0, 100.0), None);
    let build = |p: &mut Presentation,
                 drawable: &Value,
                 kind: &str,
                 effect: &str,
                 delivery: &str| {
        p.add(
            "KN.BuildArchive",
            json!({ "drawable": drawable, "delivery": delivery,
                    "attributes": { "animationAttributes": { "animation_type": kind, "effect": effect, "duration": 0.5 } } }),
        )
    };
    let appear = build(&mut p, &a, "In", "apple:appear", "All at Once");
    let by_paragraph = build(&mut p, &b, "In", "apple:dissolve", "By Paragraph");
    let sparkle = build(&mut p, &a, "Out", "com.example.sparkle", "All at Once");
    let orphan = build(&mut p, &r(424_242), "In", "apple:dissolve", "All at Once");
    let chunk = |p: &mut Presentation, build: &Value, automatic: bool| {
        p.add(
            "KN.BuildChunkArchive",
            json!({ "build": build, "automatic": automatic, "duration": 1 }),
        )
    };
    let chunks = vec![
        chunk(&mut p, &appear, true),
        chunk(&mut p, &by_paragraph, false),
        chunk(&mut p, &by_paragraph, false),
        chunk(&mut p, &sparkle, false),
        chunk(&mut p, &orphan, false),
    ];
    p.slide(
        json!({
            "drawables_z_order": [a, b], "builds": [appear, by_paragraph, sparkle, orphan], "buildChunks": chunks,
            "transition": { "attributes": { "animationAttributes": { "effect": "apple:cube", "duration": 1.25 } } },
        }),
        json!({}),
        None,
    );
    p.slide(
        json!({ "transition": { "attributes": { "animationAttributes": { "effect": "none", "duration": 1 } } } }),
        json!({}),
        None,
    );
    let lowered = lower(&p.bytes());
    let (first, second) = (&lowered.json["slides"][0], &lowered.json["slides"][1]);
    assert_eq!(
        first["transition"],
        json!({ "effect": { "id": "keynote:apple:cube", "fallback": "dissolve" }, "durationMs": 1250 })
    );
    assert!(second.get("transition").is_none());
    let steps: Vec<Value> = first["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            json!([
                s["target"],
                s["trigger"],
                s["kind"],
                s["effect"]["fallback"],
                s["delivery"]
            ])
        })
        .collect();
    assert_eq!(
        steps,
        [
            json!([{ "element": kn(&a) }, "afterPrevious", "entrance", "appear", "all"]),
            json!([{ "element": kn(&b), "paragraphs": [0, 0] }, "click", "entrance", "dissolve", "byParagraph"]),
            json!([{ "element": kn(&b), "paragraphs": [1, 1] }, "click", "entrance", "dissolve", "byParagraph"]),
            json!([{ "element": kn(&a) }, "click", "exit", "dissolve", "all"]),
        ]
    );
    assert_eq!(lowered.codes(), ["KN1401", "KN1402", "KN1403"]);
}

/// A vector image's `data` is a PDF and its thumbnail a small preview: its SVG original is drawn when it passes the
/// allow-list, the thumbnail when it does not, which is reported (KN1113, with the refusal).
#[test]
fn draws_a_vector_image_from_its_svg_original_when_the_svg_passes() {
    const DRAWN: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 190 108" width="1024px" height="582px"><path d="M 0 0 H 190 V 108 H 0 Z M 9 9 V 99 H 181 V 9 Z" fill="#FE7867" fill-rule="evenodd"/></svg>"##;
    const SCRIPTED: &[u8] =
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><script>alert(1)</script></svg>"#;
    let mut p = Presentation::new();
    let vector = p.data("frame.pdf", PDF.to_vec());
    let thumbnail = p.data("frame-small.png", png_with(&[1]));
    let drawn = p.data("frame.svg", DRAWN.to_vec());
    let scripted = p.data("scripted.svg", SCRIPTED.to_vec());
    let photo = p.data("photo.png", PNG.to_vec());
    let image = |p: &mut Presentation, data: Value| {
        let message = support::keynote::merge(
            json!({ "super": { "geometry": geometry(0.0, 0.0, 190.0, 108.0) } }),
            data,
        );
        p.add("TSD.ImageArchive", message)
    };
    let drawables = vec![
        image(
            &mut p,
            json!({ "data": vector, "originalSVGData": drawn, "thumbnailData": thumbnail }),
        ),
        image(
            &mut p,
            json!({ "data": vector, "originalSVGData": scripted, "thumbnailData": thumbnail }),
        ),
        // A raster's own data comes first.
        image(&mut p, json!({ "data": photo, "originalSVGData": drawn })),
    ];
    p.slide(json!({ "drawables_z_order": drawables }), json!({}), None);
    let lowered = lower(&p.bytes());
    let ir = &lowered.json;
    let parts: Vec<Value> = lowered
        .elements(0)
        .iter()
        .map(|e| ir["media"][e["media"].as_str().unwrap()]["origin"]["part"].clone())
        .collect();
    assert_eq!(
        parts,
        [
            json!("Data/frame.svg"),
            json!("Data/frame-small.png"),
            json!("Data/photo.png")
        ]
    );
    let svg = &ir["media"][lowered.elements(0)[0]["media"].as_str().unwrap()];
    assert_eq!(
        (&svg["type"], &svg["width"], &svg["height"]),
        (&json!("image/svg+xml"), &json!(1024), &json!(582))
    );
    // The refused SVG is never recorded.
    let recorded: Vec<&Value> = ir["media"]
        .as_object()
        .unwrap()
        .values()
        .map(|entry| &entry["origin"]["part"])
        .collect();
    assert!(
        !recorded.contains(&&json!("Data/scripted.svg")),
        "{recorded:?}"
    );
    assert_eq!(lowered.codes(), ["KN1113"]);
    let message = &lowered.diagnostics[0].message;
    assert!(
        message.contains(&format!(
            "(data {})",
            scripted["identifier"].as_str().unwrap()
        )) && message.ends_with("is not shown: the element script is not shown"),
        "{message}"
    );
}

/// Keynote keeps an inserted HEIC photo as it is, beside a JPEG preview (256 pixels wide): the photo is the picture's
/// media, which a host may decode, and the next image the output shows is its fallback (the preview, or the
/// original under an adjusted HEIC). A HEIC preview is no fallback.
#[test]
fn keeps_a_heic_photo_with_the_next_shown_image_as_its_fallback() {
    const HEIC: &[u8] = include_bytes!("../../../probes/heif/single.heic");
    let mut p = Presentation::new();
    let photo = p.data("photo.heic", HEIC.to_vec());
    let preview = p.data("photo-small.png", png_with(&[2]));
    let heic_preview = p.data("photo-small.heic", [HEIC, &[0]].concat());
    let original = p.data("original.png", png_with(&[3]));
    let image = |p: &mut Presentation, data: Value| {
        let message = support::keynote::merge(
            json!({ "super": { "geometry": geometry(0.0, 0.0, 64.0, 48.0) } }),
            data,
        );
        p.add("TSD.ImageArchive", message)
    };
    let drawables = vec![
        image(&mut p, json!({ "data": photo, "thumbnailData": preview })),
        image(
            &mut p,
            json!({ "data": photo, "thumbnailData": heic_preview }),
        ),
        image(
            &mut p,
            json!({ "adjustedImageData": photo, "data": original, "thumbnailData": preview }),
        ),
    ];
    p.slide(json!({ "drawables_z_order": drawables }), json!({}), None);
    let lowered = lower(&p.bytes());
    let ir = &lowered.json;
    let part = |sha: &Value| ir["media"][sha.as_str().unwrap()]["origin"]["part"].clone();
    let pictures: Vec<Value> = lowered
        .elements(0)
        .iter()
        .map(|e| json!([part(&e["media"]), e.get("fallback").map(part)]))
        .collect();
    assert_eq!(
        pictures,
        [
            json!(["Data/photo.heic", "Data/photo-small.png"]),
            json!(["Data/photo.heic", null]),
            json!(["Data/photo.heic", "Data/original.png"]),
        ]
    );
    // The photo is sized as it shows.
    let heic = &ir["media"][lowered.elements(0)[0]["media"].as_str().unwrap()];
    assert_eq!(
        (&heic["type"], &heic["width"], &heic["height"]),
        (&json!("image/heic"), &json!(64), &json!(48))
    );
    assert!(lowered.codes().is_empty(), "{:?}", lowered.codes());
}

/// Keynote cuts an image to its Instant Alpha outline (retro-fidelity rf 9's logo: its grey background and two lines
/// of its lettering are gone on Keynote's page). The outline is in the image's natural size, so it is the picture's
/// shape as a custom geometry of that size, beside its mask. An outline that is not read leaves the image whole, and is
/// reported (KN1114).
#[test]
fn cuts_an_image_to_its_instant_alpha_outline_or_reports_it_unread() {
    let mut p = Presentation::new();
    let photo = p.data("logo.png", PNG.to_vec());
    let mask = p.add(
        "TSD.MaskArchive",
        json!({ "super": { "geometry": geometry(5.0, 0.0, 70.0, 40.0) },
                "pathsource": { "scalar_path_source": { "type": 0, "scalar": 0 } } }),
    );
    let outline = json!({ "elements": [
        { "type": 1, "points": [{ "x": 20, "y": 10 }] },
        { "type": 2, "points": [{ "x": 180, "y": 10 }] },
        { "type": 4, "points": [{ "x": 190, "y": 50 }, { "x": 150, "y": 90 }, { "x": 100, "y": 90 }] },
        { "type": 2, "points": [{ "x": 20, "y": 90 }] },
        { "type": 5 },
        { "type": 1, "points": [{ "x": 20, "y": 10 }] },
    ] });
    let image = |p: &mut Presentation, data: Value| {
        let message = support::keynote::merge(
            json!({ "super": { "geometry": geometry(100.0, 100.0, 80.0, 40.0) }, "data": photo }),
            data,
        );
        p.add("TSD.ImageArchive", message)
    };
    let drawables = vec![
        image(
            &mut p,
            json!({ "instantAlphaPath": outline, "naturalSize": { "width": 200, "height": 100 }, "mask": mask }),
        ),
        image(&mut p, json!({ "instantAlphaPath": outline })),
        image(&mut p, json!({})),
    ];
    p.slide(json!({ "drawables_z_order": drawables }), json!({}), None);
    let lowered = lower(&p.bytes());
    let elements = lowered.elements(0);
    assert_eq!(kinds(elements), ["picture", "picture", "picture"]);
    assert_eq!(
        elements[0]["clip"],
        json!({ "kind": "custom", "paths": [{ "w": 200, "h": 100, "fill": true, "stroke": true, "fillRule": "nonzero",
                "segments": [["M", 20, 10], ["L", 180, 10], ["C", 190, 50, 150, 90, 100, 90], ["L", 20, 90], ["Z"]] }] })
    );
    assert_eq!(
        elements[0]["mask"],
        json!([{ "segments": [["M", 5, 0], ["L", 75, 0], ["L", 75, 40], ["L", 5, 40], ["Z"]],
                 "fill": true, "stroke": false, "fillRule": "nonzero" }])
    );
    assert!(elements[1].get("clip").is_none());
    assert!(elements[2].get("clip").is_none());
    assert_eq!(lowered.codes(), ["KN1114"]);
    let message = &lowered.diagnostics[0].message;
    assert!(
        message.ends_with("its Instant Alpha outline is not read: the image has no natural size"),
        "{message}"
    );
    let slide = lowered.presentation.slides[0].id.as_str();
    assert!(lowered.features(slide).contains(&"image.clip"));
}

#[test]
fn reads_pictures_masks_movies_groups_and_tables_and_what_it_leaves_out() {
    let mut p = Presentation::new();
    let photo = p.data("photo.png", PNG.to_vec());
    let adjusted = p.data("photo-adjusted.png", png_with(&[3]));
    let vector = p.data("vector.pdf", PDF.to_vec());
    let thumbnail = p.data("vector-small.png", png_with(&[1]));
    let gif = p.data("anim.gif", GIF.to_vec());
    let clip = p.data("clip.mp4", movie("vide", "avc1", true, None));
    let poster = p.data("poster.png", png_with(&[2]));
    let mask = p.add(
        "TSD.MaskArchive",
        json!({ "super": { "geometry": geometry(10.0, 0.0, 50.0, 40.0) },
                "pathsource": { "scalar_path_source": { "type": 0, "scalar": 0 } } }),
    );
    let image = |p: &mut Presentation, data: Value| {
        let message = support::keynote::merge(
            json!({ "super": { "geometry": geometry(100.0, 100.0, 80.0, 40.0) } }),
            data,
        );
        p.add("TSD.ImageArchive", message)
    };
    let film = |p: &mut Presentation, data: Value| {
        let message = support::keynote::merge(
            json!({ "super": { "geometry": geometry(0.0, 0.0, 10.0, 10.0) } }),
            data,
        );
        p.add("TSD.MovieArchive", message)
    };
    let child = shape(&mut p);
    let drawables = vec![
        image(&mut p, json!({ "data": photo, "mask": mask })),
        image(&mut p, json!({ "data": photo, "adjustedImageData": adjusted })),
        image(&mut p, json!({ "data": vector, "thumbnailData": thumbnail })),
        film(&mut p, json!({ "movieData": gif })),
        film(&mut p, json!({ "movieData": clip, "posterImageData": poster, "startTime": 2, "loop_option": 1, "volume": 0 })),
        film(&mut p, json!({ "movieData": { "identifier": "9999" }, "posterImageData": poster })),
        film(&mut p, json!({ "movieData": clip, "audioOnly": true })),
        image(&mut p, json!({ "data": { "identifier": "9998" } })),
        p.add(
            "TSD.GroupArchive",
            json!({ "super": { "geometry": geometry(200.0, 200.0, 100.0, 100.0) }, "children": [child] }),
        ),
        p.add("TST.TableInfoArchive", json!({ "super": { "geometry": geometry(0.0, 0.0, 300.0, 100.0) } })),
        p.add("TSCH.ChartDrawableArchive", json!({ "super": { "geometry": geometry(0.0, 0.0, 300.0, 100.0) } })),
        p.add_record(Type::Name("TSD.FreehandDrawingToolkitUIState"), Vec::new()),
        r(777_777),
    ];
    p.slide(json!({ "drawables_z_order": drawables }), json!({}), None);
    let lowered = lower(&p.bytes());
    let ir = &lowered.json;
    let elements = lowered.elements(0);
    assert_eq!(
        kinds(elements),
        [
            "picture", "picture", "picture", "picture", "media", "media", "group", "foreign",
            "foreign", "foreign"
        ]
    );
    let part_of = |sha: &Value| ir["media"][sha.as_str().unwrap()]["origin"].clone();
    let (masked, toned, rendered, animated, video, missing) = (
        &elements[0],
        &elements[1],
        &elements[2],
        &elements[3],
        &elements[4],
        &elements[5],
    );
    assert_eq!(
        masked["mask"],
        json!([{ "segments": [["M", 10, 0], ["L", 60, 0], ["L", 60, 40], ["L", 10, 40], ["Z"]],
                 "fill": true, "stroke": false, "fillRule": "nonzero" }])
    );
    // An adjusted image shows as Keynote saved it, with its adjustments applied; a vector shows its raster.
    assert_eq!(
        part_of(&toned["media"]),
        json!({ "kind": "source", "part": "Data/photo-adjusted.png" })
    );
    assert_eq!(
        part_of(&rendered["media"]),
        json!({ "kind": "source", "part": "Data/vector-small.png" })
    );
    assert_eq!(
        part_of(&animated["media"]),
        json!({ "kind": "source", "part": "Data/anim.gif" })
    );
    assert_eq!(video["mediaKind"], "video");
    assert_eq!(video["trim"], json!({ "startMs": 2000 }));
    assert_eq!(
        (&video["loop"], &video["muted"], &video["autoplay"]),
        (&json!(true), &json!(true), &json!(false))
    );
    assert_eq!(
        part_of(&video["poster"]),
        json!({ "kind": "source", "part": "Data/poster.png" })
    );
    assert!(missing.get("media").is_none());
    assert_eq!(missing["poster"], video["poster"]);
    let foreign: Vec<&Value> = elements[7..].iter().map(|e| &e["type"]).collect();
    assert_eq!(
        foreign,
        [&json!("table"), &json!("chart"), &json!("unknown")]
    );
    assert_eq!(
        ir["media"][masked["media"].as_str().unwrap()]["type"],
        "image/png"
    );
    assert_eq!(lowered.codes(), ["KN1008", "KN1105", "KN1107", "KN1110"]);
}

fn count(elements: &[Value]) -> usize {
    elements
        .iter()
        .map(|e| {
            1 + e
                .get("children")
                .and_then(Value::as_array)
                .map_or(0, |c| count(c))
        })
        .sum()
}

#[test]
fn places_a_drawable_once_however_often_it_is_referenced_and_fails_a_slide_past_the_group_depth() {
    let mut p = Presentation::new();
    let text = p.storage("x", None, json!({}));
    let text_box = p.text_box(text, geometry(0.0, 0.0, 10.0, 10.0), None);
    // Fifteen groups, each listing the one inside it twice: 65,535 drawables if every reference were followed.
    let mut child = text_box.clone();
    for _ in 0..15 {
        child = p.add(
            "TSD.GroupArchive",
            json!({ "super": { "geometry": geometry(0.0, 0.0, 10.0, 10.0) }, "children": [child, child] }),
        );
    }
    p.slide(
        json!({ "drawables_z_order": [child, text_box] }),
        json!({}),
        None,
    );
    let bytes = p.bytes();
    let lowered = lower(&bytes);
    assert_eq!(count(lowered.elements(0)), 16);
    // Each repeated reference is reported; repeats of one object merge into one diagnostic with a count.
    assert!(lowered.codes().iter().all(|code| *code == "KN1010"));
    let total: u32 = lowered
        .diagnostics
        .iter()
        .map(|d| d.count.unwrap_or(1))
        .sum();
    assert_eq!(total, 16);
    let deepest = Limits {
        group_depth: 10,
        ..Limits::default()
    };
    let limited = lower_with(&bytes, None, deepest);
    let slide = &limited.json["slides"][0];
    assert_eq!(slide["failed"], json!({ "code": "KN1901" }));
    assert_eq!(slide["elements"], json!([]));
}

#[test]
fn fails_only_the_slide_past_the_element_or_text_limits() {
    let mut p = Presentation::new();
    let text_box = |p: &mut Presentation, text: &str| {
        let storage = p.storage(text, None, json!({}));
        p.text_box(storage, geometry(0.0, 0.0, 10.0, 10.0), None)
    };
    let boxes = vec![
        text_box(&mut p, "a"),
        text_box(&mut p, "b"),
        text_box(&mut p, "c"),
    ];
    p.slide(json!({ "drawables_z_order": boxes }), json!({}), None);
    let boxes = vec![text_box(&mut p, "eleven char")];
    p.slide(json!({ "drawables_z_order": boxes }), json!({}), None);
    let boxes = vec![text_box(&mut p, "ten chars!"), text_box(&mut p, "d")];
    p.slide(json!({ "drawables_z_order": boxes }), json!({}), None);
    let limits = Limits {
        elements_per_container: 2,
        text_per_body: 10,
        ..Limits::default()
    };
    let lowered = lower_with(&p.bytes(), None, limits);
    let failed: Vec<&Value> = lowered.json["slides"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| &s["failed"])
        .collect();
    assert_eq!(
        failed,
        [
            &json!({ "code": "KN1901" }),
            &json!({ "code": "KN1901" }),
            &Value::Null
        ]
    );
}

#[test]
fn reports_an_object_a_list_repeats_once_however_often_it_repeats() {
    let mut p = Presentation::new();
    let text = p.storage("x", None, json!({}));
    let text_box = p.text_box(text, geometry(0.0, 0.0, 10.0, 10.0), None);
    p.slide(
        json!({ "drawables_z_order": vec![text_box; 1000] }),
        json!({}),
        None,
    );
    let lowered = lower(&p.bytes());
    assert_eq!(lowered.elements(0).len(), 1);
    let codes: Vec<(&str, u32)> = lowered
        .diagnostics
        .iter()
        .map(|d| (d.code.code(), d.count.unwrap_or(1)))
        .collect();
    assert_eq!(codes, [("KN1010", 1)]);
}

#[test]
fn keeps_a_shape_whose_outline_does_not_read_as_a_foreign_in_its_frame() {
    let mut p = Presentation::new();
    let odd = p.add(
        "TSD.ShapeArchive",
        json!({ "super": { "geometry": geometry(5.0, 6.0, 10.0, 10.0) },
                "pathsource": { "scalar_path_source": { "type": 1, "scalar": "Infinity" } } }),
    );
    p.slide(json!({ "drawables_z_order": [odd] }), json!({}), None);
    let lowered = lower(&p.bytes());
    let element = &lowered.elements(0)[0];
    assert_eq!(
        (&element["kind"], &element["type"]),
        (&json!("foreign"), &json!("unknown"))
    );
    assert_eq!(
        (&element["frame"]["x"], &element["frame"]["y"]),
        (&json!(5), &json!(6))
    );
    assert_eq!(element["payload"]["reason"], "outline");
}

#[test]
fn keeps_a_slide_whose_content_is_missing_in_place_failed_with_its_flags() {
    let record = |id: u64, name: &'static str, message: Value| {
        IwaRecord::raw(id, Type::Name(name), support::json::encode(name, &message))
    };
    let bytes = zip(
        &[(
            "Index/Document.iwa",
            iwa_file(&[
                record(
                    1,
                    "KN.DocumentArchive",
                    json!({ "show": { "identifier": "2" } }),
                ),
                record(
                    2,
                    "KN.ShowArchive",
                    json!({ "slideTree": { "slides": [{ "identifier": "3" }] }, "size": { "width": 800, "height": 600 } }),
                ),
                record(
                    3,
                    "KN.SlideNodeArchive",
                    json!({ "slide": { "identifier": "424242" }, "isSkipped": true }),
                ),
            ]),
        )],
        0,
    );
    let lowered = lower(&bytes);
    let slides = lowered.json["slides"].as_array().unwrap();
    assert_eq!(slides.len(), 1);
    assert_eq!(slides[0]["id"], "kn-3");
    assert_eq!(slides[0]["hidden"], true);
    assert_eq!(slides[0]["failed"], json!({ "code": "KN1009" }));
    assert_eq!(slides[0]["elements"], json!([]));
    assert_eq!(lowered.codes(), ["KN1009"]);
}

#[test]
fn sniffs_a_keynote_zip_at_its_root_or_one_folder_down() {
    let names = |names: &[&str]| names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
    let sniff = |zip_names: Option<&[String]>| {
        KeynoteFrontend.sniff(
            &[],
            &SniffHints {
                name: "x",
                zip_names,
            },
        )
    };
    assert!(sniff(Some(&names(&["Index/Document.iwa"]))));
    assert!(sniff(Some(&names(&[
        "presentation.key/Index/Document.iwa"
    ]))));
    assert!(!sniff(Some(&names(&["ppt/presentation.xml"]))));
    assert!(!sniff(None));
}

#[test]
fn never_reads_media_no_container_references_and_hashes_what_it_shows() {
    let mut p = Presentation::new();
    let unused = vec![0u8; 4 * 1024 * 1024];
    p.data("unused.mov", unused.clone());
    let shown = p.data("shown.png", [&PNG[..], &[0u8; 256 * 1024][..]].concat());
    let image = p.add(
        "TSD.ImageArchive",
        json!({ "super": { "geometry": geometry(0.0, 0.0, 10.0, 10.0) }, "data": shown }),
    );
    p.slide(json!({ "drawables_z_order": [image] }), json!({}), None);
    let bytes = p.bytes();
    let reader = CountingReader::new(BytesReader::new(bytes.as_slice()));
    let input = Input::new(NAME, &reader);
    let (before, _) = with_ctx(&Limits::default(), |ctx| {
        let analysis = KeynoteFrontend.analyze(&input, ctx).unwrap();
        assert_eq!(analysis.media_bytes, (PNG.len() + 256 * 1024) as u64);
        let before = reader.bytes_read();
        let facts = analysis.facts();
        let lowered = KeynoteFrontend
            .lower(
                &input,
                ctx,
                &LowerOptions {
                    facts: &facts,
                    only: None,
                    native_motion: false,
                },
            )
            .unwrap();
        let slidock_core_api::Unverified::Typed(presentation) = &lowered.presentation else {
            panic!("a typed presentation");
        };
        assert_eq!(presentation.media.len(), 1);
        before
    });
    // Lowering reads the index, the shown image and the input once more for its hash; the unused movie stays unread.
    assert!(reader.bytes_read() - before < (bytes.len() + unused.len() / 2) as u64);
}

#[test]
fn a_movie_set_to_play_across_slides_says_so_on_each_slide_and_no_other_does() {
    let mut p = Presentation::new();
    let clip = p.data("clip.mp4", movie("vide", "avc1", true, None));
    let poster = p.data("poster.png", png_with(&[2]));
    let film = |p: &mut Presentation, across: Option<bool>| {
        let mut message = json!({ "super": { "geometry": geometry(0.0, 0.0, 10.0, 10.0) },
                                  "movieData": clip, "posterImageData": poster });
        if let Some(across) = across {
            message["playsAcrossSlides"] = json!(across);
        }
        p.add("TSD.MovieArchive", message)
    };
    for across in [Some(true), Some(true), Some(false), None] {
        let movie = film(&mut p, across);
        p.slide(json!({ "drawables_z_order": [movie] }), json!({}), None);
    }
    let lowered = lower(&p.bytes());
    let flags: Vec<&Value> = (0..4)
        .map(|slide| &lowered.elements(slide)[0]["acrossSlides"])
        .collect();
    assert_eq!(
        flags,
        [&json!(true), &json!(true), &Value::Null, &Value::Null]
    );
    assert_eq!(lowered.codes(), Vec::<&str>::new());
}

#[test]
fn lowers_groups_nested_to_the_depth_limit_within_a_wasm_sized_stack() {
    let mut p = Presentation::new();
    let text = p.storage("deep", None, json!({}));
    let mut child = p.text_box(text, geometry(0.0, 0.0, 10.0, 10.0), None);
    let depth = Limits::default().group_depth;
    for _ in 0..depth {
        child = p.add(
            "TSD.GroupArchive",
            json!({ "super": { "geometry": geometry(0.0, 0.0, 10.0, 10.0) }, "children": [child] }),
        );
    }
    p.slide(json!({ "drawables_z_order": [child] }), json!({}), None);
    let bytes = p.bytes();
    // wasm32's default stack, which a release build lowers in; an unoptimised build keeps every element temporary (an
    // element is about 7 KB) on the stack, and takes twice as much.
    let stack = if cfg!(debug_assertions) { 2 } else { 1 } * 1024 * 1024;
    let lowered = std::thread::Builder::new()
        .stack_size(stack)
        .spawn(move || lower(&bytes).json)
        .unwrap()
        .join()
        .unwrap();
    let slide = &lowered["slides"][0];
    assert!(slide.get("failed").is_none());
    assert_eq!(count(slide["elements"].as_array().unwrap()), depth + 1);
}

#[test]
fn keeps_web_and_mail_links_on_drawables_and_reports_any_other_scheme() {
    let mut p = Presentation::new();
    let linked = |p: &mut Presentation, url: &str| {
        p.add(
            "TSD.ShapeArchive",
            json!({ "super": { "geometry": geometry(0.0, 0.0, 10.0, 10.0), "hyperlink_url": url } }),
        )
    };
    let drawables = vec![
        linked(&mut p, "http://example.com"),
        linked(&mut p, "HTTPS://example.com"),
        linked(&mut p, "mailto:someone@example.com"),
        linked(&mut p, "file:///etc/passwd"),
    ];
    p.slide(json!({ "drawables_z_order": drawables }), json!({}), None);
    let lowered = lower(&p.bytes());
    let links: Vec<&Value> = lowered.elements(0).iter().map(|e| &e["link"]).collect();
    assert_eq!(
        links,
        [
            &json!({ "kind": "url", "url": "http://example.com" }),
            &json!({ "kind": "url", "url": "HTTPS://example.com" }),
            &json!({ "kind": "url", "url": "mailto:someone@example.com" }),
            &Value::Null,
        ]
    );
    assert_eq!(lowered.codes(), ["KN1108"]);
    assert_eq!(
        lowered.features(lowered.presentation.slides[0].id.as_str()),
        ["element.link", "element.shape", "geometry.preset"]
    );
}

#[test]
fn mirrors_a_drawable_whose_geometry_flags_flip_it_bit_by_bit() {
    let mut p = Presentation::new();
    let photo = p.data("photo.png", PNG.to_vec());
    let clip = p.data("clip.mp4", movie("vide", "avc1", true, None));
    let at = |flags: u32| geometry_with(10.0, 20.0, 40.0, 30.0, flags, Some(30.0));
    // The mask's box is its image's left half, and its own flip mirrors the outline (fitted to the box) in it.
    let mask = p.add(
        "TSD.MaskArchive",
        json!({ "super": { "geometry": geometry_with(0.0, 0.0, 20.0, 30.0, 7, None) },
                "pathsource": { "bezier_path_source": { "natural_size": { "width": 20, "height": 30 }, "path": { "elements": [
                    { "type": 1, "points": [{ "x": 0, "y": 0 }] }, { "type": 2, "points": [{ "x": 10, "y": 0 }] },
                    { "type": 2, "points": [{ "x": 0, "y": 30 }] }, { "type": 5 },
                ] } } } }),
    );
    let child = shape(&mut p);
    let drawables = vec![
        p.add(
            "TSD.GroupArchive",
            json!({ "super": { "geometry": at(7) }, "children": [child] }),
        ),
        p.add(
            "TSD.ImageArchive",
            json!({ "super": { "geometry": at(11) }, "data": photo, "mask": mask }),
        ),
        p.add(
            "TSD.MovieArchive",
            json!({ "super": { "geometry": at(15) }, "movieData": clip }),
        ),
        p.add(
            "TSD.ShapeArchive",
            json!({ "super": { "geometry": at(3) } }),
        ),
        // 16 is no flag Keynote is known to set: the bits slidock knows still hold.
        p.add(
            "TSD.ShapeArchive",
            json!({ "super": { "geometry": at(16 | 7) } }),
        ),
    ];
    p.slide(json!({ "drawables_z_order": drawables }), json!({}), None);
    let lowered = lower(&p.bytes());
    let elements = lowered.elements(0);
    let flips: Vec<(&Value, &Value)> = elements
        .iter()
        .map(|e| (&e["frame"]["flipH"], &e["frame"]["flipV"]))
        .collect();
    let (yes, no) = (&json!(true), &json!(false));
    assert_eq!(
        flips,
        [(yes, no), (no, yes), (yes, yes), (no, no), (yes, no)]
    );
    // A flip leaves the box and the turn as they are: Keynote mirrors about the centre, then turns.
    for element in elements {
        let frame = &element["frame"];
        assert_eq!(
            (&frame["x"], &frame["w"], &frame["rotation"]),
            (&json!(10), &json!(40), &json!(330))
        );
    }
    assert_eq!(
        elements[1]["mask"][0]["segments"],
        json!([["M", 20, 0], ["L", 0, 0], ["L", 20, 30], ["Z"]])
    );
    assert!(lowered.codes().is_empty());
    assert!(
        lowered
            .features(lowered.presentation.slides[0].id.as_str())
            .contains(&"frame.flip")
    );
}

/// Keynote's flips as it stores them (its flip probe): none, Flip Horizontally, Flip Vertically (the horizontal bit
/// plus a half turn), both (a half turn, no bit), Flip Horizontally then 30°.
const STORED_FLIPS: [(u32, Option<f64>); 5] = [
    (3, None),
    (7, None),
    (7, Some(180.0)),
    (3, Some(180.0)),
    (7, Some(30.0)),
];

/// A slide of text boxes and of shapes holding text, one of each per [`STORED_FLIPS`] entry.
fn flipped_text() -> Vec<u8> {
    let mut p = Presentation::new();
    let mut drawables = vec![];
    for (i, (flags, angle)) in STORED_FLIPS.into_iter().enumerate() {
        let y = 120.0 * i as f64;
        let text = p.storage("TEXT box", None, json!({}));
        drawables.push(p.text_box(
            text,
            geometry_with(10.0, y, 200.0, 50.0, flags, angle),
            None,
        ));
        let text = p.storage("TEXT", None, json!({}));
        drawables.push(p.add_in(
            "TSWP.ShapeInfoArchive",
            json!({ "super": { "super": { "geometry": geometry_with(300.0, y, 200.0, 100.0, flags, angle) } },
                    "owned_storage": text }),
            support::keynote::SLIDE,
        ));
    }
    p.slide(json!({ "drawables_z_order": drawables }), json!({}), None);
    p.bytes()
}

#[test]
fn mirrors_a_drawables_text_with_it() {
    let lowered = lower(&flipped_text());
    let elements = lowered.elements(0);
    assert_eq!(elements.len(), 2 * STORED_FLIPS.len());
    let frames: Vec<(&Value, &Value, &Value)> = elements
        .iter()
        .map(|e| {
            assert_eq!(e["text"]["flipsWithFrame"], json!(true), "{}", e["id"]);
            let frame = &e["frame"];
            (&frame["flipH"], &frame["flipV"], &frame["rotation"])
        })
        .collect();
    let (yes, no) = (&json!(true), &json!(false));
    let expected = [
        (no, no, &json!(0)),
        (yes, no, &json!(0)),
        (yes, no, &json!(180)),
        (no, no, &json!(180)),
        (yes, no, &json!(330)),
    ];
    assert_eq!(
        frames,
        expected
            .iter()
            .flat_map(|frame| [*frame, *frame])
            .collect::<Vec<_>>()
    );

    // Drawn: the element carries the frame's turn and flips and holds its text, so the text is mirrored left to
    // right (H), top to bottom (V: H turned 180°), turned 180° (both), and mirrored then turned (H, 30°).
    let compiled = support::harness::compile(&flipped_text(), "flips.key");
    assert!(compiled.errors().is_empty(), "{:?}", compiled.errors());
    let documents = compiled.documents.unwrap();
    let (_, html) = documents
        .text
        .iter()
        .find(|(path, _)| path.starts_with("slides/"))
        .unwrap();
    let style_of = |tag_start: &str| -> Vec<String> {
        html.match_indices(tag_start)
            .map(|(at, _)| {
                let tag = &html[at..at + html[at..].find('>').unwrap()];
                let style = tag.find("style=\"").map_or("", |s| &tag[s + 7..]);
                style[..style.find('"').unwrap_or(style.len())].to_owned()
            })
            .collect()
    };
    let decl = |style: &str, name: &str| {
        style
            .split(';')
            .find_map(|d| d.strip_prefix(&format!("{name}:")).map(str::to_owned))
    };
    let shapes = style_of("<div data-el=");
    assert_eq!(shapes.len(), 2 * STORED_FLIPS.len());
    let drawn: Vec<(Option<String>, Option<String>)> = shapes
        .iter()
        .map(|style| (decl(style, "rotate"), decl(style, "scale")))
        .collect();
    let turn = |rotate: Option<&str>, scale: Option<&str>| {
        (rotate.map(str::to_owned), scale.map(str::to_owned))
    };
    let expected = [
        turn(None, None),
        turn(None, Some("-1 1")),
        turn(Some("180deg"), Some("-1 1")),
        turn(Some("180deg"), None),
        turn(Some("330deg"), Some("-1 1")),
    ];
    assert_eq!(
        drawn,
        expected
            .iter()
            .flat_map(|turn| [turn.clone(), turn.clone()])
            .collect::<Vec<_>>()
    );
    // A text box of its own (one the element does not hold itself) would only be there to turn the text apart.
    let text_boxes = style_of("<div data-sd-text");
    assert_eq!(text_boxes, Vec::<String>::new());
}

#[test]
fn records_unknown_type_ids_in_order_with_an_untyped_payload_as_minus_one() {
    let mut p = Presentation::new();
    p.add_record(Type::Id(999_999), vec![]);
    p.add_record(Type::Untyped, vec![]);
    p.add_record(Type::Id(888_888), vec![]);
    p.slide(json!({}), json!({}), None);
    let lowered = lower(&p.bytes());
    assert_eq!(
        lowered.json["ext"]["keynote"]["objects"]["unknownTypeIds"],
        json!([-1, 888_888, 999_999])
    );
}

/// `text` cut to its first `units` UTF-16 code units, a pair the cut splits dropped.
fn clipped(text: &str, units: usize) -> String {
    let mut length = 0;
    text.chars()
        .take_while(|c| {
            length += c.len_utf16();
            length <= units
        })
        .collect()
}

#[test]
fn cuts_links_alt_text_and_layout_names_at_their_lengths_in_utf16_units() {
    // An odd prefix, so each cut splits a pair: the UTF-16 length is one unit short of the cut.
    let long = |prefix: &str, units: usize| format!("{prefix}{}", "\u{1f600}".repeat(units));
    let (link, other, alt, name) = (
        long("https://e.x/a", 2048),
        long("file:///x", 256),
        long("a", 2048),
        long("a", 256),
    );
    let mut p = Presentation::new();
    let drawable = |p: &mut Presentation, extra: Value| {
        let mut base = json!({ "geometry": geometry(0.0, 0.0, 10.0, 10.0) });
        base.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        p.add("TSD.ShapeArchive", json!({ "super": base }))
    };
    let linked = drawable(&mut p, json!({ "hyperlink_url": link }));
    let refused = drawable(&mut p, json!({ "hyperlink_url": other }));
    let described = drawable(&mut p, json!({ "accessibility_description": alt }));
    let template = p.template(&name, vec![], json!({}));
    p.slide(
        json!({ "template_slide": template, "drawables_z_order": [linked, refused, described] }),
        json!({}),
        None,
    );
    let lowered = lower(&p.bytes());
    let elements = lowered.elements(0);
    let cut = [
        (&elements[0]["link"]["url"], &link, 2048),
        (&elements[2]["alt"], &alt, 2048),
        (&lowered.json["layouts"][kn(&template)]["name"], &name, 256),
    ];
    for (value, text, units) in cut {
        let value = value.as_str().unwrap();
        assert_eq!(value, clipped(text, units));
        assert_eq!(value.encode_utf16().count(), units - 1);
    }
    let reported: Vec<Value> = lowered
        .diagnostics
        .iter()
        .map(|d| serde_json::to_value(d).unwrap()["data"]["url"].clone())
        .collect();
    assert_eq!(reported, [json!(clipped(&other, 256))]);
}

/// A text box Keynote sizes to its text (geometry flags 0) in a group, centred, left- or right-aligned, inside one
/// group or two.
fn grown_text_in_groups() -> Vec<u8> {
    let mut p = Presentation::new();
    let aligned = |p: &mut Presentation, alignment: i32, x: f64, y: f64| {
        let style = p.paragraph_style(json!({}), None, json!({ "alignment": alignment }));
        let text = p.storage("Label text", Some(style), json!({}));
        p.text_box(text, geometry_with(x, y, 0.0, 0.0, 0, None), None)
    };
    let centred = aligned(&mut p, 2, 262.875, 140.459);
    // Anchored off the slide, past the edge each grows toward: left-aligned beyond the right edge, right-aligned
    // before the left one, centred beyond the right one.
    let beyond = [
        aligned(&mut p, 0, 2000.0, 0.0),
        aligned(&mut p, 1, -50.0, 0.0),
        aligned(&mut p, 2, 1930.0, 0.0),
    ];
    let arrow = shape(&mut p);
    let labelled = p.add(
        "TSD.GroupArchive",
        json!({ "super": { "geometry": geometry(750.882, 612.502, 525.75, 202.703) }, "children": [arrow, centred] }),
    );
    let left = aligned(&mut p, 0, 10.0, 0.0);
    let right = aligned(&mut p, 1, 90.0, 0.0);
    let inner = p.add(
        "TSD.GroupArchive",
        json!({ "super": { "geometry": geometry(400.0, 50.0, 100.0, 40.0) }, "children": [right] }),
    );
    let outer = p.add(
        "TSD.GroupArchive",
        json!({ "super": { "geometry": geometry(1000.0, 0.0, 600.0, 100.0) }, "children": [left, inner] }),
    );
    p.slide(
        json!({ "drawables_z_order": [labelled, outer, beyond[0], beyond[1], beyond[2]] }),
        json!({}),
        None,
    );
    p.bytes()
}

#[test]
fn wraps_a_grown_text_box_in_a_group_at_the_slides_edges_not_its_groups() {
    let lowered = lower(&grown_text_in_groups());
    let elements = lowered.elements(0);
    let body = |element: &Value| element["text"]["body"].clone();
    let labelled = &elements[0]["children"];
    // Centred on 750.882 + 262.875 = 1013.757 of 1920: twice the nearer (right) edge's 906.243.
    assert_eq!(
        body(&labelled[1]),
        json!({ "insets": { "l": 4, "t": 4, "r": 4, "b": 4 }, "anchor": "top", "wrap": "square",
                "autofit": { "kind": "resize", "width": true, "height": true, "anchorX": "center" },
                "maxWidth": 1812.486 })
    );
    let outer = &elements[1]["children"];
    // Left-aligned at 1000 + 10 grows to the right edge; right-aligned at 1000 + 400 + 90 to the left one.
    assert_eq!(body(&outer[0])["maxWidth"], json!(910));
    assert_eq!(body(&outer[1]["children"][0])["maxWidth"], json!(1490));
    // Off the slide past the edge it grows toward, a box keeps the lines its text takes: no cap.
    for element in &elements[2..5] {
        assert_eq!(body(element)["autofit"]["kind"], "resize");
        assert!(body(element).get("maxWidth").is_none(), "{element}");
    }
}
