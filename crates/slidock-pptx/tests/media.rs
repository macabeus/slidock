//! Pictures, movies, sound, notes, transitions and animations, as lowered and as printed.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::{Value, json};
use slidock_ir::diagnostics::Severity;
use slidock_ir::ids::Sha256;
use support::fixtures::{media, media_with};
use support::{Lowered, assert_matches, compile_bytes, json, lower};

fn element(lowered: &Lowered, id: &str) -> Option<Value> {
    lowered
        .presentation()
        .slides
        .iter()
        .flat_map(|slide| &slide.elements)
        .find(|element| element.id().as_str() == id)
        .map(json)
}

#[test]
fn reads_opacity_crops_clips_and_outlines() {
    let lowered = lower(&media());
    assert_matches(
        &element(&lowered, "pp-256.10").unwrap(),
        json!({ "kind": "picture", "opacity": 0.4, "alt": "Logo" }),
    );
    assert_eq!(
        element(&lowered, "pp-256.11").unwrap()["crop"],
        json!({ "l": 0.1, "t": -0.05, "r": 0.2, "b": 0.92 })
    );
    assert_matches(
        &element(&lowered, "pp-256.13").unwrap(),
        json!({
            "clip": { "kind": "preset", "name": "ellipse" },
            "stroke": { "width": 2, "fill": { "kind": "solid", "color": { "slot": "accent1" } } },
        }),
    );
}

#[test]
fn records_each_picture_once_by_the_hash_of_its_bytes_typed_and_sized_from_them() {
    let lowered = lower(&media());
    let sha = element(&lowered, "pp-256.10").unwrap()["media"].clone();
    let entries = json(&lowered.presentation().media);
    let entry = &entries[sha.as_str().unwrap()];
    assert_eq!(
        *entry,
        json!({
            "sha256": sha,
            "type": "image/png",
            "bytes": entry["bytes"],
            "width": 4,
            "height": 2,
            "origin": { "kind": "source", "part": "ppt/media/image1.png" },
        })
    );
    assert_eq!(entries.as_object().unwrap().len(), 3);
}

#[test]
fn declines_pictures_it_cannot_show_cropped_to_nothing_linked_missing() {
    let lowered = lower(&media());
    for id in ["pp-256.14", "pp-256.15", "pp-256.16"] {
        assert_eq!(element(&lowered, id), None, "{id}");
    }
    let codes: Vec<String> = ["pp-256.14", "pp-256.15", "pp-256.16"]
        .iter()
        .flat_map(|id| lowered.codes(id))
        .collect();
    assert_eq!(codes, ["PX2305", "PX2301", "PX2307"]);
    assert!(
        lowered
            .diagnostics
            .iter()
            .filter(|d| matches!(d.code.code(), "PX2301" | "PX2305" | "PX2307"))
            .all(|d| d.severity == Severity::Decline)
    );
}

#[test]
fn notes_colour_effects_fill_insets_and_tiling_it_does_not_show() {
    let lowered = lower(&media());
    assert_eq!(lowered.codes("pp-256.12"), ["PX2302"]);
    assert_eq!(lowered.codes("pp-256.17"), ["PX2303"]);
    assert_eq!(lowered.codes("pp-256.18"), ["PX2104"]);
}

#[test]
fn lets_a_picture_placeholder_inherit_its_frame() {
    let lowered = lower(&media());
    let placeholder = element(&lowered, "pp-256.19").unwrap();
    assert_matches(
        &placeholder,
        json!({ "placeholder": { "role": "pic", "index": 13 } }),
    );
    assert!(placeholder.get("frame").is_none());
}

#[test]
fn reads_a_movie_with_its_embedded_bytes_codec_poster_and_trim() {
    let lowered = lower(&media());
    let movie = element(&lowered, "pp-257.30").unwrap();
    assert_matches(
        &movie,
        json!({ "kind": "media", "mediaKind": "video", "trim": { "startMs": 1500 } }),
    );
    let entries = json(&lowered.presentation().media);
    assert_matches(
        &entries[movie["media"].as_str().unwrap()],
        json!({ "type": "video/mp4", "codec": "avc1" }),
    );
    assert_matches(
        &entries[movie["poster"].as_str().unwrap()],
        json!({ "type": "image/png", "width": 8 }),
    );
    let online = element(&lowered, "pp-257.31").unwrap();
    assert!(online.get("media").is_none());
    assert_eq!(online["poster"], movie["poster"]);
    assert_eq!(lowered.codes("pp-257.31"), ["PX2306"]);
}

#[test]
fn reads_a_trim_against_the_movies_length() {
    for (trim, movie_ms, expected, codes) in [
        (
            "st=\"1500\" end=\"2000\"",
            Some(10_000),
            json!({ "startMs": 1500, "endMs": 8000 }),
            vec![],
        ),
        (
            "end=\"2000\"",
            Some(10_000),
            json!({ "startMs": 0, "endMs": 8000 }),
            vec![],
        ),
        // The cut from the end needs the movie's length to become an end time.
        (
            "st=\"1500\" end=\"2000\"",
            None,
            json!({ "startMs": 1500 }),
            vec!["PX2309"],
        ),
        ("end=\"2000\"", None, Value::Null, vec!["PX2309"]),
    ] {
        let lowered = lower(&media_with(trim, movie_ms));
        let movie = element(&lowered, "pp-257.30").unwrap();
        assert_eq!(
            movie.get("trim").cloned().unwrap_or(Value::Null),
            expected,
            "{trim}"
        );
        let found: Vec<String> = lowered
            .codes("pp-257.30")
            .into_iter()
            .filter(|code| code == "PX2309")
            .collect();
        assert_eq!(found, codes, "{trim}");
    }
}

#[test]
fn keeps_a_sound_picture_as_a_picture_without_the_sound() {
    let lowered = lower(&media());
    assert_eq!(element(&lowered, "pp-257.32").unwrap()["kind"], "picture");
    assert_eq!(lowered.codes("pp-257.32"), ["PX2304"]);
}

#[test]
fn reads_notes_as_text_and_leaves_empty_notes_out() {
    let lowered = lower(&media());
    let slides = &lowered.presentation().slides;
    let notes = json(&slides[0].notes);
    let texts: Vec<Vec<Value>> = notes["paragraphs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            p["inlines"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i["text"].clone())
                .collect()
        })
        .collect();
    assert_eq!(texts, [vec![json!("Say hello")], vec![json!("then go")]]);
    assert!(slides[1].notes.is_none());
}

#[test]
fn reads_transitions_as_data_an_office_2010_one_from_its_alternate_content() {
    let lowered = lower(&media());
    let slides = &lowered.presentation().slides;
    assert_eq!(
        json(&slides[0].transition),
        json!({
            "effect": { "id": "pptx:fade", "fallback": "dissolve" },
            "durationMs": 750,
            "advance": { "onClick": false, "afterMs": 3000 },
        })
    );
    assert_eq!(
        json(&slides[1].transition),
        json!({
            "effect": { "id": "pptx:prism", "fallback": "dissolve" },
            "durationMs": 1600,
            "advance": { "onClick": true },
        })
    );
}

#[test]
fn drops_animations_with_a_lossy_diagnostic() {
    let lowered = lower(&media());
    assert_matches(
        &json(lowered.find("PX2401").unwrap()),
        json!({ "container": "pp-257", "feature": "animation" }),
    );
}

#[test]
fn hands_the_writer_each_items_bytes_a_stored_entry_as_a_range_of_the_input() {
    for level in [6, 0] {
        let bytes = support::zip(&support::unzip(&media()), level);
        let lowered = lower(&bytes);
        for (sha256, entry) in &lowered.presentation().media {
            let handle = lowered.lowered.media.open(sha256).unwrap();
            assert_eq!(handle.bytes, entry.bytes);
            let data = match handle.source {
                slidock_core_api::MediaSource::Bytes(data) if level != 0 => data.to_vec(),
                slidock_core_api::MediaSource::InputRange { offset, len } if level == 0 => {
                    bytes[offset as usize..(offset + len) as usize].to_vec()
                }
                other => panic!("level {level}: {other:?}"),
            };
            assert_eq!(Sha256::of(&data), *sha256);
        }
        assert!(
            lowered
                .lowered
                .media
                .open(&Sha256::of(b"nothing"))
                .is_none()
        );
    }
}

#[test]
fn prints_pictures_movies_with_their_posters_notes_and_transitions() {
    let compiled = compile_bytes(&media(), "presentation.pptx");
    assert!(compiled.errors().is_empty(), "{:?}", compiled.errors());
    let slide = compiled.text("slides/pp-257.html");
    let at = slide.find("<video src=\"../media/").unwrap();
    let tag = &slide[at..at + slide[at..].find('>').unwrap()];
    assert!(tag.contains(".mp4\" poster=\"../media/"), "{tag}");
    assert!(tag.contains(".png\""), "{tag}");
    let manifest: Value = serde_json::from_str(compiled.text("presentation.json")).unwrap();
    assert_matches(
        &manifest["slides"][0],
        json!({ "notes": "Say hello\nthen go", "transition": { "effect": "pptx:fade" } }),
    );
    assert_eq!(compiled.emitted.as_ref().unwrap().files.media.len(), 3);
}

/// Office 2016 keeps an SVG picture's raster preview in `a:blip` and the SVG in `asvg:svgBlip`: the SVG is drawn when
/// it passes the allow-list, the preview when it does not (reported, PX2310) or when there is none.
#[test]
fn draws_an_svg_blip_in_place_of_its_raster_preview_when_the_svg_passes() {
    const DRAWN: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 190 108" width="190" height="108"><path d="M 0 0 H 190 V 108 H 0 Z M 9 9 V 99 H 181 V 9 Z" fill="#FE7867" fill-rule="evenodd"/></svg>"##;
    const SCRIPTED: &[u8] =
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><script>alert(1)</script></svg>"#;
    let picture = |id: u32, svg: Option<&str>| {
        let ext = svg.map_or_else(String::new, |rel| {
            format!(
                "<a:extLst><a:ext uri=\"{{96DAC541-7B7A-43D3-8B79-37D633B846F1}}\"><asvg:svgBlip xmlns:asvg=\"http://schemas.microsoft.com/office/drawing/2016/SVG/main\" r:embed=\"{rel}\"/></a:ext></a:extLst>"
            )
        });
        format!(
            "<p:pic><p:nvPicPr><p:cNvPr id=\"{id}\" name=\"Picture {id}\"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed=\"rIdPng\">{ext}</a:blip><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>",
            support::xfrm(0.0, 0.0, 190.0, 108.0, "")
        )
    };
    let tree = [
        picture(2, Some("rIdSvg")),
        picture(3, Some("rIdScripted")),
        picture(4, None),
        picture(5, Some("rIdGone")),
    ]
    .concat();
    let bytes = support::pptx(support::PptxSpec {
        slides: vec![support::SlideSpec {
            rels: vec![
                support::rel("rIdPng", "image", "../media/image1.png"),
                support::rel("rIdSvg", "image", "../media/image2.svg"),
                support::rel("rIdScripted", "image", "../media/image3.svg"),
            ],
            ..support::slide(tree)
        }],
        parts: vec![
            (
                "ppt/media/image1.png".to_owned(),
                support::fixtures::png(4, 2),
            ),
            ("ppt/media/image2.svg".to_owned(), DRAWN.to_vec()),
            ("ppt/media/image3.svg".to_owned(), SCRIPTED.to_vec()),
        ],
        ..support::PptxSpec::default()
    });
    let lowered = lower(&bytes);
    let entries = json(&lowered.presentation().media);
    let part = |id: &str| {
        let sha = element(&lowered, id).unwrap()["media"].clone();
        entries[sha.as_str().unwrap()]["origin"]["part"].clone()
    };
    assert_eq!(
        [part("pp-256.2"), part("pp-256.3"), part("pp-256.4")],
        [
            json!("ppt/media/image2.svg"),
            json!("ppt/media/image1.png"),
            json!("ppt/media/image1.png")
        ]
    );
    let svg = &entries[element(&lowered, "pp-256.2").unwrap()["media"]
        .as_str()
        .unwrap()];
    assert_eq!(
        (&svg["type"], &svg["width"], &svg["height"]),
        (&json!("image/svg+xml"), &json!(190), &json!(108))
    );
    // The refused SVG is never recorded; the preview drawn in its place is reported, as is an svgBlip with no part.
    assert_eq!(entries.as_object().unwrap().len(), 2);
    assert_eq!(part("pp-256.5"), json!("ppt/media/image1.png"));
    let codes: Vec<Vec<String>> = ["pp-256.2", "pp-256.3", "pp-256.4", "pp-256.5"]
        .iter()
        .map(|id| lowered.codes(id))
        .collect();
    assert_eq!(
        codes,
        [
            vec![],
            vec!["PX2310".to_owned()],
            vec![],
            vec!["PX2310".to_owned()]
        ]
    );
}
