//! The passes on presentations built in Rust (the cases that need no fixture file): frames, placeholder binding, theme
//! references, geometry and paint, media plans and budgets, list labels, font binding, titles and reading order, and
//! the pass list itself. The cases that read the private IR fixtures are in `fixtures.rs`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::{Value, json};
use slidock_core::{PassManager, Pipeline};
use slidock_fonts::{CatalogFace, FamilyKey, FontCatalog, builtin_catalog};
use slidock_ir::diagnostics::Diagnostic;
use slidock_ir::fonts::{FaceStyle, FontMetrics};
use slidock_ir::frame::Frame;
use slidock_ir::paint::Fill;
use slidock_ir::source::{Element, Presentation};
use slidock_ir::text::AutoNumScheme;
use slidock_passes::{bind_family, lower_fill, number_label};
use support::*;

fn elements_of(presentation: &Presentation) -> Value {
    to_json(&presentation.slides[0].elements)
}

fn codes(diagnostics: &[Diagnostic]) -> Vec<String> {
    diagnostics
        .iter()
        .map(|d| d.code.code().to_owned())
        .collect()
}

mod pass_list {
    use super::*;

    #[test]
    fn is_the_v1_order_and_a_complete_pipeline() {
        let names: Vec<String> = slidock_passes::passes()
            .iter()
            .map(|pass| pass.name().to_owned())
            .collect();
        assert_eq!(
            names,
            [
                "normalize-frames",
                "bind-placeholders",
                "resolve-inheritance",
                "resolve-theme",
                "cascade-text",
                "number-lists",
                "bind-fonts",
                "text-metrics",
                "lower-geometry",
                "plan-media",
                "normalize-timeline",
                "a11y",
            ]
        );
        assert!(PassManager::new(slidock_passes::passes()).is_ok());
        let pipeline = Pipeline::new(
            slidock_passes::before_fonts(),
            slidock_passes::after_fonts(),
        )
        .unwrap();
        assert_eq!(pipeline.font_boundary(), slidock_core::BEFORE_FONTS.len());
        assert_eq!(
            slidock_passes::before_fonts().len(),
            slidock_core::BEFORE_FONTS.len()
        );
        for (pass, name) in slidock_passes::before_fonts()
            .iter()
            .zip(slidock_core::BEFORE_FONTS)
        {
            assert_eq!(pass.name(), name);
        }
        for (pass, name) in slidock_passes::after_fonts()
            .iter()
            .zip(slidock_core::AFTER_FONTS)
        {
            assert_eq!(pass.name(), name);
        }
    }

    #[test]
    fn registers_as_the_standard_pipeline() {
        let mut registry = slidock_core::PassRegistry::new();
        for pass in slidock_passes::passes() {
            registry.register(pass).unwrap();
        }
        let pipeline = Pipeline::standard(registry).unwrap();
        assert_eq!(pipeline.manager().passes().len(), 12);
        assert_eq!(pipeline.font_boundary(), 6);
    }
}

mod normalize_frames {
    use super::*;

    fn normalized(elements: Vec<Value>) -> Value {
        let out = apply_pass(&pass_named("normalize-frames"), &with_elements(elements));
        elements_of(&out.presentation)
    }

    fn group(id: &str, at: Value, extra: Value, children: Vec<Value>) -> Value {
        with(
            json!({ "id": id, "kind": "group", "origin": agent(), "frame": at, "children": children }),
            extra,
        )
    }

    #[test]
    fn maps_a_child_space_into_the_group_box_and_drops_it() {
        let space = json!({ "childSpace": { "x": 1000, "y": 1000, "w": 2000, "h": 1000 } });
        let turned = with(
            frame(2000.0, 1500.0, 1000.0, 500.0),
            json!({ "rotation": 45 }),
        );
        let out = normalized(vec![group(
            "ir-g",
            frame(60.0, 160.0, 300.0, 150.0),
            space,
            vec![
                rect("ir-g.a", frame(1000.0, 1000.0, 1000.0, 500.0)),
                rect("ir-g.b", turned),
            ],
        )]);
        assert!(out[0].get("childSpace").is_none());
        assert_eq!(out[0]["frame"], frame(60.0, 160.0, 300.0, 150.0));
        assert_eq!(out[0]["children"][0]["frame"], frame(0.0, 0.0, 150.0, 75.0));
        assert_eq!(
            out[0]["children"][1]["frame"],
            with(frame(150.0, 75.0, 150.0, 75.0), json!({ "rotation": 45 }))
        );
    }

    #[test]
    fn scales_a_tables_grid_with_its_frame_so_its_tracks_still_add_up_to_it() {
        let cell = json!({ "text": { "body": {}, "paragraphs": [] } });
        let table = json!({
            "id": "ir-g.table", "kind": "table", "origin": agent(), "frame": frame(10.0, 10.0, 180.0, 60.0),
            "columns": [{ "width": 33.333 }, { "width": 100 }, { "width": 46.667 }],
            "rows": [
                { "height": 20.001, "cells": [cell, cell, cell] },
                { "height": 39.999, "cells": [cell, cell, cell] },
            ],
        });
        let space = json!({ "childSpace": { "x": 0, "y": 0, "w": 200, "h": 80 } });
        let input = with_elements(vec![group(
            "ir-g",
            frame(0.0, 0.0, 300.0, 120.0),
            space,
            vec![table],
        )]);
        let out = apply_pass(&pass_named("normalize-frames"), &input).presentation;
        let scaled = &elements_of(&out)[0]["children"][0];
        assert_eq!(scaled["frame"], frame(15.0, 15.0, 270.0, 90.0));
        // Each track spans two scaled grid lines: 49.9995 and 199.9995 land on 50 and 200.
        assert_eq!(
            scaled["columns"],
            json!([{ "width": 50 }, { "width": 150 }, { "width": 70 }])
        );
        assert_eq!(
            scaled["rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["height"].clone())
                .collect::<Vec<_>>(),
            [json!(30.002), json!(59.998)]
        );
        let verified = slidock_core::verify_source_presentation(&out.into(), None);
        assert!(
            verified.diagnostics.is_empty(),
            "{:?}",
            verified.diagnostics
        );
    }

    #[test]
    fn composes_nested_child_spaces_each_relative_to_its_own_group_box() {
        let inner = group(
            "ir-g.inner",
            frame(1500.0, 1500.0, 500.0, 500.0),
            json!({ "childSpace": { "x": 0, "y": 0, "w": 100, "h": 100 } }),
            vec![rect("ir-g.inner.dot", frame(50.0, 50.0, 50.0, 25.0))],
        );
        let outer = group(
            "ir-g",
            frame(60.0, 160.0, 300.0, 150.0),
            json!({ "childSpace": { "x": 1000, "y": 1000, "w": 2000, "h": 1000 } }),
            vec![inner],
        );
        let out = normalized(vec![outer]);
        let mapped = &out[0]["children"][0];
        // 500 child units of a 2000-wide space in a 300 pt box: 75 pt; the inner space then maps 100 units onto it.
        assert_eq!(mapped["frame"], frame(75.0, 75.0, 75.0, 75.0));
        assert_eq!(
            mapped["children"][0]["frame"],
            frame(37.5, 37.5, 37.5, 18.75)
        );
    }

    #[test]
    fn leaves_an_empty_child_space_axis_unscaled_and_quantises_the_result() {
        let out = normalized(vec![group(
            "ir-g",
            frame(0.0, 0.0, 100.0, 100.0),
            json!({ "childSpace": { "x": 0, "y": 0, "w": 0, "h": 300 } }),
            vec![rect("ir-g.a", frame(10.0, 100.0, 20.0, 100.0))],
        )]);
        assert_eq!(
            out[0]["children"][0]["frame"],
            frame(10.0, 33.333, 20.0, 33.333)
        );
    }

    #[test]
    fn moves_a_groups_flips_down_to_its_children_mirrored_in_the_groups_box() {
        let inner = group(
            "ir-g.inner",
            frame(0.0, 50.0, 40.0, 20.0),
            json!({}),
            vec![rect("ir-g.inner.dot", frame(5.0, 0.0, 10.0, 20.0))],
        );
        let children = vec![
            rect(
                "ir-g.a",
                with(frame(10.0, 20.0, 30.0, 10.0), json!({ "rotation": 45 })),
            ),
            inner,
        ];
        let flipped = with(
            frame(60.0, 160.0, 100.0, 80.0),
            json!({ "rotation": 30, "flipH": true }),
        );
        let out = normalized(vec![group("ir-g", flipped, json!({}), children.clone())]);
        assert_eq!(
            out[0]["frame"],
            with(frame(60.0, 160.0, 100.0, 80.0), json!({ "rotation": 30 }))
        );
        // Mirrored across the group's width, turning the other way, and flipped itself.
        assert_eq!(
            out[0]["children"][0]["frame"],
            with(
                frame(60.0, 20.0, 30.0, 10.0),
                json!({ "rotation": 315, "flipH": true })
            )
        );
        // A child group passes the flip on in turn.
        let nested = &out[0]["children"][1];
        assert_eq!(nested["frame"], frame(60.0, 50.0, 40.0, 20.0));
        assert_eq!(
            nested["children"][0]["frame"],
            with(frame(25.0, 0.0, 10.0, 20.0), json!({ "flipH": true }))
        );

        let both = with(
            frame(60.0, 160.0, 100.0, 80.0),
            json!({ "rotation": 30, "flipH": true, "flipV": true }),
        );
        let turned = normalized(vec![group("ir-g", both, json!({}), children)]);
        // Both flips are a half turn: the rotation stays, the box is mirrored on both axes.
        assert_eq!(
            turned[0]["children"][0]["frame"],
            with(
                frame(60.0, 50.0, 30.0, 10.0),
                json!({ "rotation": 45, "flipH": true, "flipV": true })
            )
        );
    }

    #[test]
    fn keeps_frames_without_a_child_space_and_everything_else_on_the_element() {
        let arrow = with(
            rect(
                "ir-s1.arrow",
                with(
                    frame(400.0, 300.0, 200.0, 60.0),
                    json!({ "rotation": 30, "flipH": true }),
                ),
            ),
            json!({
                "name": "Arrow 7", "ext": { "pptx": { "connection": { "start": "4" } } },
                "geometry": { "kind": "preset", "name": "rightArrow", "adjust": { "adj1": 50000 } },
            }),
        );
        let out = normalized(vec![arrow.clone()]);
        assert_eq!(out[0], arrow);
    }
}

mod lower_geometry {
    use super::*;

    fn box_frame(w: f64, h: f64, extra: Value) -> Frame {
        serde_json::from_value(with(frame(0.0, 0.0, w, h), extra)).unwrap()
    }

    fn linear(angle: f64, scaled: bool, rotate_with_shape: bool) -> Fill {
        serde_json::from_value(json!({
            "kind": "linear", "angle": angle, "scaled": scaled, "rotateWithShape": rotate_with_shape,
            "stops": [{ "pos": 0, "color": black() }]
        }))
        .unwrap()
    }

    /// A lowered linear gradient's endpoints.
    fn ends(fill: &Fill) -> Value {
        let value = to_json(fill);
        json!([value["from"], value["to"]])
    }

    fn lowered(fill: &Fill, frame: &Frame) -> Fill {
        lower_fill(fill, frame, None).unwrap().fill
    }

    fn lower(elements: Vec<Value>) -> Value {
        let out = apply_pass(&pass_named("lower-geometry"), &with_elements(elements));
        elements_of(&out.presentation)
    }

    #[test]
    fn turns_linear_angles_into_frame_local_endpoints_scaled_or_in_the_real_proportions() {
        let wide = box_frame(200.0, 100.0, json!({}));
        let point = |x: f64, y: f64| to_json(&json!({ "x": x, "y": y }));
        assert_eq!(
            ends(&lowered(&linear(0.0, false, true), &wide)),
            json!([point(0.0, 0.5), point(1.0, 0.5)])
        );
        assert_eq!(
            ends(&lowered(&linear(90.0, true, true), &wide)),
            json!([point(0.5, 0.0), point(0.5, 1.0)])
        );
        assert_eq!(
            ends(&lowered(&linear(45.0, true, true), &wide)),
            json!([point(0.0, 0.0), point(1.0, 1.0)])
        );
        // Unscaled 45° in a 2:1 box: isochromes at -45° in the page, so the line runs along (2, 1) in box
        // fractions.
        assert_eq!(
            ends(&lowered(&linear(45.0, false, true), &wide)),
            json!([point(-0.1, 0.2), point(1.1, 0.8)])
        );
    }

    #[test]
    fn turns_a_gradient_that_does_not_rotate_with_its_shape_back_by_the_rotation_and_flips() {
        let square = |extra: Value| box_frame(100.0, 100.0, extra);
        let fixed = |extra: Value| ends(&lowered(&linear(90.0, true, false), &square(extra)));
        let free = |angle: f64| ends(&lowered(&linear(angle, true, true), &square(json!({}))));
        assert_eq!(fixed(json!({ "rotation": 90 })), free(0.0));
        assert_eq!(fixed(json!({ "flipV": true })), free(270.0));
        assert_eq!(
            ends(&lowered(
                &linear(0.0, true, false),
                &square(json!({ "flipH": true }))
            )),
            free(180.0)
        );
    }

    #[test]
    fn gives_a_radial_gradient_the_radius_to_its_farthest_corner_and_keeps_other_fills() {
        let radial: Fill = serde_json::from_value(json!({
            "kind": "radial", "center": { "x": 0.25, "y": 0.25 }, "stops": [{ "pos": 0, "color": black() }]
        }))
        .unwrap();
        let out = to_json(&lowered(&radial, &box_frame(10.0, 10.0, json!({}))));
        assert_eq!(out["radius"], json!(1.06066));
        assert_eq!(out["center"], json!({ "x": 0.25, "y": 0.25 }));
        let solid: Fill =
            serde_json::from_value(json!({ "kind": "solid", "color": rgb(1.0, 1.0, 1.0) }))
                .unwrap();
        assert_eq!(lowered(&solid, &box_frame(10.0, 10.0, json!({}))), solid);
    }

    #[test]
    fn lowers_a_missing_geometry_as_a_rectangle() {
        let bare = rect("ir-s1.bare", frame(0.0, 0.0, 150.0, 80.0));
        let out = lower(vec![bare]);
        assert_eq!(
            out[0]["geometry"],
            json!({ "kind": "paths", "preset": "rect", "paths": [{
                "segments": [["M", 0, 0], ["L", 150, 0], ["L", 150, 80], ["L", 0, 80], ["Z"]],
                "fill": true, "stroke": true, "fillRule": "nonzero"
            }] })
        );
    }

    #[test]
    fn gives_closed_paths_no_line_ends() {
        let closed = with(
            rect("ir-x", frame(0.0, 0.0, 100.0, 50.0)),
            json!({
                "geometry": { "kind": "preset", "name": "rect", "adjust": {} },
                "stroke": { "width": 1, "fill": { "kind": "solid", "color": black() }, "dash": "solid",
                            "cap": "flat", "join": "miter", "tail": { "type": "triangle", "w": "med", "len": "med" } }
            }),
        );
        let out = lower(vec![closed]);
        let paths = out[0]["geometry"]["paths"].as_array().unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].get("role").is_none());
    }

    #[test]
    fn adds_line_ends_to_an_open_stroked_path_and_none_without_a_stroke() {
        let line = |stroke: Value| {
            with(
                rect("ir-line", frame(0.0, 0.0, 240.0, 0.0)),
                json!({ "geometry": { "kind": "preset", "name": "line", "adjust": {} }, "stroke": stroke }),
            )
        };
        let solid = json!({ "width": 2, "fill": { "kind": "solid", "color": black() }, "dash": "solid",
            "cap": "flat", "join": "miter", "tail": { "type": "triangle", "w": "med", "len": "med" } });
        let out = lower(vec![line(solid.clone())]);
        let paths = out[0]["geometry"]["paths"].as_array().unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[1]["role"], "line-end");
        assert_eq!(paths[1]["fillRule"], "nonzero");
        // The line stops where the arrowhead begins.
        assert_eq!(paths[0]["segments"], json!([["M", 0, 0], ["L", 234, 0]]));
        let unpainted = with(solid, json!({ "fill": { "kind": "none" } }));
        let out = lower(vec![line(unpainted)]);
        assert_eq!(out[0]["geometry"]["paths"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn lowers_a_table_cells_fill_free_of_the_height_its_rows_grow_to_and_orients_its_text() {
        let cell = |fill: Fill, extra: Value| {
            with(
                json!({ "fill": to_json(&fill), "text": { "body": {}, "paragraphs": [] } }),
                extra,
            )
        };
        let table = json!({
            "id": "ir-t", "kind": "table", "origin": agent(),
            "frame": with(frame(0.0, 0.0, 400.0, 100.0), json!({ "rotation": 30, "flipV": true })),
            "columns": [{ "width": 100 }, { "width": 300 }],
            "rows": [
                { "height": 0, "cells": [cell(linear(45.0, false, true), json!({ "colSpan": 2 })), null] },
                { "height": 100, "cells": [cell(linear(45.0, true, true), json!({})), cell(linear(45.0, false, false), json!({}))] },
            ],
        });
        let out = lower(vec![table]);
        let rows = &out[0]["rows"];
        let stops = json!([{ "pos": 0, "color": black() }]);
        // Unscaled, the angle holds in the box as drawn, so it stays an angle, even in a row of no least height.
        assert_eq!(
            rows[0]["cells"][0]["fill"],
            json!({ "kind": "linear", "angle": 45, "stops": stops })
        );
        // Scaled, it holds in the unit box: endpoints in fractions, whatever the box.
        let unit = ends(&lowered(
            &linear(45.0, true, true),
            &box_frame(1.0, 1.0, json!({})),
        ));
        assert_eq!(
            json!([
                rows[1]["cells"][0]["fill"]["from"],
                rows[1]["cells"][0]["fill"]["to"]
            ]),
            unit
        );
        // Not rotating with the table, it is turned back by the frame's rotation, then its flip: -(45 - 30).
        assert_eq!(
            rows[1]["cells"][1]["fill"],
            json!({ "kind": "linear", "angle": 345, "stops": stops })
        );
        for cell in [
            &rows[0]["cells"][0],
            &rows[1]["cells"][0],
            &rows[1]["cells"][1],
        ] {
            assert_eq!(cell["text"]["rotation"], 180);
        }
    }

    #[test]
    fn turns_readable_text_under_a_vertical_flip_and_leaves_text_that_flips_with_its_frame_unturned()
     {
        // Each frame as Keynote stores its flips: H, V (H plus a half turn), both (a half turn), H then 30°, and a
        // vertical flip bit as DrawingML writes it.
        let frames = [
            json!({ "flipH": true }),
            json!({ "flipH": true, "rotation": 180 }),
            json!({ "rotation": 180 }),
            json!({ "flipH": true, "rotation": 330 }),
            json!({ "flipV": true }),
        ];
        let rotations = |flips_with_frame: bool| -> Vec<Value> {
            let text = json!({ "body": {}, "paragraphs": [], "flipsWithFrame": flips_with_frame });
            let shapes = frames
                .iter()
                .enumerate()
                .map(|(i, extra)| {
                    json!({
                        "id": format!("ir-s1.t{i}"), "kind": "shape", "origin": agent(),
                        "frame": with(frame(0.0, 0.0, 200.0, 100.0), extra.clone()), "text": text,
                    })
                })
                .collect();
            let table = json!({
                "id": "ir-s1.table", "kind": "table", "origin": agent(),
                "frame": with(frame(0.0, 0.0, 100.0, 100.0), json!({ "flipV": true })),
                "columns": [{ "width": 100 }],
                "rows": [{ "height": 100, "cells": [{ "text": text }] }],
            });
            let out = lower([shapes, vec![table]].concat());
            let elements = out.as_array().unwrap();
            let mut rotations: Vec<Value> = elements[..frames.len()]
                .iter()
                .map(|shape| {
                    let flag = shape["text"]["flipsWithFrame"].as_bool().unwrap_or(false);
                    assert_eq!(flag, flips_with_frame);
                    shape["text"]["rotation"].clone()
                })
                .collect();
            rotations
                .push(elements[frames.len()]["rows"][0]["cells"][0]["text"]["rotation"].clone());
            rotations
        };
        assert_eq!(
            rotations(false),
            [
                json!(0),
                json!(0),
                json!(0),
                json!(0),
                json!(180),
                json!(180)
            ]
        );
        assert_eq!(rotations(true), vec![json!(0); 6]);
    }

    #[test]
    fn lowers_a_picture_clip_like_a_geometry_and_the_background_in_the_slide_box() {
        let sha = slidock_ir::ids::Sha256::of(b"photo").to_hex();
        let photo = json!({
            "id": "ir-s1.photo", "kind": "picture", "origin": agent(), "frame": frame(36.0, 300.0, 120.0, 90.0),
            "media": sha, "clip": { "kind": "preset", "name": "ellipse", "adjust": {} }
        });
        let mut input = to_json(&with_elements(vec![photo]));
        input["slides"][0]["background"] = to_json(&linear(90.0, true, true));
        let out = apply_pass(&pass_named("lower-geometry"), &parse(input)).presentation;
        let clip = &elements_of(&out)[0]["clip"];
        assert_eq!(
            (&clip["kind"], &clip["preset"]),
            (&json!("paths"), &json!("ellipse"))
        );
        let background = to_json(&out.slides[0].background);
        assert_eq!(background["from"], json!({ "x": 0.5, "y": 0 }));
        assert_eq!(background["to"], json!({ "x": 0.5, "y": 1 }));
    }
}

/// Gradients lowered to functions of the box's fractions u.
mod gradients {
    use super::*;

    /// √½ on the ratio grid.
    fn root_half() -> f64 {
        (std::f64::consts::FRAC_1_SQRT_2 * 1e5).round() / 1e5
    }

    fn turned(w: f64, h: f64, rotation: f64, flip_h: bool, flip_v: bool) -> Frame {
        serde_json::from_value(
            json!({ "x": 0, "y": 0, "w": w, "h": h, "rotation": rotation,
                                       "flipH": flip_h, "flipV": flip_v }),
        )
        .unwrap()
    }

    fn lowered(fill: Value, frame: &Frame) -> Value {
        let fill: Fill = serde_json::from_value(fill).unwrap();
        to_json(&lower_fill(&fill, frame, None).unwrap().fill)
    }

    fn stops() -> Value {
        json!([{ "pos": 0, "color": black() }, { "pos": 1, "color": rgb(1.0, 1.0, 1.0) }])
    }

    fn num(value: &Value) -> f64 {
        value.as_f64().unwrap()
    }

    fn xy(value: &Value) -> (f64, f64) {
        (num(&value["x"]), num(&value["y"]))
    }

    /// A lowered linear gradient's t at u.
    fn t_linear(fill: &Value, (x, y): (f64, f64)) -> f64 {
        let (fx, fy) = xy(&fill["from"]);
        let (tx, ty) = xy(&fill["to"]);
        let (dx, dy) = (tx - fx, ty - fy);
        ((x - fx) * dx + (y - fy) * dy) / (dx * dx + dy * dy)
    }

    /// The frame fractions of a page point (relative to the frame's centre) under rotation θ (clockwise) and flips:
    /// u = ((W/2, H/2) + F·R(−θ)·p)/(W, H).
    fn u_of(w: f64, h: f64, theta: f64, flip: (bool, bool), (x, y): (f64, f64)) -> (f64, f64) {
        let (c, s) = (
            slidock_math::cos(theta.to_radians()),
            slidock_math::sin(theta.to_radians()),
        );
        let (bx, by) = (c * x + s * y, -s * x + c * y);
        let (bx, by) = (if flip.0 { -bx } else { bx }, if flip.1 { -by } else { by });
        ((w / 2.0 + bx) / w, (h / 2.0 + by) / h)
    }

    fn page_of(w: f64, h: f64, theta: f64, (u, v): (f64, f64)) -> (f64, f64) {
        let (c, s) = (
            slidock_math::cos(theta.to_radians()),
            slidock_math::sin(theta.to_radians()),
        );
        let (x, y) = (u * w - w / 2.0, v * h - h / 2.0);
        (c * x - s * y, s * x + c * y)
    }

    fn lower(elements: Vec<Value>) -> Value {
        let out = apply_pass(&pass_named("lower-geometry"), &with_elements(elements));
        elements_of(&out.presentation)
    }

    fn odf(style: &str, angle: f64, border: f64, cx: f64, cy: f64) -> Value {
        json!({ "kind": "odf", "style": style, "angle": angle, "border": border,
                "center": { "x": cx, "y": cy }, "stops": stops() })
    }

    #[test]
    fn spans_an_outset_angle_across_the_corners_of_the_grown_box() {
        let angle = |a: f64, extra: Value| {
            with(
                json!({ "kind": "linear", "angle": a, "scaled": false, "rotateWithShape": true, "stops": stops() }),
                extra,
            )
        };
        // Keynote's +1 pt a side: at 0° the line reaches 1 pt past each side of retro's 1012.364 pt box.
        let out = lowered(
            angle(0.0, json!({ "outset": 1 })),
            &turned(1012.364, 1062.004, 0.0, false, false),
        );
        assert_eq!(out["from"], json!({ "x": -0.00099, "y": 0.5 }));
        assert_eq!(out["to"], json!({ "x": 1.00099, "y": 0.5 }));
        // At 90° on a 600 × 20 bar: ±1/20 past the box.
        let out = lowered(
            angle(90.0, json!({ "outset": 1 })),
            &turned(600.0, 20.0, 0.0, false, false),
        );
        assert_eq!(out["from"], json!({ "x": 0.5, "y": -0.05 }));
        assert_eq!(out["to"], json!({ "x": 0.5, "y": 1.05 }));
        // A tile: the right half of the box.
        let tile = json!({ "tile": { "l": 0.5, "t": 0, "r": 0, "b": 0 } });
        let out = lowered(angle(0.0, tile), &turned(600.0, 200.0, 0.0, false, false));
        assert_eq!(out["from"], json!({ "x": 0.5, "y": 0.5 }));
        assert_eq!(out["to"], json!({ "x": 1, "y": 0.5 }));
    }

    fn path(shape: &str, focus: Value, tile: Value) -> Value {
        json!({ "kind": "path", "path": shape, "focus": focus, "tile": tile, "stops": stops() })
    }

    fn edges(l: f64, t: f64, r: f64, b: f64) -> Value {
        json!({ "l": l, "t": t, "r": r, "b": b })
    }

    #[test]
    fn lowers_a_circle_path_to_a_real_circle_with_bowens_focus() {
        let (w, h) = (600.0, 200.0);
        let out = lowered(
            path(
                "circle",
                edges(0.1, 0.3, 0.8, 0.2),
                edges(0.0, 0.0, 0.0, 0.0),
            ),
            &turned(w, h, 0.0, false, false),
        );
        assert_eq!(out["kind"], "radial");
        assert_eq!(out["center"], json!({ "x": 0.5, "y": 0.5 }));
        assert_eq!(out["focus"], json!({ "x": 0.11111, "y": 0.6 }));
        let r = (w * w + h * h).sqrt() / 2.0;
        assert!((num(&out["radius"]) * w - r).abs() < 0.01);
        assert!((num(&out["radiusY"]) * h - r).abs() < 0.01);
        // The corner preset: the focus point at the bottom-right corner, the tile twice the box.
        let out = lowered(
            path(
                "circle",
                edges(1.0, 1.0, 0.0, 0.0),
                edges(0.0, 0.0, -1.0, -1.0),
            ),
            &turned(w, h, 0.0, false, false),
        );
        assert_eq!(out["center"], json!({ "x": 1, "y": 1 }));
        assert!(out.get("focus").is_none(), "{out}");
        assert!((num(&out["radius"]) * w - (w * w + h * h).sqrt()).abs() < 0.01);
        // A centred circle on a square box is today's radial: no y radius, no focus.
        let out = lowered(
            path(
                "circle",
                edges(0.5, 0.5, 0.5, 0.5),
                edges(0.0, 0.0, 0.0, 0.0),
            ),
            &turned(300.0, 300.0, 0.0, false, false),
        );
        assert_eq!(
            out,
            json!({ "kind": "radial", "center": { "x": 0.5, "y": 0.5 }, "radius": root_half(),
                                "stops": stops() })
        );
    }

    #[test]
    fn lowers_a_rect_path_to_rectangle_contours_around_the_focus_rectangle() {
        let out = lowered(
            path("rect", edges(0.1, 0.3, 0.8, 0.2), edges(0.0, 0.0, 0.0, 0.0)),
            &turned(600.0, 200.0, 0.0, false, false),
        );
        assert_eq!(
            out,
            json!({ "kind": "rect", "origin": { "x": 0.5, "y": 0.5 }, "axisX": { "x": 0.5, "y": 0 },
                    "axisY": { "x": 0, "y": 0.5 }, "inner": { "l": -0.8, "t": -0.4, "r": -0.6, "b": 0.6 },
                    "stops": stops() })
        );
    }

    #[test]
    fn moves_a_focus_outside_the_circle_inside_it_and_says_so() {
        let shape = with(
            rect("ir-s1.focus", frame(0.0, 0.0, 600.0, 200.0)),
            json!({ "fill": path("circle", edges(0.5, -2.0, 0.5, 3.0), edges(0.0, 0.0, 0.0, 0.0)) }),
        );
        let applied = apply_pass(&pass_named("lower-geometry"), &with_elements(vec![shape]));
        let fill = &elements_of(&applied.presentation)[0]["fill"];
        let r = (600.0f64 * 600.0 + 200.0 * 200.0).sqrt() / 2.0 / 200.0;
        assert!(
            (num(&fill["focus"]["y"]) - (0.5 - 0.98 * r)).abs() < 1e-4,
            "{fill}"
        );
        assert_eq!(codes(&applied.diagnostics), ["PX2106"]);
        assert_eq!(
            to_json(&applied.diagnostics[0])["data"],
            json!({ "reason": "the focus lies outside the circle" })
        );
    }

    /// ODF's linear s over the page-aligned bounds B (centre c, size bw × bh): ((p − c)·d)/E + ½.
    fn odf_s(alpha: f64, (bw, bh): (f64, f64), (x, y): (f64, f64)) -> f64 {
        let (s, c) = (
            slidock_math::sin(alpha.to_radians()),
            slidock_math::cos(alpha.to_radians()),
        );
        let extent = bw * s.abs() + bh * c.abs();
        (x * s + y * c) / extent + 0.5
    }

    #[test]
    fn keeps_an_odf_linear_gradient_page_aligned_over_the_turned_bounds() {
        // L08: linear 30° on a 400 × 200 rectangle turned 30° (counter-clockwise in ODF): B is 446.41 × 373.21.
        let (w, h, theta) = (400.0, 200.0, 330.0);
        let out = lowered(
            odf("linear", 30.0, 0.0, 0.5, 0.5),
            &turned(w, h, theta, false, false),
        );
        let b = (446.410, 373.205);
        for p in [
            (-223.2, -186.6),
            (223.2, 186.6),
            (100.0, -50.0),
            (-30.0, 120.0),
        ] {
            let t = t_linear(&out, u_of(w, h, theta, (false, false), p));
            assert!((t - odf_s(30.0, b, p)).abs() < 1e-4, "{p:?}: {t}");
        }
        // X03: under a vertical flip the start colour stays at the top of the page.
        let out = lowered(
            odf("linear", 0.0, 0.0, 0.5, 0.5),
            &turned(w, h, 0.0, false, true),
        );
        assert_eq!(out["from"], json!({ "x": 0.5, "y": 1 }));
        assert_eq!(out["to"], json!({ "x": 0.5, "y": 0 }));
        // A border holds the start colour over its fraction of the ramp.
        let out = lowered(
            odf("linear", 0.0, 0.25, 0.5, 0.5),
            &turned(w, h, 0.0, false, false),
        );
        assert_eq!(out["stops"][0]["pos"], json!(0.25));
    }

    #[test]
    fn mirrors_axial_stops_with_the_start_colour_on_the_axis() {
        let out = lowered(
            odf("axial", 0.0, 0.2, 0.5, 0.5),
            &turned(400.0, 200.0, 0.0, false, false),
        );
        let at: Vec<f64> = out["stops"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| num(&s["pos"]))
            .collect();
        assert_eq!(at, [0.1, 0.5, 0.9]);
        assert_eq!(out["stops"][1]["color"], black());
        assert_eq!(out["stops"][0]["color"], rgb(1.0, 1.0, 1.0));
    }

    #[test]
    fn centres_an_odf_radial_gradient_on_the_bounds_and_maps_it_back_into_the_frame() {
        // R04: cx = cy = 25 % on a 400 × 200 rectangle at (100, 100) turned 30°: the page centre (188.39, 106.70),
        // R = 290.94 pt, the end colour at the centre.
        let (w, h, theta) = (400.0, 200.0, 330.0);
        let out = lowered(
            odf("radial", 0.0, 0.0, 0.25, 0.25),
            &turned(w, h, theta, false, false),
        );
        let (x, y) = page_of(w, h, theta, xy(&out["center"]));
        assert!(
            (x + 300.0 - 188.39).abs() < 0.02 && (y + 200.0 - 106.70).abs() < 0.02,
            "{x} {y}"
        );
        assert!((num(&out["radius"]) * w - 290.94).abs() < 0.01, "{out}");
        assert!((num(&out["radiusY"]) * h - 290.94).abs() < 0.01, "{out}");
        assert_eq!(out["stops"][0]["color"], rgb(1.0, 1.0, 1.0));
    }

    #[test]
    fn bounds_a_curved_outline_by_its_curve_not_its_control_points() {
        // C02: an ellipse 400 × 200 turned 30°: tight bounds 360.56 × 264.58, R = ½·hypot = 223.61.
        let shape = with(
            rect("ir-s1.c02", frame(100.0, 100.0, 400.0, 200.0)),
            json!({ "frame": with(frame(100.0, 100.0, 400.0, 200.0), json!({ "rotation": 330 })),
                    "geometry": { "kind": "preset", "name": "ellipse", "adjust": {} },
                    "fill": odf("radial", 0.0, 0.0, 0.5, 0.5) }),
        );
        let out = lower(vec![shape]);
        let fill = &out[0]["fill"];
        assert!(
            (num(&fill["radius"]) * 400.0 - 223.61).abs() < 0.2,
            "{fill}"
        );
    }

    #[test]
    fn lowers_ellipsoid_square_and_rectangular_to_contour_families() {
        let frame = turned(400.0, 200.0, 0.0, false, false);
        let out = lowered(odf("ellipsoid", 0.0, 0.0, 0.5, 0.5), &frame);
        assert_eq!(out["kind"], "ellipse");
        // √2·W/2 over W: 0.70711 of the box; inner: the long axis less the short one.
        assert_eq!(out["axisX"], json!({ "x": root_half(), "y": 0 }));
        assert_eq!(out["axisY"], json!({ "x": 0, "y": root_half() }));
        assert_eq!(out["inner"], json!({ "rx": 0.5, "ry": 0 }));
        // Black to white: LibreOffice's automatic count, ⌊1·127.5 + 0.5⌋.
        assert_eq!(out["steps"], json!(128));
        let out = lowered(odf("rectangular", 0.0, 0.0, 0.5, 0.5), &frame);
        assert_eq!(out["kind"], "rect");
        assert_eq!(out["inner"], json!({ "l": -0.5, "t": 0, "r": 0.5, "b": 0 }));
        let out = lowered(odf("square", 0.0, 0.0, 0.5, 0.5), &frame);
        assert_eq!(out["axisX"], json!({ "x": 0.5, "y": 0 }));
        assert_eq!(out["axisY"], json!({ "x": 0, "y": 1 }));
        assert_eq!(out["inner"], json!({ "l": 0, "t": 0, "r": 0, "b": 0 }));
    }

    #[test]
    fn lowers_gradients_on_boxes_without_area_to_fills_that_verify() {
        let angle = |extra: Value| {
            with(
                json!({ "kind": "linear", "angle": 30, "scaled": false, "rotateWithShape": false, "stops": stops() }),
                extra,
            )
        };
        let mut fills = vec![
            angle(json!({ "outset": 1 })),
            angle(json!({ "scaled": true, "tile": edges(0.1, 0.0, 0.2, 0.0) })),
            path(
                "circle",
                edges(0.1, 0.3, 0.8, 0.2),
                edges(0.0, 0.0, 0.0, 0.0),
            ),
            path("rect", edges(0.5, 0.5, 0.5, 0.5), edges(0.0, 0.0, 0.0, 0.0)),
        ];
        for style in [
            "linear",
            "axial",
            "radial",
            "ellipsoid",
            "square",
            "rectangular",
        ] {
            fills.push(odf(style, 30.0, 0.1, 0.3, 0.6));
            fills.push(with(odf(style, 0.0, 0.0, 0.5, 0.5), json!({ "steps": 4 })));
        }
        let mut elements = Vec::new();
        for (w, h) in [(0.0, 50.0), (80.0, 0.0), (0.0, 0.0)] {
            for (i, fill) in fills.iter().enumerate() {
                elements.push(json!({
                    "id": format!("ir-s1.z{}x{}n{i}", w as u32, h as u32), "kind": "shape", "origin": agent(),
                    "frame": with(frame(10.0, 10.0, w, h), json!({ "rotation": 30 })),
                    "geometry": { "kind": "preset", "name": "rect", "adjust": {} }, "fill": fill,
                }));
            }
        }
        // A row declared 0 high, which its text grows as it is drawn.
        let cell = |fill: &Value| json!({ "text": { "body": {}, "paragraphs": [] }, "fill": fill });
        elements.push(json!({
            "id": "ir-s1.t", "kind": "table", "origin": agent(), "frame": frame(0.0, 0.0, 300.0, 0.0),
            "columns": [{ "width": 300 }],
            "rows": [{ "height": 0, "cells": [cell(&fills[2])] }],
        }));
        let out = apply_pass(&pass_named("lower-geometry"), &with_elements(elements));
        assert_eq!(
            slidock_ir::validate::validate(&out.presentation),
            Vec::new()
        );
        let elements = elements_of(&out.presentation);
        assert_eq!(elements[0]["fill"]["kind"], "linear");
        assert_eq!(elements.as_array().unwrap().len(), 3 * fills.len() + 1);
        assert_eq!(
            elements[3 * fills.len()]["rows"][0]["cells"][0]["fill"]["kind"],
            "radial"
        );
    }

    #[test]
    fn holds_smooth_linear_cell_gradients_by_angle_and_lowers_the_rest_against_the_cell() {
        let cell = |fill: Value| json!({ "text": { "body": {}, "paragraphs": [] }, "fill": fill });
        let table = json!({
            "id": "ir-s1.t", "kind": "table", "origin": { "kind": "agent" },
            "frame": frame(0.0, 0.0, 300.0, 100.0),
            "columns": [{ "width": 200 }, { "width": 100 }],
            "rows": [{ "height": 100, "cells": [
                cell(odf("linear", 30.0, 0.2, 0.5, 0.5)),
                cell(path("circle", edges(0.5, 0.5, 0.5, 0.5), edges(0.0, 0.0, 0.0, 0.0))),
            ] }]
        });
        let out = lower(vec![table]);
        let cells = &out[0]["rows"][0]["cells"];
        // ODF 30° (counter-clockwise from the top) is 60° clockwise from the x axis.
        assert_eq!(cells[0]["fill"]["angle"], json!(60));
        assert!(cells[0]["fill"].get("from").is_none());
        assert_eq!(cells[0]["fill"]["stops"][0]["pos"], json!(0.2));
        // A real circle in the second cell's own 100 × 100 box.
        assert_eq!(cells[1]["fill"]["radius"], json!(root_half()));
        assert!(cells[1]["fill"].get("radiusY").is_none());
    }
}

mod plan_media {
    use super::*;
    use slidock_core_api::MediaPolicy;

    fn sha(n: u64) -> String {
        format!("{n:064x}")
    }

    fn entry(n: u64, mime: &str, bytes: u64, extra: Value) -> Value {
        with(
            json!({ "sha256": sha(n), "type": mime, "bytes": bytes, "origin": agent() }),
            extra,
        )
    }

    fn picture(id: &str, n: u64) -> Value {
        json!({ "id": id, "kind": "picture", "origin": agent(), "media": sha(n), "frame": frame(0.0, 0.0, 10.0, 10.0) })
    }

    /// Three slides showing pictures 1, 2 and 3, with these media entries.
    fn budget_presentation(entries: Vec<Value>) -> Value {
        let mut presentation = minimal_json();
        presentation["slides"] = json!(
            (1..=3)
                .map(|n| slide(&format!("ir-s{n}"), vec![picture(&format!("ir-s{n}.p"), n)]))
                .collect::<Vec<_>>()
        );
        presentation["media"] = json!(
            entries
                .into_iter()
                .map(|e| (e["sha256"].as_str().unwrap().to_owned(), e))
                .collect::<serde_json::Map<_, _>>()
        );
        presentation
    }

    fn plan(presentation: Value, policy: MediaPolicy) -> Applied {
        let presentation = parse(presentation);
        let mut env = Env::of(&presentation);
        env.policy = policy;
        apply_pass_in(&pass_named("plan-media"), &presentation, &env)
    }

    fn plans(applied: &Applied) -> Vec<Value> {
        applied
            .presentation
            .media
            .values()
            .map(|e| to_json(&e.plan))
            .collect()
    }

    fn sites(applied: &Applied) -> Vec<(String, String)> {
        applied
            .diagnostics
            .iter()
            .map(|d| {
                (
                    d.code.code().to_owned(),
                    d.container.as_ref().unwrap().to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn decides_by_type_and_codec() {
        let entries = vec![
            entry(1, "image/png", 10, json!({})),
            entry(2, "image/gif", 10, json!({ "frames": 4 })),
            entry(3, "image/tiff", 10, json!({ "width": 1, "height": 1 })),
            entry(4, "image/svg+xml", 10, json!({})),
            entry(5, "video/mp4", 10, json!({ "codec": "avc1" })),
            entry(6, "video/mp4", 10, json!({ "codec": "mp4v" })),
            entry(7, "video/quicktime", 10, json!({ "codec": "hvc1" })),
            entry(8, "audio/mpeg", 10, json!({})),
            entry(9, "image/bmp", 10, json!({ "width": 2, "height": 2 })),
            // Sized: its frontend's check passed it (entry 4 is an SVG the check refused).
            entry(10, "image/svg+xml", 10, json!({ "width": 3, "height": 2 })),
        ];
        let mut presentation = budget_presentation(entries);
        let pictures: Vec<Value> = (1..=10)
            .map(|n| picture(&format!("ir-s1.m{n}"), n))
            .collect();
        presentation["slides"] = json!([slide("ir-s1", pictures)]);
        let out = plan(presentation, MediaPolicy::default());
        let actions: Vec<Value> = plans(&out)
            .iter()
            .map(|p| json!([p["action"], p["output"]["type"], p["reason"]]))
            .collect();
        assert_eq!(
            actions,
            [
                json!(["copy", "image/png", null]),
                json!(["copy", "image/gif", null]),
                json!(["transcode", "image/png", null]),
                json!(["omit", null, "unsupported-type"]),
                json!(["copy", "video/mp4", null]),
                json!(["omit", null, "unsupported-codec"]),
                json!(["copy", "video/quicktime", null]),
                json!(["omit", null, "audio"]),
                json!(["transcode", "image/png", null]),
                json!(["copy", "image/svg+xml", null]),
            ]
        );
        let reported: Vec<Value> = out
            .diagnostics
            .iter()
            .map(|d| json!([d.code.code(), d.feature, d.data.as_ref().unwrap()["sha256"]]))
            .collect();
        assert_eq!(
            reported,
            [
                json!(["MID5606", null, sha(2)]),
                json!(["MID5602", null, sha(3)]),
                json!(["MID5601", "image.vector", sha(4)]),
                json!(["MID5605", null, sha(6)]),
                json!(["MID5607", null, sha(7)]),
                json!(["MID5602", null, sha(9)]),
            ]
        );
    }

    #[test]
    fn transcodes_a_heic_to_the_file_a_host_decoded_and_omits_it_otherwise() {
        use slidock_core_api::HostDecoded;
        let presentation = parse(budget_presentation(vec![
            entry(1, "image/heic", 3_000, json!({})),
            entry(2, "image/heic", 3_000, json!({})),
            entry(3, "image/png", 10, json!({})),
        ]));
        let decoded = |n: u64, mime: &'static str| HostDecoded {
            sha256: slidock_ir::ids::Sha256::parse(&sha(n)).unwrap(),
            mime,
            width: 4,
            height: 3,
            bytes: 20,
        };
        let mut env = Env::of(&presentation);
        // A host's file for a type no host decodes changes nothing.
        env.host_media = vec![decoded(1, "image/jpeg"), decoded(3, "image/jpeg")];
        let out = apply_pass_in(&pass_named("plan-media"), &presentation, &env);
        let actions: Vec<Value> = plans(&out)
            .iter()
            .map(|p| json!([p["action"], p["output"], p["reason"]]))
            .collect();
        assert_eq!(
            actions,
            [
                json!(["transcode", { "type": "image/jpeg", "file": format!("media/{}.jpg", sha(1)) }, null]),
                json!(["omit", null, "unsupported-type"]),
                json!(["copy", { "type": "image/png", "file": format!("media/{}.png", sha(3)) }, null]),
            ]
        );
        let reported: Vec<Value> = out
            .diagnostics
            .iter()
            .map(|d| json!([d.code.code(), d.severity, d.data.as_ref().unwrap()]))
            .collect();
        assert_eq!(
            reported,
            [
                json!(["MID5608", "info", { "sha256": sha(1), "type": "image/heic", "output": "JPEG" }]),
                json!(["MID5601", "decline", { "sha256": sha(2), "type": "image/heic" }]),
            ]
        );
        assert_eq!(
            out.diagnostics[0].message,
            format!(
                "Media {} is image/heic, which the host's browser decoded to JPEG: the deck's copy of it depends on that browser",
                sha(1)
            )
        );

        // A PNG is planned as one; the budget counts the decoded file's bytes, not the source's.
        env.host_media = vec![decoded(1, "image/png"), decoded(2, "image/jpeg")];
        env.policy = MediaPolicy {
            presentation_bytes: 50,
            ..MediaPolicy::default()
        };
        let out = apply_pass_in(&pass_named("plan-media"), &presentation, &env);
        let actions: Vec<Value> = plans(&out)
            .iter()
            .map(|p| json!([p["action"], p["output"]["type"]]))
            .collect();
        assert_eq!(
            actions,
            [
                json!(["transcode", "image/png"]),
                json!(["transcode", "image/jpeg"]),
                json!(["copy", "image/png"]),
            ]
        );
    }

    /// A HEIC no host decoded is drawn from the picture's fallback (Keynote's JPEG preview), with both sizes
    /// (MID5609); a decoded one keeps its media. Either way no picture keeps its fallback, and a picture without one
    /// still omits the HEIC (MID5601).
    #[test]
    fn draws_a_fallback_in_place_of_a_heic_no_host_decoded() {
        use slidock_core_api::HostDecoded;
        let with_fallback = |id: &str, n: u64, fallback: u64| {
            with(picture(id, n), json!({ "fallback": sha(fallback) }))
        };
        let mut presentation = budget_presentation(vec![
            entry(
                1,
                "image/heic",
                3_000,
                json!({ "width": 1600, "height": 1200 }),
            ),
            entry(2, "image/jpeg", 40, json!({ "width": 256, "height": 192 })),
            entry(3, "image/heic", 3_000, json!({})),
            entry(4, "image/png", 10, json!({})),
            entry(5, "image/png", 10, json!({})),
        ]);
        presentation["slides"] = json!([
            slide(
                "ir-s1",
                vec![
                    with_fallback("ir-s1.a", 1, 2),
                    with_fallback("ir-s1.b", 3, 4)
                ]
            ),
            // A shown medium drops its fallback; an unshown fallback leaves the original omitted.
            slide(
                "ir-s2",
                vec![
                    with_fallback("ir-s2.c", 5, 2),
                    with_fallback("ir-s2.d", 1, 3)
                ]
            ),
            slide("ir-s3", vec![picture("ir-s3.e", 1)]),
        ]);
        let presentation = parse(presentation);
        let drawn = |out: &Applied| -> Vec<Value> {
            out.presentation
                .slides
                .iter()
                .flat_map(|slide| to_json(&slide.elements).as_array().unwrap().clone())
                .map(|e| json!([e["id"], e["media"], e.get("fallback")]))
                .collect()
        };
        let reported = |out: &Applied| -> Vec<Value> {
            out.diagnostics
                .iter()
                .map(|d| {
                    json!([
                        d.code.code(),
                        d.severity,
                        d.container,
                        d.node,
                        d.data.as_ref().unwrap()
                    ])
                })
                .collect()
        };

        let mut env = Env::of(&presentation);
        let out = apply_pass_in(&pass_named("plan-media"), &presentation, &env);
        assert_eq!(
            drawn(&out),
            [
                json!(["ir-s1.a", sha(2), null]),
                json!(["ir-s1.b", sha(4), null]),
                json!(["ir-s2.c", sha(5), null]),
                json!(["ir-s2.d", sha(1), null]),
                json!(["ir-s3.e", sha(1), null]),
            ]
        );
        assert_eq!(
            reported(&out),
            [
                json!(["MID5601", "decline", "ir-s2", null, { "sha256": sha(1), "type": "image/heic" }]),
                json!(["MID5601", "decline", "ir-s3", null, { "sha256": sha(1), "type": "image/heic" }]),
                json!(["MID5609", "lossy", "ir-s1", "ir-s1.a",
                    { "sha256": sha(1), "preview": "256×192", "original": "1600×1200 HEIC" }]),
                json!(["MID5609", "lossy", "ir-s1", "ir-s1.b",
                    { "sha256": sha(3), "preview": "raster", "original": "HEIC" }]),
            ]
        );
        assert_eq!(
            out.diagnostics[2].message,
            format!(
                "Media {} is drawn from its 256×192 preview: the 1600×1200 HEIC original is not decoded here",
                sha(1)
            )
        );

        env.host_media = vec![HostDecoded {
            sha256: slidock_ir::ids::Sha256::parse(&sha(1)).unwrap(),
            mime: "image/jpeg",
            width: 1600,
            height: 1200,
            bytes: 900,
        }];
        let out = apply_pass_in(&pass_named("plan-media"), &presentation, &env);
        assert_eq!(drawn(&out)[0], json!(["ir-s1.a", sha(1), null]));
        let codes: Vec<&str> = out.diagnostics.iter().map(|d| d.code.code()).collect();
        assert_eq!(codes, ["MID5608", "MID5608", "MID5608", "MID5609"]);
    }

    #[test]
    fn names_outputs_by_the_source_bytes_hash_and_the_planned_extension() {
        let presentation = budget_presentation(vec![
            entry(1, "image/tiff", 10, json!({ "width": 1, "height": 1 })),
            entry(2, "image/jpeg", 10, json!({})),
            entry(3, "video/x-m4v", 10, json!({ "codec": "avc3" })),
        ]);
        let out = plans(&plan(presentation, MediaPolicy::default()));
        assert_eq!(
            out[0]["output"],
            json!({ "type": "image/png", "file": format!("media/{}.png", sha(1)) })
        );
        assert_eq!(out[1]["output"]["file"], format!("media/{}.jpg", sha(2)));
        assert_eq!(out[2]["output"]["file"], format!("media/{}.mp4", sha(3)));
        let svg = budget_presentation(vec![entry(
            1,
            "image/svg+xml",
            10,
            json!({ "width": 3, "height": 2 }),
        )]);
        let out = plans(&plan(svg, MediaPolicy::default()));
        assert_eq!(out[0]["output"]["file"], format!("media/{}.svg", sha(1)));
    }

    #[test]
    fn omits_an_item_over_its_limit_and_a_raster_too_large_to_transcode() {
        let policy = MediaPolicy {
            item_bytes: 100,
            transcode_pixels: 1_000,
            ..MediaPolicy::default()
        };
        let presentation = budget_presentation(vec![
            entry(1, "image/png", 101, json!({})),
            entry(2, "image/tiff", 10, json!({ "width": 100, "height": 11 })),
            entry(3, "image/png", 100, json!({})),
        ]);
        let out = plan(presentation, policy);
        let outcome: Vec<Value> = plans(&out)
            .iter()
            .map(|p| p.get("reason").cloned().unwrap_or(p["action"].clone()))
            .collect();
        assert_eq!(
            outcome,
            [json!("too-large"), json!("too-large"), json!("copy")]
        );
        assert_eq!(
            sites(&out),
            [
                ("MID5603".to_owned(), "ir-s1".to_owned()),
                ("MID5603".to_owned(), "ir-s2".to_owned())
            ]
        );
        assert_eq!(
            out.diagnostics[1].data.as_ref().unwrap()["pixels"],
            1100.0.into()
        );
    }

    #[test]
    fn keeps_the_transcodes_within_the_presentation_pixel_budget_in_media_table_order() {
        let policy = MediaPolicy {
            presentation_transcode_pixels: 1_000,
            ..MediaPolicy::default()
        };
        let entries = || {
            vec![
                entry(1, "image/tiff", 10, json!({ "width": 20, "height": 30 })),
                entry(2, "image/bmp", 10, json!({ "width": 20, "height": 30 })),
                entry(3, "image/tiff", 10, json!({ "width": 20, "height": 20 })),
            ]
        };
        let out = plan(budget_presentation(entries()), policy);
        let outcome: Vec<Value> = plans(&out)
            .iter()
            .map(|p| p.get("reason").cloned().unwrap_or(p["action"].clone()))
            .collect();
        // 600 pixels, then 600 more would pass 1,000; the third's 400 still fit.
        assert_eq!(
            outcome,
            [json!("transcode"), json!("over-budget"), json!("transcode")]
        );
        let over: Vec<&Diagnostic> = out
            .diagnostics
            .iter()
            .filter(|d| d.code.code() == "MID5610")
            .collect();
        assert_eq!(over.len(), 1);
        assert_eq!(over[0].container.as_ref().unwrap().as_str(), "ir-s2");
        assert_eq!(
            over[0].message,
            format!(
                "Media {} (600 pixels) is left out: transcoding it would pass the presentation's budget of 1000 pixels",
                sha(2)
            )
        );
        let within = plan(budget_presentation(entries()), MediaPolicy::default());
        assert!(
            within
                .diagnostics
                .iter()
                .all(|d| d.code.code() == "MID5602")
        );
    }

    #[test]
    fn keeps_the_presentation_within_budget_by_dropping_the_largest_items_of_the_latest_slides_first()
     {
        let policy = MediaPolicy {
            presentation_bytes: 250,
            ..MediaPolicy::default()
        };
        let entries = || {
            vec![
                entry(1, "image/png", 200, json!({})),
                entry(2, "image/png", 50, json!({})),
                entry(3, "image/png", 60, json!({})),
            ]
        };
        let out = plan(budget_presentation(entries()), policy);
        let actions: Vec<Value> = plans(&out).iter().map(|p| p["action"].clone()).collect();
        assert_eq!(actions, [json!("copy"), json!("copy"), json!("omit")]);
        assert_eq!(sites(&out), [("MID5604".to_owned(), "ir-s3".to_owned())]);
        assert_eq!(
            out.diagnostics[0].data.as_ref().unwrap()["budget"],
            250.0.into()
        );
        // Within the default budget, everything stays.
        assert!(
            plan(budget_presentation(entries()), MediaPolicy::default())
                .diagnostics
                .is_empty()
        );
    }

    #[test]
    fn drops_the_larger_of_two_items_on_the_same_slide_first() {
        let policy = MediaPolicy {
            presentation_bytes: 260,
            ..MediaPolicy::default()
        };
        let mut presentation = budget_presentation(vec![
            entry(1, "image/png", 200, json!({})),
            entry(3, "image/png", 60, json!({})),
            entry(4, "image/png", 100, json!({})),
        ]);
        presentation["slides"] = json!([
            slide("ir-s1", vec![picture("ir-s1.p", 1)]),
            slide("ir-s3", vec![picture("ir-s3.p", 3), picture("ir-s3.q", 4)]),
        ]);
        let out = plan(presentation, policy);
        let actions: Vec<Value> = plans(&out).iter().map(|p| p["action"].clone()).collect();
        assert_eq!(actions, [json!("copy"), json!("copy"), json!("omit")]);
    }

    #[test]
    fn drops_the_lower_hash_first_of_two_equal_items_on_the_same_slide() {
        let policy = MediaPolicy {
            presentation_bytes: 300,
            ..MediaPolicy::default()
        };
        let mut presentation = budget_presentation(vec![
            entry(1, "image/png", 200, json!({})),
            entry(4, "image/png", 100, json!({})),
            entry(3, "image/png", 100, json!({})),
        ]);
        presentation["slides"] = json!([
            slide("ir-s1", vec![picture("ir-s1.p", 1)]),
            slide("ir-s3", vec![picture("ir-s3.q", 4), picture("ir-s3.p", 3)]),
        ]);
        let out = plan(presentation, policy);
        let actions: Vec<Value> = plans(&out).iter().map(|p| p["action"].clone()).collect();
        assert_eq!(actions, [json!("copy"), json!("copy"), json!("omit")]);
    }

    #[test]
    fn counts_only_the_media_the_presentation_shows_against_the_budget() {
        let policy = MediaPolicy {
            presentation_bytes: 250,
            ..MediaPolicy::default()
        };
        let mut presentation = budget_presentation(vec![
            entry(1, "image/png", 200, json!({})),
            entry(2, "image/png", 50, json!({})),
            entry(4, "image/png", 1_000, json!({})),
        ]);
        presentation["slides"].as_array_mut().unwrap().truncate(2);
        let out = plan(presentation, policy);
        assert!(out.diagnostics.is_empty());
        let actions: Vec<Value> = plans(&out).iter().map(|p| p["action"].clone()).collect();
        assert_eq!(actions, [json!("copy"), json!("copy"), json!("copy")]);
    }

    #[test]
    fn counts_a_table_cell_image_fill_as_media_its_slide_shows() {
        let mut presentation = budget_presentation(vec![
            entry(1, "image/png", 200, json!({})),
            entry(2, "image/svg+xml", 50, json!({})),
        ]);
        let table = json!({
            "id": "ir-s3.t", "kind": "table", "origin": agent(), "frame": frame(0.0, 0.0, 10.0, 10.0),
            "columns": [{ "width": 10 }],
            "rows": [{ "height": 10, "cells": [{ "fill": { "kind": "image", "media": sha(2), "mode": "stretch" },
                                                "text": { "body": {}, "paragraphs": [] } }] }],
        });
        let mut third = presentation["slides"][2].clone();
        third["elements"] = json!([table]);
        presentation["slides"] = json!([presentation["slides"][0], third]);
        let out = plan(presentation, MediaPolicy::default());
        assert_eq!(sites(&out), [("MID5601".to_owned(), "ir-s3".to_owned())]);
    }
}

mod number_lists {
    use super::*;

    #[test]
    fn formats_every_scheme() {
        use AutoNumScheme::*;
        let schemes = [
            ArabicPeriod,
            ArabicParenR,
            ArabicParenBoth,
            ArabicPlain,
            AlphaLcPeriod,
            AlphaUcPeriod,
            AlphaLcParenR,
            AlphaUcParenR,
            AlphaLcParenBoth,
            AlphaUcParenBoth,
            RomanLcPeriod,
            RomanUcPeriod,
            RomanLcParenR,
            RomanUcParenR,
            RomanLcParenBoth,
            RomanUcParenBoth,
        ];
        let labels: Vec<String> = schemes.iter().map(|&s| number_label(s, 14)).collect();
        assert_eq!(
            labels,
            [
                "14.", "14)", "(14)", "14", "n.", "N.", "n)", "N)", "(n)", "(N)", "xiv.", "XIV.",
                "xiv)", "XIV)", "(xiv)", "(XIV)"
            ]
        );
        assert_eq!(number_label(AlphaLcPeriod, 27), "aa.");
        assert_eq!(number_label(RomanUcPeriod, 1994), "MCMXCIV.");
    }
}

mod bind_fonts {
    use super::*;

    fn sans_regular() -> FontMetrics {
        let family = builtin_catalog()
            .get(&FamilyKey::of("Liberation Sans"))
            .unwrap();
        family
            .faces
            .iter()
            .find(|f| f.weight == 400 && f.style == FaceStyle::Normal)
            .unwrap()
            .metrics
            .clone()
    }

    fn face(
        weight: u32,
        style: FaceStyle,
        name: &str,
        metrics: FontMetrics,
        variable: bool,
    ) -> CatalogFace {
        CatalogFace {
            weight,
            width: slidock_fonts::bind::NORMAL_WIDTH,
            style,
            postscript_name: name.to_owned(),
            metrics,
            variable,
            monospace: false,
            sha256: None,
            variation: None,
            file: None,
        }
    }

    fn with_builtins(family: &str, faces: Vec<CatalogFace>) -> FontCatalog {
        let added = FontCatalog::build(faces.into_iter().map(|f| (family, f)));
        FontCatalog::merge([builtin_catalog(), &added])
    }

    /// The minimal presentation requesting `family` in these faces, bound against `fonts`.
    fn bound(fonts: FontCatalog, family: &str, faces: Value) -> Applied {
        let mut presentation = minimal_json();
        presentation["fonts"] = json!({ slidock_ir::fonts::font_key(family): {
            "request": { "family": family, "faces": faces, "postscriptNames": [], "embedded": false } } });
        let presentation = passes_before("bind-fonts")
            .iter()
            .fold(parse(presentation), |p, pass| {
                apply_pass(pass, &p).presentation
            });
        let mut env = Env::of(&presentation);
        env.fonts = fonts;
        apply_pass_in(&pass_named("bind-fonts"), &presentation, &env)
    }

    #[test]
    fn requests_what_the_text_uses_and_binds_installed_families() {
        let out = bound(builtin_catalog().clone(), "Liberation Sans", json!([]));
        let entry = to_json(&out.presentation.fonts["liberationsans"]);
        assert_eq!(
            entry["request"],
            json!({ "family": "Liberation Sans", "faces": [{ "weight": 400, "style": "normal" }],
                    "postscriptNames": [], "embedded": false })
        );
        assert_eq!(entry["binding"]["kind"], "installed");
        assert_eq!(entry["binding"]["generic"], "sans-serif");
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn keeps_what_the_frontend_recorded_and_reports_frontend_only_requests_presentation_wide() {
        let mut presentation = minimal_json();
        presentation["fonts"] = json!({ "courier": { "request": { "family": "Courier",
            "faces": [{ "weight": 700, "style": "normal" }], "postscriptNames": ["Courier-Bold"], "embedded": true } } });
        let presentation = passes_before("bind-fonts")
            .iter()
            .fold(parse(presentation), |p, pass| {
                apply_pass(pass, &p).presentation
            });
        let out = apply_pass(&pass_named("bind-fonts"), &presentation);
        let courier = to_json(&out.presentation.fonts["courier"]);
        assert_eq!(
            courier["request"],
            json!({ "family": "Courier", "faces": [{ "weight": 700, "style": "normal" }],
                    "postscriptNames": ["Courier-Bold"], "embedded": true })
        );
        assert_eq!(courier["binding"]["family"], "Liberation Mono");
        let reported = out
            .diagnostics
            .iter()
            .find(|d| d.data.as_ref().unwrap()["family"] == "Courier".into())
            .unwrap();
        assert_eq!(reported.code.code(), "MID5401");
        assert_eq!(reported.container, None);
    }

    #[test]
    fn an_emoji_family_binds_the_emoji_generic_whatever_the_catalog_holds() {
        use slidock_ir::fonts::{BindingKind, GenericFamily};
        // Even a catalog holding a face of the family binds it to the platform emoji face: no emoji file ships.
        let fonts = with_builtins(
            "Apple Color Emoji",
            vec![face(
                400,
                FaceStyle::Normal,
                "AppleColorEmoji",
                sans_regular(),
                false,
            )],
        );
        for family in [
            "Apple Color Emoji",
            "AppleColorEmoji",
            "Segoe UI Emoji",
            "Noto Color Emoji",
        ] {
            let bound = bind_family::<&str>(&fonts, family, &[]).unwrap();
            assert_eq!(bound.binding.kind, BindingKind::Generic, "{family}");
            assert_eq!(bound.binding.generic, GenericFamily::Emoji, "{family}");
            assert_eq!(bound.binding.file, None, "{family}");
        }
        let out = bound(builtin_catalog().clone(), "Apple Color Emoji", json!([]));
        let entry = to_json(&out.presentation.fonts["applecoloremoji"]);
        assert_eq!(entry["binding"]["generic"], "emoji");
        // One info, not a substitution.
        let codes: Vec<&str> = out.diagnostics.iter().map(|d| d.code.code()).collect();
        assert_eq!(codes, ["MID5406"]);
        // Negative control: a symbol font is no emoji font, and binds as before.
        let symbols = bind_family::<&str>(builtin_catalog(), "Segoe UI Symbol", &[]).unwrap();
        assert_eq!(symbols.binding.generic, GenericFamily::SansSerif);
        assert_eq!(symbols.binding.kind, BindingKind::Generic);
    }

    #[test]
    fn a_family_whose_held_faces_were_refused_says_why() {
        let reason = "browsers would refuse its file (The face has no post table)";
        let fonts = builtin_catalog()
            .clone()
            .with_refused([("Courier", reason.to_owned())]);
        let out = bound(fonts, "Courier", json!([]));
        assert_eq!(out.diagnostics.len(), 1);
        let reported = &out.diagnostics[0];
        assert_eq!(reported.code.code(), "MID5404");
        assert_eq!(reported.data.as_ref().unwrap()["reason"], reason.into());
        assert_eq!(
            reported.data.as_ref().unwrap()["binding"],
            "Liberation Mono".into()
        );
        // By PostScript name too, for a family the presentation names otherwise.
        let fonts = builtin_catalog()
            .clone()
            .with_refused([("Courier-Bold", reason.to_owned())]);
        let by_name = bind_family(&fonts, "Courier New", &["Courier-Bold"]).unwrap();
        assert_eq!(by_name.binding.family, "Liberation Mono");
        assert_eq!(
            fonts.refusal("Courier New", &["Courier-Bold"]),
            Some(reason)
        );
        // Negative control: nothing refused, the substitution is MID5401.
        let plain = bound(builtin_catalog().clone(), "Courier", json!([]));
        assert_eq!(plain.diagnostics[0].code.code(), "MID5401");
    }

    #[test]
    fn binds_a_substitute_through_a_weight_name() {
        let bound = bind_family::<&str>(builtin_catalog(), "Arial Bold", &[]).unwrap();
        assert_eq!(
            (bound.binding.kind, bound.binding.weight),
            (slidock_ir::fonts::BindingKind::Substitute, Some(700))
        );
        assert_eq!(bound.binding.family, "Liberation Sans");
    }

    #[test]
    fn cambria_binds_caladea_as_a_serif_where_the_catalog_holds_it() {
        use slidock_fonts::catalog::builtin::floor_catalog;
        use slidock_ir::fonts::{BindingKind, GenericFamily};
        let fonts = FontCatalog::merge([builtin_catalog(), floor_catalog()]);
        let bound = bind_family::<&str>(&fonts, "Cambria", &[]).unwrap();
        assert_eq!(
            (bound.binding.kind, bound.binding.family.as_str()),
            (BindingKind::Substitute, "Caladea")
        );
        assert_eq!(bound.binding.generic, GenericFamily::Serif);
        // The built-in table lacks Caladea: Cambria binds its generic, a serif too.
        let bound = bind_family::<&str>(builtin_catalog(), "Cambria", &[]).unwrap();
        assert_eq!(
            (bound.binding.kind, bound.binding.family.as_str()),
            (BindingKind::Generic, "Liberation Serif")
        );
        assert_eq!(bound.binding.generic, GenericFamily::Serif);
    }

    #[test]
    fn reports_a_variable_fonts_other_weights() {
        let fonts = with_builtins(
            "Flex",
            vec![face(
                400,
                FaceStyle::Normal,
                "Flex-Regular",
                sans_regular(),
                true,
            )],
        );
        let out = bound(fonts, "Flex", json!([{ "weight": 700, "style": "normal" }]));
        let messages: Vec<&str> = out
            .diagnostics
            .iter()
            .filter(|d| d.code.code() == "MID5405")
            .map(|d| d.message.as_str())
            .collect();
        assert_eq!(
            messages,
            ["Flex is a variable font bound at its default instance; weight 700 is approximated"]
        );
    }

    #[test]
    fn binds_a_family_its_request_spells_differently_by_a_recorded_postscript_name() {
        let fonts = with_builtins(
            "Party LET",
            vec![face(
                400,
                FaceStyle::Normal,
                "PartyLetPlain",
                sans_regular(),
                false,
            )],
        );
        let bound = bind_family(&fonts, "Party Let Plain", &["PartyLetPlain"]).unwrap();
        assert_eq!(
            (bound.binding.kind, bound.binding.family.as_str()),
            (slidock_ir::fonts::BindingKind::Installed, "Party LET")
        );
        // The family name wins when it binds; without a matching PostScript name the substitutes and generics
        // apply.
        assert_eq!(
            bind_family(&fonts, "Liberation Sans", &["PartyLetPlain"])
                .unwrap()
                .binding
                .family,
            "Liberation Sans"
        );
        assert_eq!(
            bind_family(&fonts, "Party Let Plain", &["Other-Regular"])
                .unwrap()
                .binding
                .kind,
            slidock_ir::fonts::BindingKind::Generic
        );
    }

    #[test]
    fn records_the_metrics_of_each_drawn_face_whose_file_measures_otherwise() {
        let regular = sans_regular();
        let mut taller = regular.clone();
        taller.hhea.ascender += 100;
        let fonts = with_builtins(
            "Tall",
            vec![
                face(
                    400,
                    FaceStyle::Normal,
                    "Tall-400normal",
                    regular.clone(),
                    false,
                ),
                face(
                    400,
                    FaceStyle::Italic,
                    "Tall-400italic",
                    regular.clone(),
                    false,
                ),
                face(
                    700,
                    FaceStyle::Normal,
                    "Tall-700normal",
                    taller.clone(),
                    false,
                ),
            ],
        );
        let faces = json!([
            { "weight": 400, "style": "normal" }, { "weight": 400, "style": "italic" },
            { "weight": 700, "style": "normal" }, { "weight": 700, "style": "italic" },
        ]);
        let out = bound(fonts, "Tall", faces);
        let binding = to_json(&out.presentation.fonts["tall"].binding);
        assert_eq!(binding["metrics"], to_json(&regular));
        // Bold italic lacks a file of its own; CSS matches style before weight, so the italic face draws it.
        assert_eq!(
            binding["faces"],
            json!([{ "weight": 700, "style": "normal", "metrics": to_json(&taller) }])
        );
    }

    #[test]
    fn picks_the_face_css_draws_not_the_nearest() {
        // Asked for 600 among 400, 500 and 700, CSS draws the heavier 700; "nearest, lighter on a tie" would take
        // 500.
        let mut heavy = sans_regular();
        heavy.hhea.ascender += 50;
        let fonts = with_builtins(
            "Steps",
            vec![
                face(400, FaceStyle::Normal, "Steps-400", sans_regular(), false),
                face(500, FaceStyle::Normal, "Steps-500", sans_regular(), false),
                face(700, FaceStyle::Normal, "Steps-700", heavy.clone(), false),
            ],
        );
        let bound = bind_family(&fonts, "Steps SemiBold", &[] as &[&str]).unwrap();
        assert_eq!(bound.binding.weight, Some(600));
        assert_eq!(bound.face.weight, 700);
        assert_eq!(bound.binding.metrics, heavy);
    }

    #[test]
    fn fails_when_the_catalog_cannot_render_a_generic() {
        let error = bind_family::<&str>(&FontCatalog::default(), "Anything", &[]).unwrap_err();
        assert!(error.to_string().contains("no Liberation Sans"), "{error}");
    }
}

mod bind_placeholders {
    use super::*;

    /// One slide on layout `ir-l.x`, which has no placeholders and failed as `failed` says, with one body
    /// placeholder.
    fn on_layout(failed: Option<Value>) -> Presentation {
        let mut presentation = minimal_json();
        let mut layout =
            json!({ "id": "ir-l.x", "origin": agent(), "showMaster": true, "elements": [] });
        if let Some(failed) = failed {
            layout["failed"] = failed;
        }
        presentation["layouts"] = json!({ "ir-l.x": layout });
        let body = with(
            rect("ir-s1.body", frame(0.0, 0.0, 10.0, 10.0)),
            json!({ "placeholder": { "role": "body", "index": 1 } }),
        );
        presentation["slides"][0]["layout"] = json!("ir-l.x");
        presentation["slides"][0]["elements"] = json!([body]);
        parse(presentation)
    }

    #[test]
    fn warns_of_an_unmatched_placeholder_only_when_its_parent_has_not_failed() {
        let bind = |presentation: &Presentation| {
            let out = apply_pass(&pass_named("bind-placeholders"), presentation);
            let edge = elements_of(&out.presentation)[0].get("inherits").cloned();
            (codes(&out.diagnostics), edge)
        };
        assert_eq!(bind(&on_layout(None)), (vec!["MID5101".to_owned()], None));
        let failed = json!({ "code": "IR0908", "pass": "cascade-text" });
        assert_eq!(bind(&on_layout(Some(failed))), (vec![], None));
    }
}

mod resolve_theme {
    use super::*;

    fn accent1() -> Value {
        json!({ "kind": "theme", "slot": "accent1", "mods": [] })
    }

    fn stroke() -> Value {
        json!({ "width": 1, "fill": { "kind": "solid", "color": accent1() }, "dash": "solid", "cap": "flat",
                "join": "miter" })
    }

    #[test]
    fn resolves_the_theme_colours_of_picture_strokes_and_foreign_fallbacks_but_not_their_payloads()
    {
        let picture = |id: &str| {
            json!({ "id": id, "kind": "picture", "origin": agent(), "media": format!("{:064x}", 1),
                    "frame": frame(0.0, 0.0, 10.0, 10.0), "stroke": stroke() })
        };
        let filled = with(
            rect("ir-s1.f.shape", frame(0.0, 0.0, 10.0, 10.0)),
            json!({ "fill": { "kind": "solid", "color": accent1() } }),
        );
        let foreign = json!({
            "id": "ir-s1.f", "kind": "foreign", "origin": agent(), "dialect": "pptx", "type": "chart",
            "frame": frame(0.0, 0.0, 10.0, 10.0), "payload": { "color": accent1() },
            "fallback": { "elements": [filled, picture("ir-s1.f.picture")] },
        });
        let input = with_elements(vec![picture("ir-s1.picture"), foreign]);
        let out = apply_pass(
            &pass_named("resolve-theme"),
            &apply_passes(&passes_before("resolve-theme"), &input).presentation,
        );
        let elements = elements_of(&out.presentation);
        // No theme: Office's default accent1, 4472c4.
        let resolved = rgb(0.26667, 0.44706, 0.76863);
        let fallback = &elements[1]["fallback"]["elements"];
        for color in [
            &elements[0]["stroke"]["fill"]["color"],
            &fallback[0]["fill"]["color"],
            &fallback[1]["stroke"]["fill"]["color"],
        ] {
            assert_eq!(color["resolved"], resolved, "{color}");
        }
        assert_eq!(elements[1]["payload"]["color"], accent1());
    }
}

mod a11y {
    use super::*;

    /// A line of text at height `y`, a placeholder when `placeholder` is one.
    fn line(id: &str, y: f64, words: &str, placeholder: Value) -> Value {
        let mut shape = with(
            rect(id, frame(60.0, y, 600.0, 40.0)),
            json!({ "text": text(vec![paragraph(0, json!({}), vec![run(words, json!({}))])]) }),
        );
        if !placeholder.is_null() {
            shape["placeholder"] = placeholder;
        }
        shape
    }

    fn resolved_slide(elements: Vec<Value>) -> slidock_ir::source::Slide {
        let input = with_elements(elements);
        apply_passes(&slidock_passes::passes(), &input)
            .presentation
            .slides
            .remove(0)
    }

    #[test]
    fn cuts_a_title_at_200_utf_16_code_units_dropping_a_surrogate_pair_the_cut_splits() {
        let title = |words: &str| {
            resolved_slide(vec![line("ir-s1.t", 0.0, words, Value::Null)])
                .title
                .unwrap()
        };
        let a = |n: usize| "a".repeat(n);
        // Each astral character is two code units.
        assert_eq!(title(&"😀".repeat(150)), "😀".repeat(100));
        assert_eq!(title(&format!("{}😀b", a(198))), format!("{}😀", a(198)));
        assert_eq!(title(&format!("{}😀", a(199))), a(199));
        assert_eq!(title(&format!("{}é😀", a(198))), format!("{}é", a(198)));
    }

    #[test]
    fn reads_titles_then_placeholders_by_index_the_unindexed_last_then_the_rest_in_z_order() {
        let body = |index: Option<u64>| match index {
            Some(index) => json!({ "role": "body", "index": index }),
            None => json!({ "role": "body" }),
        };
        let mut hidden = line("ir-s1.hidden", 0.0, "hidden", body(Some(0)));
        hidden["hidden"] = json!(true);
        let slide = resolved_slide(vec![
            line("ir-s1.note", 0.0, "note", Value::Null),
            line("ir-s1.none", 0.0, "none", body(None)),
            line("ir-s1.b2", 0.0, "two", body(Some(2))),
            hidden,
            line("ir-s1.title", 0.0, "Title", json!({ "role": "title" })),
            line("ir-s1.none2", 0.0, "none again", body(None)),
            line("ir-s1.b1", 0.0, "one", body(Some(1))),
            line("ir-s1.deco", 0.0, "deco", Value::Null),
        ]);
        let order: Vec<String> = slide
            .reading_order
            .unwrap()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            order,
            [
                "ir-s1.title",
                "ir-s1.b1",
                "ir-s1.b2",
                "ir-s1.none",
                "ir-s1.none2",
                "ir-s1.note",
                "ir-s1.deco"
            ]
        );
        assert_eq!(slide.title.as_deref(), Some("Title"));
    }
}

mod properties {
    use super::*;

    /// The presentation each pass receives in the pipeline, starting from `presentation`.
    fn inputs(presentation: Presentation) -> Vec<Presentation> {
        let mut out = vec![presentation];
        for pass in slidock_passes::passes() {
            let next = apply_pass(&pass, out.last().unwrap()).presentation;
            out.push(next);
        }
        out
    }

    #[test]
    fn every_pass_is_idempotent_and_keeps_ids_on_a_built_presentation() {
        let presentations = inputs(minimal());
        for (index, pass) in slidock_passes::passes().iter().enumerate() {
            let once = &presentations[index + 1];
            let twice = apply_pass(pass, once).presentation;
            assert_eq!(to_json(&twice), to_json(once), "{}", pass.name());
            let ids = |p: &Presentation| -> Vec<String> {
                p.containers()
                    .flat_map(|c| slidock_core_api::visit::walk_elements(c.elements()))
                    .map(|e: &Element| e.id().to_string())
                    .collect()
            };
            assert_eq!(ids(once), ids(&presentations[index]), "{}", pass.name());
        }
        let resolved = presentations.last().unwrap();
        assert_eq!(
            resolved.slides[0].title.as_deref(),
            Some("Hello, presentation")
        );
        assert!(codes(&apply_passes(&slidock_passes::passes(), &minimal()).diagnostics).is_empty());
    }
}

mod shadows {
    use super::*;

    fn shadow(dy: f64, color: Value) -> Value {
        json!({ "dx": 0, "dy": dy, "sigma": 7.14, "color": color })
    }

    fn accent1() -> Value {
        json!({ "kind": "theme", "slot": "accent1", "mods": [] })
    }

    fn placeholder(id: &str, kind: &str, extra: Value) -> Value {
        let mut element = with(
            json!({ "id": id, "kind": kind, "origin": agent(), "placeholder": { "role": "body", "index": 1 } }),
            extra,
        );
        if kind == "picture" {
            element["media"] = json!(format!("{:064x}", 1));
        }
        element
    }

    /// A layout body placeholder with a shadow; on the slide a shape and a picture inheriting it, a shape stating
    /// none, and a shape with its own.
    fn chained() -> Presentation {
        let mut presentation = minimal_json();
        let layout_body = placeholder(
            "ir-l.body",
            "shape",
            json!({ "frame": frame(0.0, 0.0, 100.0, 50.0), "shadow": shadow(20.0, rgb(0.0, 0.0, 0.0)) }),
        );
        presentation["layouts"] = json!({ "ir-l": { "id": "ir-l", "origin": agent(), "showMaster": true,
            "elements": [layout_body] } });
        presentation["slides"][0]["layout"] = json!("ir-l");
        presentation["slides"][0]["elements"] = json!([
            placeholder("ir-s1.shape", "shape", json!({ "inherits": "ir-l.body" })),
            placeholder(
                "ir-s1.picture",
                "picture",
                json!({ "inherits": "ir-l.body" })
            ),
            placeholder(
                "ir-s1.none",
                "shape",
                json!({ "inherits": "ir-l.body", "shadow": null })
            ),
            placeholder(
                "ir-s1.own",
                "shape",
                json!({ "inherits": "ir-l.body", "shadow": shadow(3.0, accent1()) })
            ),
        ]);
        parse(presentation)
    }

    #[test]
    fn an_absent_shadow_inherits_whatever_the_kind_and_null_stops_it() {
        let input = chained();
        let out = apply_pass(
            &pass_named("resolve-inheritance"),
            &apply_passes(&passes_before("resolve-inheritance"), &input).presentation,
        );
        let elements = elements_of(&out.presentation);
        assert_eq!(elements[0]["shadow"]["dy"], 20);
        assert_eq!(elements[1]["shadow"]["dy"], 20);
        assert_eq!(elements[2]["shadow"], Value::Null);
        assert!(elements[2].get("shadow").is_some());
        assert_eq!(elements[3]["shadow"]["dy"], 3);
    }

    #[test]
    fn resolves_the_theme_colours_of_element_and_run_shadows() {
        let mut presentation = minimal_json();
        presentation["slides"][0]["elements"][0]["shadow"] = shadow(3.0, accent1());
        presentation["slides"][0]["elements"][0]["text"]["paragraphs"][0]["inlines"][0]["props"]
            ["shadow"] = shadow(2.0, accent1());
        let input = parse(presentation);
        let out = apply_pass(
            &pass_named("resolve-theme"),
            &apply_passes(&passes_before("resolve-theme"), &input).presentation,
        );
        let elements = elements_of(&out.presentation);
        let resolved = rgb(0.26667, 0.44706, 0.76863);
        assert_eq!(elements[0]["shadow"]["color"]["resolved"], resolved);
        assert_eq!(
            elements[0]["text"]["paragraphs"][0]["inlines"][0]["props"]["shadow"]["color"]["resolved"],
            resolved
        );
    }

    #[test]
    fn a_run_shadow_cascades_like_a_highlight_and_null_stops_it() {
        let mut presentation = minimal_json();
        let hello = &mut presentation["slides"][0]["elements"][0];
        hello["text"]["listStyle"] =
            json!({ "0": { "run": { "shadow": shadow(2.0, rgb(0.0, 0.0, 0.0)) } } });
        let run = hello["text"]["paragraphs"][0]["inlines"][0].clone();
        let mut none = run.clone();
        none["props"]["shadow"] = Value::Null;
        hello["text"]["paragraphs"][0]["inlines"] = json!([run, none]);
        let input = parse(presentation);
        let out = apply_passes(&passes_before("number-lists"), &input).presentation;
        let paragraph = &elements_of(&out)[0]["text"]["paragraphs"][0];
        assert_eq!(paragraph["inlines"][0]["props"]["shadow"]["dy"], 2);
        assert_eq!(paragraph["inlines"][1]["props"]["shadow"], Value::Null);
        assert_eq!(paragraph["endProps"]["shadow"]["dy"], 2);
        // The defaults state none, so a run without a list style has none.
        let plain = apply_passes(&passes_before("number-lists"), &minimal()).presentation;
        let props = &elements_of(&plain)[0]["text"]["paragraphs"][0]["inlines"][0]["props"];
        assert!(props.get("shadow").is_none(), "{props}");
    }

    #[test]
    fn a_groups_child_space_scales_its_childrens_frames_not_their_shadows() {
        let child = with(
            rect("ir-g.a", frame(1000.0, 1000.0, 1000.0, 500.0)),
            json!({ "shadow": shadow(20.0, rgb(0.0, 0.0, 0.0)) }),
        );
        let group = json!({ "id": "ir-g", "kind": "group", "origin": agent(), "frame": frame(60.0, 160.0, 300.0, 150.0),
            "childSpace": { "x": 1000, "y": 1000, "w": 2000, "h": 1000 }, "children": [child] });
        let out = apply_pass(&pass_named("normalize-frames"), &with_elements(vec![group]));
        let elements = elements_of(&out.presentation);
        assert_eq!(
            elements[0]["children"][0]["frame"],
            frame(0.0, 0.0, 150.0, 75.0)
        );
        let kept = &elements[0]["children"][0]["shadow"];
        assert_eq!(
            (kept["dx"].as_f64(), kept["dy"].as_f64()),
            (Some(0.0), Some(20.0))
        );
        assert_eq!(kept["sigma"].as_f64(), Some(7.14));
    }
}
