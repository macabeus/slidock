//! The gradient probe deck (`probes/out/gradient-probe.pptx`, built by `cargo xtask probes-gradients`) compiled
//! through the standard pipeline: every gradient's frame-local geometry against PowerPoint's analytic drawing in
//! `gradient-probe.cases.json`, and each slide's gradient diagnostics against the cases' list.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::path::PathBuf;

use serde_json::Value;
use support::{Compiled, compile_bytes, json};

/// The IR's ratio grid is 1e-5; the cases print six decimals.
const TOLERANCE: f64 = 2e-5;

/// The codes a gradient reports instead of drawing: the gradient itself, a gradient line, gradient text.
const GRADIENT_CODES: [&str; 3] = ["PX2105", "PX2106", "PX2211"];

fn probes() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../probes/out")
}

struct Probe {
    compiled: Compiled,
    ir: Value,
    cases: Vec<Value>,
}

fn probe() -> Probe {
    let bytes = std::fs::read(probes().join("gradient-probe.pptx")).unwrap();
    let compiled = compile_bytes(&bytes, "gradient-probe.pptx");
    assert!(compiled.errors().is_empty(), "{:?}", compiled.errors());
    let ir = json(compiled.presentation.as_ref().unwrap());
    let cases: Value =
        serde_json::from_slice(&std::fs::read(probes().join("gradient-probe.cases.json")).unwrap())
            .unwrap();
    let deck = cases["decks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|deck| deck["frontend"] == "pptx")
        .unwrap();
    Probe {
        compiled,
        ir,
        cases: deck["slides"].as_array().unwrap().clone(),
    }
}

fn slide<'a>(ir: &'a Value, id: &str) -> &'a Value {
    ir["slides"]
        .as_array()
        .unwrap()
        .iter()
        .find(|slide| slide["id"] == id)
        .unwrap_or_else(|| panic!("no slide {id}"))
}

fn named<'a>(elements: &'a Value, name: &str) -> Option<&'a Value> {
    elements.as_array()?.iter().find_map(|element| {
        if element["name"] == name {
            Some(element)
        } else {
            named(&element["children"], name)
        }
    })
}

fn num(value: &Value) -> f64 {
    value
        .as_f64()
        .unwrap_or_else(|| panic!("not a number: {value}"))
}

/// A point as the IR prints it, `{x, y}`.
fn point(value: &Value) -> (f64, f64) {
    (num(&value["x"]), num(&value["y"]))
}

/// A point as the cases print it, `[x, y]`.
fn pair(value: &Value) -> (f64, f64) {
    (num(&value[0]), num(&value[1]))
}

fn near(what: &str, actual: f64, expected: f64, problems: &mut Vec<String>) {
    if (actual - expected).abs() > TOLERANCE {
        problems.push(format!("{what}: {actual} ≠ {expected}"));
    }
}

fn near_point(what: &str, actual: (f64, f64), expected: (f64, f64), out: &mut Vec<String>) {
    near(&format!("{what}.x"), actual.0, expected.0, out);
    near(&format!("{what}.y"), actual.1, expected.1, out);
}

/// The ends of an angled cell gradient as fractions of a `w` × `h` cell: the CSS line through the centre that spans
/// the corners' projections in the cell's real proportions (unscaled DrawingML).
fn angled_ends(angle: f64, w: f64, h: f64) -> ((f64, f64), (f64, f64)) {
    let (s, c) = (
        slidock_math::sin(angle.to_radians()),
        slidock_math::cos(angle.to_radians()),
    );
    let half = (w * c.abs() + h * s.abs()) / 2.0;
    let (dx, dy) = (half * c / w, half * s / h);
    ((0.5 - dx, 0.5 - dy), (0.5 + dx, 0.5 + dy))
}

/// The differences between a resolved fill and PowerPoint's drawing of it on a `w` × `h` box.
fn compare(fill: &Value, expected: &Value, size: (f64, f64)) -> Vec<String> {
    let mut out = Vec::new();
    let kind = expected["kind"].as_str().unwrap();
    match kind {
        "linear" if fill.get("angle").is_some() => {
            let (from, to) = angled_ends(num(&fill["angle"]), size.0, size.1);
            near_point("from", from, pair(&expected["from"]), &mut out);
            near_point("to", to, pair(&expected["to"]), &mut out);
        }
        "linear" if fill["kind"] == "linear" => {
            near_point(
                "from",
                point(&fill["from"]),
                pair(&expected["from"]),
                &mut out,
            );
            near_point("to", point(&fill["to"]), pair(&expected["to"]), &mut out);
        }
        "radial" if fill["kind"] == "radial" => {
            let centre = point(&fill["center"]);
            near_point("center", centre, pair(&expected["center"]), &mut out);
            let radius = num(&fill["radius"]);
            near("radius", radius, num(&expected["radius"]), &mut out);
            let radius_y = fill.get("radiusY").map_or(radius, num);
            near("radiusY", radius_y, num(&expected["radiusY"]), &mut out);
            let focus = fill.get("focus").map_or(centre, point);
            near_point("focus", focus, pair(&expected["focus"]), &mut out);
        }
        "rect" if fill["kind"] == "rect" => {
            near_point(
                "origin",
                point(&fill["origin"]),
                pair(&expected["origin"]),
                &mut out,
            );
            near_point(
                "axisX",
                point(&fill["axisX"]),
                pair(&expected["axisX"]),
                &mut out,
            );
            near_point(
                "axisY",
                point(&fill["axisY"]),
                pair(&expected["axisY"]),
                &mut out,
            );
            for side in ["l", "t", "r", "b"] {
                near(
                    &format!("inner.{side}"),
                    num(&fill["inner"][side]),
                    num(&expected["inner"][side]),
                    &mut out,
                );
            }
        }
        _ => out.push(format!("drawn as {}, not {kind}", fill["kind"])),
    }
    out
}

fn turned_against_the_shape(object: &Value) -> bool {
    object["fill"]["rotWithShape"] == false
        && (num(&object["rot"]) != 0.0 || object["flipH"] == true || object["flipV"] == true)
}

/// The resolved fill and paint box of one case object, with PowerPoint's drawing of it.
fn drawn(slide: &Value, object: &Value) -> Vec<(String, Value, Value, (f64, f64))> {
    let name = object["element"].as_str().unwrap();
    if name == "background" {
        return vec![(
            name.to_owned(),
            slide["background"].clone(),
            object["powerpoint"].clone(),
            (960.0, 540.0),
        )];
    }
    if let Some(cells) = object["cells"].as_array() {
        let table = named(&slide["elements"], name).unwrap();
        return cells
            .iter()
            .filter(|cell| cell.get("powerpoint").is_some())
            .map(|cell| {
                let (row, column) = (cell["row"].as_u64().unwrap(), cell["col"].as_u64().unwrap());
                let fill = &table["rows"][row as usize]["cells"][column as usize]["fill"];
                (
                    format!("{name} cell {row},{column}"),
                    fill.clone(),
                    cell["powerpoint"].clone(),
                    (num(&cell["size"][0]), num(&cell["size"][1])),
                )
            })
            .collect();
    }
    let Some(expected) = object.get("powerpoint") else {
        return Vec::new();
    };
    let element = named(&slide["elements"], name).unwrap_or_else(|| panic!("no element {name}"));
    vec![(
        name.to_owned(),
        element["fill"].clone(),
        expected.clone(),
        (num(&object["box"][2]), num(&object["box"][3])),
    )]
}

#[test]
fn draws_every_probe_gradient_as_powerpoint_does_in_box_fractions() {
    let probe = probe();
    let mut problems = Vec::new();
    let mut compared = 0;
    for case in &probe.cases {
        let slide = slide(&probe.ir, case["row"].as_str().unwrap());
        for object in case["objects"].as_array().unwrap() {
            // The cases give PowerPoint's drawing in a box that turns with the shape; these turn back (below).
            if turned_against_the_shape(object) {
                continue;
            }
            for (name, fill, expected, size) in drawn(slide, object) {
                compared += 1;
                for problem in compare(&fill, &expected, size) {
                    problems.push(format!("{} {name}: {problem}", case["id"]));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    // 83 shapes (G43's 48 squares among them), 2 backgrounds and 3 cells.
    assert_eq!(compared, 88);
}

/// A lowered linear gradient's direction in the box's real proportions (degrees, clockwise from +x) and the ramp
/// parameter at its four corners.
fn real_direction_and_corners(fill: &Value, w: f64, h: f64) -> (f64, [f64; 4]) {
    let (from, to) = (point(&fill["from"]), point(&fill["to"]));
    let d = (to.0 - from.0, to.1 - from.1);
    let norm = d.0 * d.0 + d.1 * d.1;
    let t = |u: (f64, f64)| ((u.0 - from.0) * d.0 + (u.1 - from.1) * d.1) / norm;
    let angle = slidock_math::atan2(d.1 / h, d.0 / w).to_degrees();
    (
        angle,
        [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].map(t),
    )
}

#[test]
fn turns_a_gradient_that_does_not_rotate_with_its_shape_back_into_the_box() {
    let probe = probe();
    let mut problems = Vec::new();
    // G13: lin 0 on a box turned 30° stays horizontal on the page, so it runs at −30° in the box; G16: lin 45 on a
    // box flipped horizontally keeps 45° on the page, so it runs at 135° in the box. Each spans the box's corners.
    for (row, angle) in [("pp-268", -30.0), ("pp-271", 135.0)] {
        let fill = &named(&slide(&probe.ir, row)["elements"], "Gradient box").unwrap()["fill"];
        let (actual, corners) = real_direction_and_corners(fill, 600.0, 200.0);
        // The ends sit on the 1e-5 grid, which turns the line by up to about 0.001°.
        if (actual - angle).abs() > 0.01 {
            problems.push(format!("{row} angle: {actual} ≠ {angle}"));
        }
        let lowest = corners.iter().copied().fold(f64::INFINITY, f64::min);
        let highest = corners.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        near(
            &format!("{row} t at the first corner"),
            lowest,
            0.0,
            &mut problems,
        );
        near(
            &format!("{row} t at the last corner"),
            highest,
            1.0,
            &mut problems,
        );
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn reads_theme_fill_styles_with_the_references_colour() {
    let probe = probe();
    let elements = &slide(&probe.ir, "pp-295")["elements"];
    // fillRef 2: lin 270 scaled, bottom to top; fillRef 3: lin 90 unscaled, from the reference's colour to white.
    let scaled = &named(elements, "fillRef 2").unwrap()["fill"];
    let unscaled = &named(elements, "fillRef 3").unwrap()["fill"];
    let mut problems = Vec::new();
    near_point(
        "fillRef 2 from",
        point(&scaled["from"]),
        (0.5, 1.0),
        &mut problems,
    );
    near_point(
        "fillRef 2 to",
        point(&scaled["to"]),
        (0.5, 0.0),
        &mut problems,
    );
    near_point(
        "fillRef 3 from",
        point(&unscaled["from"]),
        (0.5, 0.0),
        &mut problems,
    );
    let first = &unscaled["stops"][0]["color"];
    let first = first.get("resolved").unwrap_or(first);
    for (channel, value) in [("r", 0x2F), ("g", 0x6F), ("b", 0xDF)] {
        near(
            &format!("fillRef 3 stop 0 {channel}"),
            num(&first[channel]),
            f64::from(value) / 255.0,
            &mut problems,
        );
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn reports_on_each_probe_slide_exactly_the_gradient_losses_its_case_lists() {
    let probe = probe();
    let mut problems = Vec::new();
    for case in &probe.cases {
        let row = case["row"].as_str().unwrap();
        let mut actual: Vec<&str> = probe
            .compiled
            .diagnostics
            .iter()
            .filter(|d| d.container.as_ref().is_some_and(|c| c.as_str() == row))
            .map(|d| d.code.code())
            .filter(|code| GRADIENT_CODES.contains(code))
            .collect();
        actual.sort_unstable();
        actual.dedup();
        let expected: Vec<&str> = case["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|code| code.as_str().unwrap())
            .filter(|code| GRADIENT_CODES.contains(code))
            .collect();
        if actual != expected {
            problems.push(format!("{}: {actual:?} ≠ {expected:?}", case["id"]));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
