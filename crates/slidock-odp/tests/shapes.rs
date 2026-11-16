//! Drawables: ids by native id or index, custom shapes mapped to presets, basic shapes, lines and paths, transforms
//! with rotation, mirroring and shear, groups placed on the page, links, fills and lines, and the frames that hold
//! pictures, media, embedded objects and tables.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::json;
use slidock_core_api::Limits;
use support::odp::{OdpSpec, PageSpec, odp};
use support::{assert_holds, fixtures, js, lower, lower_with};

#[test]
fn keys_drawables_by_their_native_id_else_their_document_order_index_and_suffixes_a_repeat() {
    let lowered = lower(&fixtures::shapes());
    let first = lowered.elements(0);
    let ids: Vec<&str> = first[..3]
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["od-p1.1", "od-p1.2", "od-p1.3"]);
    assert_eq!(lowered.named(0, "Next")["id"], "od-p1.next");
    assert_eq!(lowered.named(0, "Again")["id"], "od-p1.next.2");
    assert_eq!(lowered.named(1, "Photo")["id"], "od-p2.photo");
    let repeat = lowered
        .diagnostics
        .iter()
        .find(|d| d.code.code() == "OD9004")
        .unwrap();
    assert_eq!(repeat.node.as_ref().unwrap().as_str(), "od-p1.next.2");
    assert_eq!(
        lowered.named(0, "Rect")["origin"],
        json!({
            "kind": "source",
            "part": "content.xml",
            "path": "/office:document-content/office:body/office:presentation/draw:page[1]/draw:custom-shape[1]"
        })
    );
}

#[test]
fn lowers_shapes_with_their_geometry_fill_and_line() {
    let lowered = lower(&fixtures::shapes());
    assert_holds(
        lowered.named(0, "Rect"),
        json!({
            "kind": "shape",
            "frame": { "x": 72, "y": 36, "w": 144, "h": 72 },
            "geometry": { "kind": "preset", "name": "rect" },
            "fill": { "kind": "solid", "color": { "r": 0.9451, "a": 0.5 } },
            "stroke": null
        }),
    );
    let arrow = lowered.named(0, "Arrow");
    assert_eq!(arrow["frame"]["flipH"], true);
    assert_eq!(
        arrow["stroke"],
        js(&json!({
            "width": 3.005,
            "fill": { "kind": "solid", "color": { "kind": "rgb", "space": "srgb", "r": 0.34902, "g": 0.34902, "b": 0.34902, "a": 1 } },
            "dash": { "custom": [1, 3] },
            "cap": "flat",
            "join": "miter",
            "head": { "type": "oval", "w": "lg", "len": "lg" },
            "tail": { "type": "triangle", "w": "med", "len": "med" }
        }))
    );
    assert_holds(
        lowered.named(0, "Rounded"),
        json!({
            "frame": { "rotation": 90 },
            "geometry": { "name": "roundRect", "adjust": { "adj": 30000 } },
            "fill": { "kind": "odf", "style": "axial", "angle": 0, "stops": [{ "pos": 0 }, { "pos": 1 }] },
            "stroke": { "width": 0.75 }
        }),
    );
    assert_holds(
        lowered.named(0, "Native"),
        json!({
            "fill": { "kind": "image", "mode": "stretch" },
            "stroke": { "cap": "square", "dash": { "custom": [2, 1, 1, 1] }, "fill": { "color": { "a": 0.5 } } }
        }),
    );
    assert_holds(
        lowered.named(0, "Corner"),
        json!({ "geometry": { "name": "roundRect", "adjust": { "adj": 25000 } } }),
    );
    assert_holds(
        lowered.named(0, "Pie"),
        json!({ "geometry": { "name": "pie", "adjust": { "adj1": 10_800_000, "adj2": 16_200_000 } } }),
    );
}

#[test]
fn lowers_lines_from_their_ends_and_polygons_and_paths_in_their_view_box() {
    let lowered = lower(&fixtures::shapes());
    assert_holds(
        lowered.named(0, "Back"),
        json!({
            "frame": { "x": 576, "y": 36, "w": 72, "h": 36, "flipH": true, "flipV": true },
            "geometry": { "name": "line" }
        }),
    );
    assert_eq!(
        lowered.named(0, "Tri")["geometry"],
        json!({
            "kind": "custom",
            "paths": [{
                "w": 1000, "h": 1000, "fill": true, "stroke": true, "fillRule": "nonzero",
                "segments": [["M", 0, 1000], ["L", 500, 0], ["L", 1000, 1000], ["Z"]]
            }]
        })
    );
    assert_holds(
        lowered.named(0, "Routed"),
        json!({ "geometry": { "name": "straightConnector1" } }),
    );
}

#[test]
fn places_a_group_at_its_childrens_union_and_moves_them_into_it() {
    let lowered = lower(&fixtures::shapes());
    let group = lowered.named(0, "Pair");
    assert_eq!(
        group["frame"],
        json!({ "x": 432, "y": 180, "w": 108, "h": 72, "rotation": 0, "flipH": false, "flipV": false })
    );
    let places: Vec<_> = group["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|child| (child["frame"]["x"].clone(), child["frame"]["y"].clone()))
        .collect();
    assert_eq!(places, [(json!(0), json!(0)), (json!(72), json!(36))]);
}

#[test]
fn reads_links_click_actions_alt_text_and_visibility() {
    let lowered = lower(&fixtures::shapes());
    assert_eq!(
        lowered.named(0, "Linked")["link"],
        json!({ "kind": "url", "url": "https://example.com/a?b=1" })
    );
    assert_holds(
        lowered.named(0, "Next"),
        json!({ "link": { "kind": "action", "action": "next" }, "alt": "Goes on" }),
    );
    assert_eq!(lowered.named(0, "Again")["hidden"], true);
}

#[test]
fn keeps_what_it_cannot_model_as_foreign_nodes_and_drops_shears_and_unreadable_transforms_loudly() {
    // A hand-drawn shape is drawn from its enhanced path (tests/enhanced.rs); a 3D scene stays Foreign.
    let lowered = lower(&fixtures::shapes());
    assert_holds(
        lowered.named(0, "Freeform"),
        json!({ "kind": "shape", "geometry": { "kind": "custom", "paths": [{ "w": 21600, "h": 21600 }] } }),
    );
    assert_holds(
        lowered.named(0, "Cube"),
        json!({ "kind": "foreign", "type": "3d" }),
    );
    let codes: Vec<&str> = lowered
        .diagnostics
        .iter()
        .filter(|d| d.container.as_ref().is_some_and(|c| c.as_str() == "od-p1"))
        .map(|d| d.code.code())
        .collect();
    assert_eq!(
        codes,
        ["OD9004", "OD9106", "OD9107", "OD9109", "OD9110", "OD9111"]
    );
}

#[test]
fn crops_pictures_by_their_intrinsic_size_and_keeps_their_opacity_and_mirroring() {
    let lowered = lower(&fixtures::shapes());
    assert_holds(
        lowered.named(1, "Photo"),
        json!({
            "kind": "picture",
            "crop": { "l": 0.25, "t": 0, "r": 0.25, "b": 0 },
            "opacity": 0.5,
            "frame": { "flipH": true }
        }),
    );
    let recolor = lowered
        .diagnostics
        .iter()
        .find(|d| d.code.code() == "OD9305")
        .unwrap();
    assert_eq!(
        js(&recolor.data),
        json!({ "adjustments": "greyscale, luminance" })
    );
    let features = lowered.features("od-p2");
    for feature in [
        "image.crop",
        "image.opacity",
        "image.recolor",
        "image.vector",
    ] {
        assert!(features.contains(&feature), "{feature}");
    }
}

#[test]
fn declines_external_missing_and_cropped_away_pictures() {
    let lowered = lower(&fixtures::shapes());
    let names: Vec<&str> = lowered
        .elements(1)
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect();
    for name in ["Remote", "Gone", "Sliver"] {
        assert!(!names.contains(&name), "{name}");
    }
    let declined: Vec<&str> = lowered
        .diagnostics
        .iter()
        .filter(|d| d.severity == slidock_ir::diagnostics::Severity::Decline)
        .map(|d| d.code.code())
        .collect();
    assert_eq!(declined, ["OD9302", "OD9303", "OD9304"]);
}

#[test]
fn lowers_embedded_objects_as_foreign_nodes_with_the_replacement_image_as_fallback_and_tables_as_tables()
 {
    let lowered = lower(&fixtures::shapes());
    let chart = lowered.named(1, "Chart");
    assert_holds(chart, json!({ "kind": "foreign", "type": "chart" }));
    assert!(chart["fallback"]["media"].is_string());
    // Unsized tracks take the frame's size.
    assert_holds(
        lowered.named(1, "Grid"),
        json!({
            "kind": "table",
            "frame": { "x": 216, "y": 144, "w": 144, "h": 72 },
            "columns": [{ "width": 144 }],
            "rows": [{ "height": 72, "cells": [{ "text": { "paragraphs": [{ "inlines": [] }] } }] }]
        }),
    );
}

#[test]
fn lowers_movies_with_their_poster_and_a_sound_as_its_picture() {
    let lowered = lower(&fixtures::shapes());
    let clip = lowered.named(1, "Clip");
    assert_holds(
        clip,
        json!({ "kind": "media", "mediaKind": "video", "loop": true, "muted": false }),
    );
    assert!(clip["media"].is_string() && clip["poster"].is_string());
    assert!(lowered.named(1, "Online").get("media").is_none());
    assert_holds(lowered.named(1, "Sound"), json!({ "kind": "picture" }));
    let lossy: Vec<&str> = lowered
        .diagnostics
        .iter()
        .filter(|d| {
            d.container.as_ref().is_some_and(|c| c.as_str() == "od-p2")
                && d.severity == slidock_ir::diagnostics::Severity::Lossy
        })
        .map(|d| d.code.code())
        .collect();
    assert_eq!(lossy, ["OD9101", "OD9105", "OD9301", "OD9305"]);
    let entry = &lowered.json["media"][clip["media"].as_str().unwrap()];
    assert_holds(entry, json!({ "type": "video/mp4", "codec": "avc1" }));
}

#[test]
fn lowers_master_page_drawables_its_presentation_objects_as_placeholders() {
    let lowered = lower(&fixtures::shapes());
    let layout = &lowered.json["layouts"]["od-m.Default"];
    let elements: Vec<_> = layout["elements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["id"].clone(),
                e["kind"].clone(),
                e["placeholder"]["role"].clone(),
            )
        })
        .collect();
    assert_eq!(
        elements,
        [
            (json!("od-m.Default.1"), json!("picture"), json!(null)),
            (json!("od-m.Default.2"), json!("shape"), json!("title"))
        ]
    );
    assert_eq!(layout["elements"][1]["origin"]["part"], "styles.xml");
}

#[test]
fn draws_no_empty_presentation_object_on_a_slide() {
    // LibreOffice 25.2 (PDF): a title with text and no style takes "standard"'s fill; the same title flagged as an
    // empty presentation object (`presentation:placeholder="true"`) draws nothing.
    let title = |flag: &str, body: &str| {
        format!(
            r#"<draw:frame presentation:class="title" {flag} draw:name="Title" svg:x="1cm" svg:y="1cm" svg:width="5cm" svg:height="2cm"><draw:text-box>{body}</draw:text-box></draw:frame>"#
        )
    };
    let lowered = lower(&odp(&OdpSpec::pages(vec![
        PageSpec::body(title("", "<text:p>Title</text:p>")),
        PageSpec::body(title(r#"presentation:placeholder="true""#, "")),
    ])));
    assert_holds(
        lowered.named(0, "Title"),
        json!({ "kind": "shape", "placeholder": { "role": "title" }, "fill": { "kind": "solid" } }),
    );
    assert_eq!(lowered.elements(1), &Vec::<serde_json::Value>::new());
}

fn frame(text: &str) -> String {
    format!(
        r#"<draw:frame svg:x="1cm" svg:y="1cm" svg:width="5cm" svg:height="2cm"><draw:text-box><text:p>{text}</text:p></draw:text-box></draw:frame>"#
    )
}

fn failures(bytes: &[u8], limits: Limits) -> Vec<serde_json::Value> {
    let lowered = lower_with(bytes, None, limits);
    lowered.json["slides"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.get("failed").cloned().unwrap_or_default())
        .collect()
}

#[test]
fn fails_only_the_page_with_more_drawables_than_the_limit() {
    let bytes = odp(&OdpSpec::pages(vec![
        PageSpec::body(frame("a").repeat(3)),
        PageSpec::body(frame("b").repeat(2)),
    ]));
    let limits = Limits {
        elements_per_container: 2,
        ..Limits::default()
    };
    assert_eq!(
        failures(&bytes, limits),
        [json!({ "code": "OD9901" }), json!(null)]
    );
}

#[test]
fn fails_only_the_page_whose_text_body_passes_the_text_limit() {
    let bytes = odp(&OdpSpec::pages(vec![
        PageSpec::body(frame("eleven char")),
        PageSpec::body(frame("ten chars!")),
    ]));
    let limits = Limits {
        text_per_body: 10,
        ..Limits::default()
    };
    assert_eq!(
        failures(&bytes, limits),
        [json!({ "code": "OD9901" }), json!(null)]
    );
}

#[test]
fn fails_only_the_shape_whose_path_passes_the_segment_limit() {
    let path = r#"<draw:path svg:width="1cm" svg:height="1cm" svg:viewBox="0 0 10 10" svg:d="M0 0 L1 1 L2 2 L3 3"/>"#;
    let bytes = odp(&OdpSpec::pages(vec![PageSpec::body(path), PageSpec::new()]));
    let limits = Limits {
        path_segments: 3,
        ..Limits::default()
    };
    assert_eq!(
        failures(&bytes, limits),
        [json!({ "code": "OD9901" }), json!(null)]
    );
}

#[test]
fn draws_a_marker_it_cannot_name_as_a_triangle_and_says_so() {
    let bytes = odp(&OdpSpec {
        styles: r#"<draw:marker draw:name="Square" svg:viewBox="0 0 10 10" svg:d="M0 0h10v10h-10z"/>"#.to_owned(),
        automatic_styles: concat!(
            r#"<style:style style:name="gr1" style:family="graphic"><style:graphic-properties draw:stroke="solid" "#,
            r#"svg:stroke-width="0.1cm" draw:marker-end="Square" draw:marker-end-width="0.3cm"/></style:style>"#
        )
        .to_owned(),
        ..OdpSpec::pages(vec![PageSpec::body(
            r#"<draw:line draw:style-name="gr1" svg:x1="1cm" svg:y1="1cm" svg:x2="5cm" svg:y2="1cm"/>"#,
        )])
    });
    let lowered = lower(&bytes);
    assert_eq!(lowered.elements(0)[0]["stroke"]["tail"]["type"], "triangle");
    let marker = lowered
        .diagnostics
        .iter()
        .find(|d| d.code.code() == "OD9108")
        .unwrap();
    assert_eq!(js(&marker.data), json!({ "marker": "Square" }));
}

/// Subtracting a view box's origin after rounding the points, with the view box size kept as written, would leave these
/// off the 0.001 grid and fail the container (IR0007); the frontend moves them first, so they round to it.
#[test]
fn rounds_a_path_to_the_grid_after_moving_it_to_its_view_box_origin() {
    let path = concat!(
        r#"<draw:path draw:name="P" svg:x="1cm" svg:y="1cm" svg:width="1cm" svg:height="1cm" "#,
        r#"svg:viewBox="0.1 0.2 20.12345 30.0006" svg:d="M0.3 0.7 L0.1 0.2"/>"#
    );
    let lowered = lower(&odp(&OdpSpec::pages(vec![PageSpec::body(path)])));
    assert_eq!(lowered.codes(), []);
    assert_holds(
        &lowered.named(0, "P")["geometry"],
        json!({ "paths": [{
            "w": 20.123, "h": 30.001,
            "segments": [["M", 0.2, 0.5], ["L", 0, 0]]
        }] }),
    );
}

/// LibreOffice writes an SVG picture as the frame's first `draw:image` and a PNG preview after it: the SVG is drawn
/// when it passes the allow-list, the preview when it does not (reported, OD9306); a vector stand-in it cannot show
/// never wins over a raster, and a refused SVG with no preview stays the frame's picture, unreported here.
#[test]
fn draws_a_frames_svg_before_its_raster_preview_when_the_svg_passes() {
    const DRAWN: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 190 108" width="190" height="108"><path d="M 0 0 H 190 V 108 H 0 Z M 9 9 V 99 H 181 V 9 Z" fill="#FE7867" fill-rule="evenodd"/></svg>"##;
    const SCRIPTED: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><script>alert(1)</script></svg>"#;
    let frame = |name: &str, first: &str| {
        format!(
            concat!(
                r#"<draw:frame draw:name="{name}" svg:x="1cm" svg:y="1cm" svg:width="5cm" svg:height="3cm">"#,
                r#"<draw:image xlink:href="{first}" draw:mime-type="image/svg+xml"/>"#,
                r#"<draw:image xlink:href="Pictures/preview.png" draw:mime-type="image/png"/></draw:frame>"#
            ),
            name = name,
            first = first
        )
    };
    let body = [
        frame("Drawn", "Pictures/drawn.svg"),
        frame("Scripted", "Pictures/scripted.svg"),
        frame("Emf", "Pictures/drawn.emf"),
        r#"<draw:frame draw:name="Alone" svg:width="5cm" svg:height="3cm"><draw:image xlink:href="Pictures/scripted.svg"/></draw:frame>"#.to_owned(),
    ]
    .concat();
    let bytes = odp(&OdpSpec::pages(vec![PageSpec::body(body)])
        .part("Pictures/drawn.svg", DRAWN)
        .part("Pictures/scripted.svg", SCRIPTED)
        .part("Pictures/drawn.emf", b"\x01\0\0\0".to_vec())
        .part("Pictures/preview.png", fixtures::png(4, 2)));
    let lowered = lower(&bytes);
    let media = &lowered.json["media"];
    let part = |name: &str| {
        let sha = lowered.named(0, name)["media"].as_str().unwrap().to_owned();
        media[sha.as_str()]["origin"]["part"].clone()
    };
    assert_eq!(
        [part("Drawn"), part("Scripted"), part("Emf")],
        [
            json!("Pictures/drawn.svg"),
            json!("Pictures/preview.png"),
            json!("Pictures/preview.png")
        ]
    );
    let svg = &media[lowered.named(0, "Drawn")["media"].as_str().unwrap()];
    assert_eq!(
        (&svg["type"], &svg["width"], &svg["height"]),
        (&json!("image/svg+xml"), &json!(190), &json!(108))
    );
    // The refused SVG is recorded only where nothing stands in for it.
    assert_eq!(part("Alone"), json!("Pictures/scripted.svg"));
    assert_eq!(media.as_object().unwrap().len(), 3);
    let reported: Vec<(&str, &str, String)> = lowered
        .diagnostics
        .iter()
        .map(|d| {
            (
                d.code.code(),
                d.node.as_ref().map_or("", |node| node.as_str()),
                d.message.clone(),
            )
        })
        .collect();
    let scripted = lowered.named(0, "Scripted")["id"].as_str().unwrap();
    assert_eq!(
        reported,
        [(
            "OD9306",
            scripted,
            "The picture is drawn from its raster preview: its SVG Pictures/scripted.svg is not shown: the element script is not shown".to_owned()
        )]
    );
}
