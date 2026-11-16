//! Gradient fills: `a:path` as the IR's path source with its `fillToRect` and `tileRect`
//! insets, `a:lin` with its tile, what PX2106 reports instead of drawing, and the lowered geometry of shapes, cells,
//! backgrounds and theme fill styles.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::{Value, json};
use slidock_ir::features::FeatureId;
use support::{
    DEFAULT_THEME, MasterSpec, PptxSpec, Row, SlideSpec, Sp, TableSpec, assert_matches,
    compile_bytes, element, json, lower, pptx, slide, sp, table, tc, xfrm,
};

const STOPS: &str = "<a:gsLst><a:gs pos=\"0\"><a:srgbClr val=\"000000\"/></a:gs><a:gs pos=\"100000\"><a:srgbClr val=\"FFFFFF\"/></a:gs></a:gsLst>";

/// A path gradient of this type with these `fillToRect` attributes and this `tileRect` (none when empty).
fn path(kind: &str, fill_to: &str, tile: &str) -> String {
    let tile = if tile.is_empty() {
        String::new()
    } else {
        format!("<a:tileRect {tile}/>")
    };
    format!(
        "<a:gradFill rotWithShape=\"1\">{STOPS}<a:path path=\"{kind}\"><a:fillToRect {fill_to}/></a:path>{tile}</a:gradFill>"
    )
}

/// A linear gradient at `ang` (60000ths of a degree) with this `tileRect` (none when empty).
fn lin(ang: i64, tile: &str) -> String {
    let tile = if tile.is_empty() {
        String::new()
    } else {
        format!("<a:tileRect {tile}/>")
    };
    format!(
        "<a:gradFill rotWithShape=\"1\">{STOPS}<a:lin ang=\"{ang}\" scaled=\"0\"/>{tile}</a:gradFill>"
    )
}

/// A 600 × 200 pt rectangle filled with `fill`.
fn shape(id: u64, fill: &str) -> String {
    sp(Sp {
        id,
        sp_pr: format!(
            "{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>{fill}",
            xfrm(180.0, 170.0, 600.0, 200.0, "")
        ),
        ..Sp::default()
    })
}

fn deck(shapes: &[String]) -> Vec<u8> {
    pptx(PptxSpec {
        slides: vec![slide(shapes.join(""))],
        ..PptxSpec::default()
    })
}

/// The PX2106 reasons on `node`, in output order.
fn reasons(diagnostics: &[slidock_ir::diagnostics::Diagnostic], node: &str) -> Vec<String> {
    diagnostics
        .iter()
        .filter(|d| d.code.code() == "PX2106")
        .filter(|d| d.node.as_ref().is_some_and(|n| n.as_str() == node))
        .map(|d| json(&d.data)["reason"].as_str().unwrap().to_owned())
        .collect()
}

fn features(diagnostics: &[slidock_ir::diagnostics::Diagnostic], node: &str) -> Vec<FeatureId> {
    diagnostics
        .iter()
        .filter(|d| d.code.code() == "PX2106")
        .filter(|d| d.node.as_ref().is_some_and(|n| n.as_str() == node))
        .filter_map(|d| d.feature)
        .collect()
}

#[test]
fn keeps_path_gradients_as_their_fill_to_rect_and_tile_rect_insets() {
    let lowered = lower(&deck(&[
        shape(
            10,
            &path(
                "circle",
                "l=\"10000\" t=\"30000\" r=\"80000\" b=\"20000\"",
                "",
            ),
        ),
        shape(
            11,
            &path(
                "rect",
                "l=\"100000\" t=\"100000\"",
                "r=\"-100000\" b=\"-100000\"",
            ),
        ),
    ]));
    let slide = &lowered.presentation().slides[0];
    assert_matches(
        &element(slide, "pp-256.10")["fill"],
        json!({
            "kind": "path", "path": "circle",
            "focus": { "l": 0.1, "t": 0.3, "r": 0.8, "b": 0.2 },
            "tile": { "l": 0, "t": 0, "r": 0, "b": 0 },
        }),
    );
    assert_matches(
        &element(slide, "pp-256.11")["fill"],
        json!({
            "kind": "path", "path": "rect",
            "focus": { "l": 1, "t": 1, "r": 0, "b": 0 },
            "tile": { "l": 0, "t": 0, "r": -1, "b": -1 },
        }),
    );
    // A tile larger than the box draws exactly: nothing to report.
    assert!(lowered.find("PX2106").is_none());
    let features = &lowered.lowered.features["pp-256"];
    assert!(features.contains(&FeatureId::FillRadial) && features.contains(&FeatureId::FillPath));
}

#[test]
fn draws_a_path_that_follows_the_outline_as_a_rectangular_one_and_reports_it() {
    let lowered = lower(&deck(&[shape(
        10,
        &path(
            "shape",
            "l=\"50000\" t=\"50000\" r=\"50000\" b=\"50000\"",
            "",
        ),
    )]));
    assert_matches(
        &element(&lowered.presentation().slides[0], "pp-256.10")["fill"],
        json!({ "kind": "path", "path": "rect" }),
    );
    assert_eq!(
        reasons(&lowered.diagnostics, "pp-256.10"),
        ["a path gradient that follows the outline is drawn as a rectangular one"]
    );
    assert_eq!(
        features(&lowered.diagnostics, "pp-256.10"),
        [FeatureId::FillPath]
    );
    assert!(lowered.lowered.features["pp-256"].contains(&FeatureId::FillPath));
}

#[test]
fn draws_nothing_for_an_unknown_path_type_and_reports_it() {
    let lowered = lower(&deck(&[shape(10, &path("star", "", ""))]));
    assert_matches(
        &element(&lowered.presentation().slides[0], "pp-256.10")["fill"],
        json!({ "kind": "none" }),
    );
    assert_eq!(
        reasons(&lowered.diagnostics, "pp-256.10"),
        ["unknown path type star"]
    );
}

#[test]
fn draws_a_tile_smaller_than_the_shape_once_and_reports_it() {
    let lowered = lower(&deck(&[
        shape(10, &lin(0, "l=\"50000\"")),
        shape(11, &lin(0, "l=\"-50000\" r=\"-50000\"")),
        shape(12, &path("circle", "", "t=\"60000\" b=\"40000\"")),
        shape(13, &lin(0, "")),
    ]));
    let slide = &lowered.presentation().slides[0];
    assert_matches(
        &element(slide, "pp-256.10")["fill"],
        json!({ "kind": "linear", "tile": { "l": 0.5, "t": 0, "r": 0, "b": 0 } }),
    );
    assert_eq!(
        reasons(&lowered.diagnostics, "pp-256.10"),
        ["a tile smaller than the shape is drawn once, not repeated"]
    );
    assert_eq!(
        features(&lowered.diagnostics, "pp-256.10"),
        [FeatureId::FillLinear]
    );
    // An outset tile: drawn exactly.
    assert_matches(
        &element(slide, "pp-256.11")["fill"],
        json!({ "kind": "linear", "tile": { "l": -0.5, "r": -0.5 } }),
    );
    assert!(reasons(&lowered.diagnostics, "pp-256.11").is_empty());
    // An empty tile: the box.
    assert_eq!(
        element(slide, "pp-256.12")["fill"]["tile"],
        json!({ "l": 0, "t": 0, "r": 0, "b": 0 })
    );
    assert_eq!(
        reasons(&lowered.diagnostics, "pp-256.12"),
        ["an empty tile is drawn as the shape"]
    );
    // No tile: none in the IR.
    assert!(element(slide, "pp-256.13")["fill"].get("tile").is_none());
}

/// The resolved fill of shape `id` on the only slide.
fn resolved_fill(ir: &Value, id: &str) -> Value {
    ir["slides"][0]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["id"] == id)
        .unwrap()["fill"]
        .clone()
}

#[test]
fn lowers_a_circle_path_to_a_real_circle_about_the_tile_with_its_focus() {
    let compiled = compile_bytes(
        &deck(&[
            shape(
                10,
                &path(
                    "circle",
                    "l=\"10000\" t=\"30000\" r=\"80000\" b=\"20000\"",
                    "",
                ),
            ),
            // A focus 2 box heights above the box lies outside the circle: moved to 0.98 of its ray.
            shape(
                11,
                &path(
                    "circle",
                    "l=\"50000\" t=\"-200000\" r=\"50000\" b=\"300000\"",
                    "",
                ),
            ),
        ]),
        "paths.pptx",
    );
    assert!(compiled.errors().is_empty(), "{:?}", compiled.errors());
    let ir = json(compiled.presentation.as_ref().unwrap());
    // R = ½·hypot(600, 200) = 316.228 pt: 0.52705 of W, 1.58114 of H; the focus divides the focus rectangle as
    // the focus rectangle divides the box.
    assert_eq!(
        resolved_fill(&ir, "pp-256.10"),
        json!({
            "kind": "radial", "center": { "x": 0.5, "y": 0.5 }, "radius": 0.52705, "radiusY": 1.58114,
            "focus": { "x": 0.11111, "y": 0.6 },
            "stops": resolved_fill(&ir, "pp-256.10")["stops"],
        })
    );
    let moved = resolved_fill(&ir, "pp-256.11");
    let focus_y = moved["focus"]["y"].as_f64().unwrap();
    assert!((focus_y - (0.5 - 0.98 * 1.58114)).abs() < 2e-5, "{moved}");
    assert_eq!(
        reasons(&compiled.diagnostics, "pp-256.11"),
        ["the focus lies outside the circle"]
    );
    assert!(reasons(&compiled.diagnostics, "pp-256.10").is_empty());
}

#[test]
fn lowers_cell_backgrounds_and_theme_path_gradients_against_their_own_boxes() {
    let cell_path = path(
        "circle",
        "l=\"50000\" t=\"50000\" r=\"50000\" b=\"50000\"",
        "",
    );
    let grid = table(TableSpec {
        id: 20,
        frame: xfrm(180.0, 170.0, 600.0, 200.0, ""),
        tbl_pr: None,
        columns: vec![300.0, 300.0],
        rows: vec![Row {
            height: 100.0,
            cells: vec![
                tc(None, &format!("<a:tcPr>{cell_path}</a:tcPr>"), ""),
                tc(
                    None,
                    &format!("<a:tcPr>{}</a:tcPr>", lin(5_400_000, "")),
                    "",
                ),
            ],
        }],
        ph: String::new(),
    });
    // A theme whose third background fill style is a centred circle path in the reference's colour.
    let theme_path = "<a:gradFill rotWithShape=\"1\"><a:gsLst><a:gs pos=\"0\"><a:schemeClr val=\"phClr\"/></a:gs><a:gs pos=\"100000\"><a:srgbClr val=\"FFFFFF\"/></a:gs></a:gsLst><a:path path=\"circle\"><a:fillToRect l=\"50000\" t=\"50000\" r=\"50000\" b=\"50000\"/></a:path></a:gradFill>";
    let theme = DEFAULT_THEME.replace(
        "<a:bgFillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"><a:tint val=\"95000\"/></a:schemeClr></a:solidFill>",
        &format!("<a:bgFillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"><a:tint val=\"95000\"/></a:schemeClr></a:solidFill>{theme_path}"),
    );
    assert_ne!(theme, DEFAULT_THEME);
    let bytes = pptx(PptxSpec {
        masters: Some(vec![MasterSpec {
            theme: Some(theme),
            layouts: vec![Default::default()],
            ..MasterSpec::default()
        }]),
        slides: vec![SlideSpec {
            bg: Some("<p:bgRef idx=\"1003\"><a:srgbClr val=\"2F6FDF\"/></p:bgRef>".to_owned()),
            ..slide(grid)
        }],
        ..PptxSpec::default()
    });
    let compiled = compile_bytes(&bytes, "boxes.pptx");
    assert!(compiled.errors().is_empty(), "{:?}", compiled.errors());
    let ir = json(compiled.presentation.as_ref().unwrap());
    let cells = &ir["slides"][0]["elements"][0]["rows"][0]["cells"];
    // The 300 × 100 cell's circle: R = ½·hypot(300, 100).
    assert_matches(
        &cells[0]["fill"],
        json!({ "kind": "radial", "center": { "x": 0.5, "y": 0.5 }, "radius": 0.52705, "radiusY": 1.58114 }),
    );
    assert!(cells[0]["fill"].get("focus").is_none());
    // An unscaled lin stays an angle the cell's CSS draws.
    assert_matches(&cells[1]["fill"], json!({ "kind": "linear", "angle": 90 }));
    // The 960 × 540 slide's circle: R = ½·hypot(960, 540), from the reference's colour.
    let background = &ir["slides"][0]["background"];
    assert_matches(
        background,
        json!({ "kind": "radial", "center": { "x": 0.5, "y": 0.5 }, "radius": 0.57367, "radiusY": 1.01986 }),
    );
    assert_matches(
        &background["stops"][0]["color"],
        json!({ "r": 0.18431, "g": 0.43529, "b": 0.87451 }),
    );
}
