//! Shapes: ids, origins, frames, geometry, fills, lines, effects, style references, groups, connectors, graphic
//! frames, alternate content and the limits that fail a container alone.
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
use slidock_ir::features::FeatureId;
use support::fixtures::shapes;
use support::{
    PptxSpec, Sp, assert_matches, body, compile_bytes, element, json, lower, lower_with, pptx,
    slide, sp, xfrm,
};

fn lowered() -> support::Lowered {
    lower(&shapes())
}

#[test]
fn keeps_native_ids_names_and_an_origin_with_the_part_path_and_object() {
    let lowered = lowered();
    let slide = &lowered.presentation().slides[0];
    let shape = element(slide, "pp-256.10");
    assert_eq!(shape["name"], "Rotated");
    assert_eq!(
        shape["origin"],
        json!({ "kind": "source", "part": "ppt/slides/slide1.xml", "path": "/p:sld/p:cSld/p:spTree/p:sp[1]", "object": "10" })
    );
    assert_eq!(element(slide, "pp-256.70.2")["name"], "Same id");
    assert!(lowered.codes("pp-256.70.2").contains(&"PX2004".to_owned()));
}

#[test]
fn reads_frames_offset_extent_rotation_and_flips_in_points_and_degrees() {
    let lowered = lowered();
    let slide = &lowered.presentation().slides[0];
    assert_eq!(
        element(slide, "pp-256.10")["frame"],
        json!({ "x": 20, "y": 20, "w": 100, "h": 50, "rotation": 90, "flipH": true, "flipV": false })
    );
    let features = &lowered.lowered.features["pp-256"];
    assert!(
        features.contains(&FeatureId::FrameRotation) && features.contains(&FeatureId::FrameFlip)
    );
}

#[test]
fn reads_preset_geometry_with_its_adjustments_and_custom_paths_with_arcs_as_cubics() {
    let lowered = lowered();
    let slide = &lowered.presentation().slides[0];
    assert_eq!(
        element(slide, "pp-256.11")["geometry"],
        json!({ "kind": "preset", "name": "roundRect", "adjust": { "adj": 30000 } })
    );
    let custom = &element(slide, "pp-256.15")["geometry"];
    assert_eq!(custom["kind"], "custom");
    let segments = custom["paths"][0]["segments"].as_array().unwrap();
    assert_eq!(segments[1], json!(["L", 635000, 0]));
    assert_eq!(segments[2][0], "C");
    assert_eq!(
        segments[2].as_array().unwrap()[5..],
        [json!(1270000), json!(635000)]
    );
    assert_matches(
        &custom["paths"][1],
        json!({ "fill": false, "stroke": false, "w": 1270000 }),
    );
}

#[test]
fn lowers_a_preset_outside_the_standard_as_a_foreign_node() {
    let lowered = lowered();
    assert_matches(
        &element(&lowered.presentation().slides[0], "pp-256.16"),
        json!({ "kind": "foreign", "type": "unknown", "payload": { "geometry": "fancyStar" } }),
    );
}

#[test]
fn keeps_gradients_in_drawingml_terms_with_stops_sorted_and_theme_colours_symbolic() {
    let lowered = lowered();
    let slide = &lowered.presentation().slides[0];
    let fill = &element(slide, "pp-256.11")["fill"];
    assert_matches(
        fill,
        json!({ "kind": "linear", "angle": 90, "scaled": true, "rotateWithShape": false }),
    );
    assert_matches(
        &fill["stops"][0],
        json!({ "pos": 0, "color": { "slot": "accent1" } }),
    );
    let ops: Vec<&str> = fill["stops"][0]["color"]["mods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["op"].as_str().unwrap())
        .collect();
    assert_eq!(ops, ["lumMod", "lumOff"]);
    assert_eq!(fill["stops"][1]["pos"], 1);
    assert_matches(
        &element(slide, "pp-256.12")["fill"],
        json!({ "kind": "path", "path": "rect", "focus": { "l": 0.5, "t": 0.25, "r": 0.5, "b": 0.75 } }),
    );
    assert!(lowered.codes("pp-256.12").is_empty());
}

#[test]
fn approximates_patterns_and_drops_effects_each_with_a_lossy_diagnostic() {
    let lowered = lowered();
    let shape = element(&lowered.presentation().slides[0], "pp-256.13");
    assert_matches(
        &shape["fill"],
        json!({ "kind": "solid", "color": { "r": 0.75294 } }),
    );
    // The glow is dropped; the shadow beside it is drawn.
    assert_eq!(shape["shadow"]["dx"], json!(1.575));
    let mut codes = lowered.codes("pp-256.13");
    codes.sort();
    assert_eq!(codes, ["PX2101", "PX2103"]);
}

#[test]
fn reads_lines_width_literal_colour_with_alpha_dash_cap_join_and_line_ends() {
    let lowered = lowered();
    let slide = &lowered.presentation().slides[0];
    assert_eq!(
        element(slide, "pp-256.10")["stroke"],
        json!({
            "width": 3,
            "fill": { "kind": "solid", "color": { "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 0.5 } },
            "dash": "dot",
            "cap": "round",
            "join": "bevel",
        })
    );
    assert_matches(
        &element(slide, "pp-256.30")["stroke"],
        json!({ "head": { "type": "oval", "w": "sm", "len": "sm" }, "tail": { "type": "triangle", "w": "med", "len": "lg" } }),
    );
    assert_eq!(element(slide, "pp-256.11")["stroke"], json!(null));
}

#[test]
fn resolves_style_references_the_reference_colour_fills_phclr_keeping_its_slot() {
    let lowered = lowered();
    let slide = &lowered.presentation().slides[0];
    let styled = element(slide, "pp-256.14");
    assert_matches(
        &styled["fill"],
        json!({
            "kind": "linear",
            "stops": [
                { "pos": 0, "color": { "kind": "theme", "slot": "accent2", "mods": [{ "op": "lumMod", "value": 1.1 }] } },
                { "pos": 1, "color": { "kind": "theme", "slot": "accent2", "mods": [{ "op": "shade", "value": 0.78 }] } },
            ],
        }),
    );
    assert_matches(
        &styled["stroke"],
        json!({ "width": 1, "fill": { "kind": "solid", "color": { "kind": "theme", "slot": "accent1", "mods": [{ "op": "shade", "value": 0.5 }] } } }),
    );
    assert_eq!(
        styled["ext"]["pptx"]["styleRef"],
        json!({ "fillRef": 2, "lnRef": 2, "effectRef": 3, "fontRef": "minor" })
    );
    assert_matches(
        &styled["shadow"],
        json!({ "dy": 1.5, "color": { "kind": "rgb", "a": 0.63 } }),
    );
    assert!(lowered.codes("pp-256.14").is_empty());
    let none = json(&lowered.presentation().slides[1].elements[0]);
    assert_eq!(none["fill"], json!({ "kind": "none" }));
    assert_eq!(none["stroke"], json!(null));
}

#[test]
fn reads_a_background_from_the_theme_by_bg_ref() {
    let lowered = lowered();
    assert_matches(
        &json(&lowered.presentation().slides[1].background),
        json!({
            "kind": "linear",
            "stops": [
                { "color": { "slot": "accent6", "mods": [] } },
                { "color": { "slot": "accent6", "mods": [{ "op": "shade" }] } },
            ],
        }),
    );
}

#[test]
fn keeps_groups_with_their_child_space_and_fills_grp_fill_from_the_enclosing_group() {
    let lowered = lowered();
    let slide = &lowered.presentation().slides[0];
    let group = element(slide, "pp-256.40");
    assert_matches(
        &group["frame"],
        json!({ "x": 100, "y": 200, "w": 200, "h": 100, "rotation": 20 }),
    );
    assert_eq!(
        group["childSpace"],
        json!({ "x": 0, "y": 0, "w": 400, "h": 200 })
    );
    assert_eq!(
        element(slide, "pp-256.41")["fill"],
        json!({ "kind": "solid", "color": { "kind": "rgb", "space": "srgb", "r": 0, "g": 1, "b": 0, "a": 1 } })
    );
    assert_eq!(element(slide, "pp-256.43")["fill"]["kind"], "solid");
    assert_eq!(
        element(slide, "pp-256.43")["origin"]["path"],
        "/p:sld/p:cSld/p:spTree/p:grpSp[1]/p:grpSp[1]/p:sp[1]"
    );
}

#[test]
fn keeps_a_connector_glued_to_its_shapes_for_a_later_export() {
    let lowered = lowered();
    assert_eq!(
        element(&lowered.presentation().slides[0], "pp-256.30")["ext"],
        json!({ "pptx": { "connection": { "start": { "id": "10", "idx": 3 }, "end": { "id": "11", "idx": 1 } } } })
    );
}

#[test]
fn lowers_charts_ole_objects_and_a_table_without_a_grid_as_foreign_nodes() {
    let lowered = lowered();
    let slide = &lowered.presentation().slides[0];
    assert_matches(
        &element(slide, "pp-256.50"),
        json!({ "kind": "foreign", "type": "table", "dialect": "pptx" }),
    );
    assert_matches(
        &element(slide, "pp-256.51"),
        json!({ "kind": "foreign", "type": "chart" }),
    );
    let ole = element(slide, "pp-256.52");
    assert_matches(&ole, json!({ "kind": "foreign", "type": "ole" }));
    assert_eq!(ole["fallback"]["media"].as_str().unwrap().len(), 64);
}

#[test]
fn reads_the_understood_choice_of_alternate_content_else_its_fallback() {
    let lowered = lowered();
    let ids: Vec<&str> = lowered.presentation().slides[0]
        .elements
        .iter()
        .map(|element| element.id().as_str())
        .collect();
    assert!(ids.contains(&"pp-256.60") && !ids.contains(&"pp-256.61"));
    assert!(ids.contains(&"pp-256.63") && !ids.contains(&"pp-256.62"));
    assert_eq!(
        json(&lowered.find("PX2005").unwrap().data),
        json!({ "requires": "cx1" })
    );
}

#[test]
fn records_picture_fills_as_media_typed_by_their_bytes_and_declines_linked_pictures() {
    let lowered = lowered();
    let presentation = lowered.presentation();
    let linked = element(&presentation.slides[0], "pp-256.70");
    assert_matches(
        &linked,
        json!({ "alt": "A described box", "hidden": true, "link": { "kind": "url", "url": "https://example.com/a?b=1" } }),
    );
    let sha = linked["fill"]["media"].as_str().unwrap();
    let media = json(&presentation.media);
    assert_matches(
        &media[sha],
        json!({ "type": "image/png", "width": 4, "height": 2 }),
    );
    let mut codes = lowered.codes("pp-256.70.2");
    codes.sort();
    assert_eq!(codes, ["PX2004", "PX2104", "PX2301"]);
    assert_matches(
        &json(lowered.find("PX2301").unwrap()),
        json!({ "severity": "decline", "feature": "fill.image" }),
    );
}

#[test]
fn declines_objects_it_does_not_read() {
    assert_matches(
        &json(lowered().find("PX2009").unwrap()),
        json!({ "severity": "decline", "container": "pp-256" }),
    );
}

#[test]
fn compiles_with_the_core_naming_every_foreign_node() {
    let compiled = compile_bytes(&shapes(), "presentation.pptx");
    let foreign: Vec<(String, String)> = compiled
        .diagnostics
        .iter()
        .filter(|d| matches!(d.code.code(), "IR0910" | "IR0911"))
        .map(|d| {
            (
                d.code.code().to_owned(),
                d.node.as_ref().unwrap().as_str().to_owned(),
            )
        })
        .collect();
    let expected: Vec<(String, String)> = [
        ("IR0910", "pp-256.16"),
        ("IR0910", "pp-256.50"),
        ("IR0910", "pp-256.51"),
        ("IR0911", "pp-256.52"),
    ]
    .iter()
    .map(|(code, node)| ((*code).to_owned(), (*node).to_owned()))
    .collect();
    assert_eq!(foreign, expected);
    assert!(compiled.errors().is_empty(), "{:?}", compiled.errors());
}

fn flipped_group(flips: &str) -> String {
    format!(
        "<p:grpSp><p:nvGrpSpPr><p:cNvPr id=\"5\" name=\"Group\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr>{}</p:grpSpPr>{}</p:grpSp>",
        xfrm(100.0, 100.0, 200.0, 100.0, flips),
        sp(Sp {
            id: 6,
            sp_pr: xfrm(100.0, 100.0, 100.0, 50.0, ""),
            tx_body: Some(body(&["Readable"], "<a:bodyPr/>")),
            ..Sp::default()
        })
    )
}

#[test]
fn keeps_the_text_of_a_flipped_group_unmirrored() {
    // Undone outright, and undone into a half turn (a vertical flip turns text upside down, as for a single shape).
    for (flips, shape_scale, text_scale) in [
        (" flipH=\"1\"", "scale:-1 1", "scale:-1 1"),
        (" flipV=\"1\"", "scale:1 -1", "scale:-1 1"),
    ] {
        let compiled = compile_bytes(
            &pptx(PptxSpec {
                slides: vec![slide(flipped_group(flips))],
                ..PptxSpec::default()
            }),
            "presentation.pptx",
        );
        let html = compiled.text("slides/pp-256.html");
        let tag = |marker: &str| {
            let at = html
                .find(marker)
                .unwrap_or_else(|| panic!("{marker} in {html}"));
            let end = html[at..].find('>').unwrap();
            &html[at..at + end]
        };
        assert!(
            !tag("<div data-el=\"pp-256.5\"").contains("scale:"),
            "{flips}"
        );
        assert!(
            tag("<div data-el=\"pp-256.6\"").contains(shape_scale),
            "{flips}"
        );
        assert!(tag("<div data-sd-text").contains(text_scale), "{flips}");
    }
}

fn nested(depth: usize) -> String {
    if depth == 0 {
        return sp(Sp {
            id: 900,
            sp_pr: format!(
                "{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>",
                xfrm(0.0, 0.0, 10.0, 10.0, "")
            ),
            ..Sp::default()
        });
    }
    format!(
        "<p:grpSp><p:nvGrpSpPr><p:cNvPr id=\"{depth}\" name=\"g\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{}</p:grpSp>",
        nested(depth - 1)
    )
}

fn failed_codes(lowered: &support::Lowered) -> Vec<Option<String>> {
    lowered
        .presentation()
        .slides
        .iter()
        .map(|slide| {
            slide
                .failed
                .as_ref()
                .map(|failed| failed.code.as_str().to_owned())
        })
        .collect()
}

#[test]
fn fails_only_the_container_whose_groups_nest_past_the_limit() {
    let limits = Limits::default();
    let lowered = lower(&pptx(PptxSpec {
        slides: vec![slide(nested(limits.group_depth + 1)), slide(nested(2))],
        ..PptxSpec::default()
    }));
    assert_eq!(failed_codes(&lowered), [Some("PX2901".to_owned()), None]);
    assert!(lowered.presentation().slides[0].elements.is_empty());
    assert_eq!(
        lowered
            .find("PX2901")
            .unwrap()
            .container
            .as_ref()
            .unwrap()
            .as_str(),
        "pp-256"
    );
}

#[test]
fn fails_a_container_with_more_elements_than_the_limit() {
    let limits = Limits {
        elements_per_container: 3,
        ..Limits::default()
    };
    let bytes = pptx(PptxSpec {
        slides: vec![slide(nested(0).repeat(4))],
        ..PptxSpec::default()
    });
    let lowered = lower_with(&bytes, None, &limits).unwrap();
    assert_eq!(failed_codes(&lowered), [Some("PX2901".to_owned())]);
}

#[test]
fn fails_only_the_container_whose_custom_path_passes_the_path_segment_limit() {
    let path = |segments: usize| {
        let lines: String = (0..segments - 1)
            .map(|i| format!("<a:lnTo><a:pt x=\"{}\" y=\"50\"/></a:lnTo>", i * 10))
            .collect();
        sp(Sp {
            id: 2,
            sp_pr: format!(
                "{}<a:custGeom><a:pathLst><a:path w=\"100\" h=\"100\"><a:moveTo><a:pt x=\"0\" y=\"0\"/></a:moveTo>{lines}</a:path></a:pathLst></a:custGeom>",
                xfrm(0.0, 0.0, 100.0, 100.0, "")
            ),
            ..Sp::default()
        })
    };
    let limits = Limits {
        path_segments: 3,
        ..Limits::default()
    };
    let bytes = pptx(PptxSpec {
        slides: vec![slide(path(4)), slide(path(3))],
        ..PptxSpec::default()
    });
    let lowered = lower_with(&bytes, None, &limits).unwrap();
    assert_eq!(failed_codes(&lowered), [Some("PX2901".to_owned()), None]);
}

/// A custom geometry whose first point's x is the guide `x = <formula>`, in a 100 pt square at `y`.
fn guided_path(formula: &str) -> String {
    format!(
        "<a:custGeom><a:gdLst><a:gd name=\"x\" fmla=\"{formula}\"/></a:gdLst><a:pathLst><a:path w=\"100\" h=\"100\"><a:moveTo><a:pt x=\"x\" y=\"0\"/></a:moveTo><a:lnTo><a:pt x=\"100\" y=\"x\"/></a:lnTo></a:path></a:pathLst></a:custGeom>"
    )
}

fn guided_shape(id: u64, formula: &str, y: f64) -> String {
    sp(Sp {
        id,
        sp_pr: format!("{}{}", xfrm(0.0, y, 100.0, 100.0, ""), guided_path(formula)),
        ..Sp::default()
    })
}

/// `sqrt` of a negative guide and `0 * Infinity` are NaN; an overflow alone is Infinity, which clamps.
const SQRT_NEGATIVE: &str = "sqrt -4";

/// A guide whose literal overflows to Infinity (`1` and 400 zeros).
fn overflow() -> String {
    format!("+- 1{} 0 0", "0".repeat(400))
}

fn not_a_number_deck() -> Vec<u8> {
    let overflow_times_zero = format!(
        "<a:gd name=\"big\" fmla=\"{}\"/><a:gd name=\"x\" fmla=\"*/ big 0 1\"/>",
        overflow()
    );
    let group = format!(
        "<p:grpSp><p:nvGrpSpPr><p:cNvPr id=\"3\" name=\"Group\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr>{}</p:grpSpPr>{}</p:grpSp>",
        xfrm(0.0, 0.0, 100.0, 100.0, ""),
        guided_shape(4, SQRT_NEGATIVE, 0.0).replace(
            &format!("<a:gd name=\"x\" fmla=\"{SQRT_NEGATIVE}\"/>"),
            &overflow_times_zero
        )
    );
    let picture = format!(
        "<p:pic><p:nvPicPr><p:cNvPr id=\"5\" name=\"Picture\"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed=\"rIdImg\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{}{}</p:spPr></p:pic>",
        xfrm(0.0, 0.0, 100.0, 100.0, ""),
        guided_path(SQRT_NEGATIVE)
    );
    let image = || vec![support::rel("rIdImg", "image", "../media/image1.png")];
    pptx(PptxSpec {
        slides: vec![
            slide(guided_shape(2, SQRT_NEGATIVE, 0.0)),
            slide(group),
            support::SlideSpec {
                rels: image(),
                ..slide(picture)
            },
            slide(guided_shape(6, &overflow(), 0.0)),
        ],
        parts: vec![(
            "ppt/media/image1.png".to_owned(),
            support::fixtures::png(4, 2),
        )],
        ..PptxSpec::default()
    })
}

#[test]
fn hands_a_custom_path_coordinate_that_is_not_a_number_to_the_verifier_as_null() {
    let lowered = support::lower(&not_a_number_deck());
    let slidock_core_api::Unverified::Json(presentation) = &lowered.lowered.presentation else {
        panic!("a NaN coordinate has no typed IR value");
    };
    let slides = &presentation["slides"];
    let first = |geometry: &serde_json::Value| json(&geometry["paths"][0]["segments"]);
    assert_eq!(
        first(&slides[0]["elements"][0]["geometry"]),
        json!([["M", null, 0], ["L", 100, null]])
    );
    assert_eq!(
        first(&slides[1]["elements"][0]["children"][0]["geometry"]),
        json!([["M", null, 0], ["L", 100, null]])
    );
    assert_eq!(
        first(&slides[2]["elements"][0]["clip"]),
        json!([["M", null, 0], ["L", 100, null]])
    );
    // Infinity alone clamps to the path range.
    assert_eq!(
        first(&slides[3]["elements"][0]["geometry"]),
        json!([["M", 1_000_000_000, 0], ["L", 100, 1_000_000_000]])
    );
}

#[test]
fn a_custom_path_coordinate_that_is_not_a_number_fails_its_container_with_ir0001() {
    let compiled = compile_bytes(&not_a_number_deck(), "presentation.pptx");
    let slides = json(compiled.presentation.as_ref().unwrap())["slides"].clone();
    let failed: Vec<&serde_json::Value> = (0..4).map(|i| &slides[i]["failed"]).collect();
    assert_eq!(
        failed,
        [
            &json!({ "code": "IR0001" }),
            &json!({ "code": "IR0001" }),
            &json!({ "code": "IR0001" }),
            &serde_json::Value::Null
        ]
    );
    // Failed as the verifier fails a container: it keeps its layout and loses its elements.
    assert_matches(
        &slides[0],
        json!({ "layout": "pp-2147483649", "elements": [] }),
    );
    let sites: Vec<(&str, Option<&str>)> = compiled
        .errors()
        .iter()
        .map(|d| (d.code.code(), d.node.as_ref().map(|node| node.as_str())))
        .collect();
    assert_eq!(
        sites,
        [
            ("IR0001", Some("pp-256.2")),
            ("IR0001", Some("pp-257.4")),
            ("IR0001", Some("pp-258.5")),
        ]
    );
    assert!(
        compiled.errors()[2]
            .message
            .starts_with("Schema violation at slides[2].elements[0].clip.paths[0].segments[0][1]:"),
        "{}",
        compiled.errors()[2].message
    );
}

#[test]
fn a_custom_path_point_or_arc_missing_a_required_attribute_is_unknown_not_read_as_0() {
    let custom = |commands: &str| {
        format!(
            "<a:custGeom><a:pathLst><a:path w=\"100\" h=\"100\">{commands}</a:path></a:pathLst></a:custGeom>"
        )
    };
    let shape = |id: u64, commands: &str| {
        sp(Sp {
            id,
            sp_pr: format!("{}{}", xfrm(0.0, 0.0, 100.0, 100.0, ""), custom(commands)),
            ..Sp::default()
        })
    };
    const START: &str = "<a:moveTo><a:pt x=\"0\" y=\"0\"/></a:moveTo>";
    let whole = format!(
        "{START}<a:lnTo><a:pt x=\"100\" y=\"0\"/></a:lnTo><a:arcTo wR=\"50\" hR=\"50\" stAng=\"0\" swAng=\"5400000\"/><a:close/>"
    );
    let cases = [
        (2, format!("{START}<a:lnTo><a:pt x=\"100\"/></a:lnTo>"), "y"),
        (
            3,
            "<a:moveTo/><a:lnTo><a:pt x=\"100\" y=\"0\"/></a:lnTo>".to_owned(),
            "x",
        ),
        (
            4,
            format!("{START}<a:arcTo wR=\"50\" hR=\"50\" stAng=\"0\"/>"),
            "swAng",
        ),
    ];
    let mut slides: Vec<_> = cases
        .iter()
        .map(|(id, commands, _)| slide(shape(*id, commands)))
        .collect();
    slides.push(slide(shape(5, &whole)));
    let picture = format!(
        "<p:pic><p:nvPicPr><p:cNvPr id=\"6\" name=\"Picture\"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed=\"rIdImg\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{}{}</p:spPr></p:pic>",
        xfrm(0.0, 0.0, 100.0, 100.0, ""),
        custom(&format!("{START}<a:lnTo><a:pt y=\"100\"/></a:lnTo>"))
    );
    slides.push(support::SlideSpec {
        rels: vec![support::rel("rIdImg", "image", "../media/image1.png")],
        ..slide(picture)
    });
    let bytes = pptx(PptxSpec {
        slides,
        parts: vec![(
            "ppt/media/image1.png".to_owned(),
            support::fixtures::png(4, 2),
        )],
        ..PptxSpec::default()
    });
    let lowered = lower(&bytes);
    let presentation = lowered.presentation();
    for (index, (id, _, missing)) in cases.iter().enumerate() {
        assert_matches(
            &element(
                &presentation.slides[index],
                &format!("pp-{}.{id}", 256 + index),
            ),
            json!({
                "kind": "foreign",
                "type": "unknown",
                "payload": { "geometry": format!("a path point or arc has no {missing}") }
            }),
        );
    }
    // Every required attribute present: the custom geometry is read.
    let read = element(&presentation.slides[3], "pp-259.5");
    assert_eq!(read["geometry"]["kind"], "custom");
    assert_eq!(
        read["geometry"]["paths"][0]["segments"][1],
        json!(["L", 100, 0])
    );
    // A picture's clip missing one is not read: the picture is drawn whole and PX2308 says why.
    let photo = element(&presentation.slides[4], "pp-260.6");
    assert_eq!(photo["kind"], "picture");
    assert!(
        photo.get("clip").is_none_or(serde_json::Value::is_null),
        "{photo}"
    );
    assert_eq!(lowered.codes("pp-260.6"), ["PX2308"]);
}
