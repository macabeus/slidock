//! Custom shapes outside the preset map drawn from their own enhanced geometry: LibreOffice's callouts and hand-drawn
//! shapes, as LibreOffice 25.2 draws them (its PDF of `probes/out/callout-probe.odp`, every outline within 0.11 pt).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::{Value, json};
use slidock_core_api::Limits;
use support::odp::{OdpSpec, PageSpec, odp};
use support::{assert_holds, lower, lower_with};

/// A 2 × 1 inch custom shape of this geometry, named "S".
fn shape(geometry: &str, equations: &str) -> Vec<u8> {
    let body = format!(
        r#"<draw:custom-shape draw:name="S" svg:x="1in" svg:y="1in" svg:width="2in" svg:height="1in"><draw:enhanced-geometry {geometry}>{equations}</draw:enhanced-geometry></draw:custom-shape>"#
    );
    odp(&OdpSpec::pages(vec![PageSpec::body(body)]))
}

fn equations(formulas: &[&str]) -> String {
    formulas
        .iter()
        .enumerate()
        .map(|(k, f)| format!(r#"<draw:equation draw:name="f{k}" draw:formula="{f}"/>"#))
        .collect()
}

fn geometry_of(bytes: &[u8]) -> Value {
    let lowered = lower(bytes);
    lowered.named(0, "S")["geometry"].clone()
}

/// The points a path's segments end on.
fn ends(path: &Value) -> Vec<(f64, f64)> {
    path["segments"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| {
            let s = s.as_array().unwrap();
            let n = s.len();
            (n >= 3).then(|| (s[n - 2].as_f64().unwrap(), s[n - 1].as_f64().unwrap()))
        })
        .collect()
}

fn near(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01
}

/// A rectangular callout whose tail leaves the side its tip lies beyond: the side's slot is the tip there and a point
/// on the edge elsewhere.
const SIDES: &[&str] = &[
    "$0 -10800",
    "$1 -10800",
    "abs(?f0 )-abs(?f1 )",
    "if(?f2 ,-1,-?f1 )",
    "if(?f2 ,?f0 ,-1)",
    "if(?f2 ,-1,?f1 )",
    "if(?f2 ,-?f0 ,-1)",
    "if(?f3 ,$0 ,8100)",
    "if(?f3 ,$1 ,0)",
    "if(?f4 ,$0 ,21600)",
    "if(?f4 ,$1 ,8100)",
    "if(?f5 ,$0 ,13500)",
    "if(?f5 ,$1 ,21600)",
    "if(?f6 ,$0 ,0)",
    "if(?f6 ,$1 ,13500)",
];
const RECT_PATH: &str = "M 0 0 L 5400 0 ?f7 ?f8 10800 0 21600 0 21600 5400 ?f9 ?f10 21600 10800 21600 21600 16200 21600 ?f11 ?f12 10800 21600 0 21600 0 16200 ?f13 ?f14 0 10800 Z N";

#[test]
fn draws_a_libreoffice_callout_with_its_tail_from_its_path_and_equations() {
    let geometry = geometry_of(&shape(
        &format!(
            r#"svg:viewBox="0 0 21600 21600" draw:type="rectangular-callout" draw:modifiers="-4000 12000" draw:enhanced-path="{RECT_PATH}""#
        ),
        &equations(SIDES),
    ));
    assert_holds(
        &geometry,
        json!({ "kind": "custom", "paths": [{ "w": 21600, "h": 21600, "fill": true, "stroke": true, "fillRule": "evenodd" }] }),
    );
    let points = ends(&geometry["paths"][0]);
    // The tail leaves the left side, between (0, 16200) and (0, 10800), for the tip; the other slots lie on the edges.
    let at = points
        .iter()
        .position(|p| near(*p, (0.0, 16200.0)))
        .unwrap();
    assert!(near(points[at + 1], (-4000.0, 12000.0)), "{points:?}");
    assert!(near(points[at + 2], (0.0, 10800.0)));
    assert!(points.contains(&(8100.0, 0.0)) && points.contains(&(21600.0, 8100.0)));
    // Above: the top slot is the tip.
    let above = geometry_of(&shape(
        &format!(
            r#"draw:type="rectangular-callout" draw:modifiers="9000 -6000" draw:enhanced-path="{RECT_PATH}""#
        ),
        &equations(SIDES),
    ));
    assert!(ends(&above["paths"][0]).contains(&(9000.0, -6000.0)));
    assert!(ends(&above["paths"][0]).contains(&(0.0, 13500.0)));
}

#[test]
fn ends_a_path_at_n_and_drops_its_fill_at_f_and_its_line_at_s() {
    let geometry = geometry_of(&shape(
        r#"draw:enhanced-path="M 0 0 L 21600 0 21600 21600 Z N M 0 0 L 100 100 F N M 5 5 L 6 6 7 5 Z S N""#,
        "",
    ));
    let flags: Vec<(bool, bool)> = geometry["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["fill"].as_bool().unwrap(), p["stroke"].as_bool().unwrap()))
        .collect();
    assert_eq!(flags, [(true, true), (false, true), (true, false)]);
}

/// `X` leaves horizontally, `Y` vertically, and each point after the first turns the other way.
#[test]
fn alternates_the_quarter_ellipses_of_x_and_y() {
    let geometry = geometry_of(&shape(
        r#"draw:enhanced-path="M 0 10800 X 10800 0 21600 10800 10800 21600 0 10800 Z N""#,
        "",
    ));
    let segments = geometry["paths"][0]["segments"].as_array().unwrap();
    // The first quarter leaves (0, 10800) horizontally: its first control point keeps the start's y.
    let first = segments[1].as_array().unwrap();
    assert_eq!(first[0], "C");
    assert!(
        (first[2].as_f64().unwrap() - 10800.0).abs() < 0.01,
        "{first:?}"
    );
    // The second leaves (10800, 0) vertically: its first control point keeps the start's x.
    let second = segments[2].as_array().unwrap();
    assert!(
        (second[1].as_f64().unwrap() - 10800.0).abs() < 0.01,
        "{second:?}"
    );
}

/// `T` runs clockwise on the page between the ellipse's points on the rays at its two angles (clockwise from 3
/// o'clock): 0 to 180 is the lower half, 180 to 0 the upper one.
#[test]
fn turns_an_angle_ellipse_clockwise_from_its_first_angle_to_its_second() {
    let lowest = |path: &str| {
        let geometry = geometry_of(&shape(&format!(r#"draw:enhanced-path="{path}""#), ""));
        let points = ends(&geometry["paths"][0]);
        (
            points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max),
            points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min),
        )
    };
    let (low, _) = lowest("M 10800 10800 T 10800 10800 10800 8000 0 180 Z N");
    assert!((low - 18800.0).abs() < 0.01, "{low}");
    let (_, high) = lowest("M 10800 10800 T 10800 10800 10800 8000 180 0 Z N");
    assert!((high - 2800.0).abs() < 0.01, "{high}");
}

/// `W` (clockwise) and `B` (counter-clockwise) between the same two radial points the other way round draw the same
/// arc; `A` joins it to the current point.
#[test]
fn draws_the_arc_commands_by_their_direction() {
    let tip = &[
        "atan2(5000,0)",
        "?f0 +0.21",
        "?f0 -0.21",
        "10800+10800*cos(?f1 )",
        "10800+10800*sin(?f1 )",
        "10800+10800*cos(?f2 )",
        "10800+10800*sin(?f2 )",
    ];
    let outline = |path: &str| {
        let geometry = geometry_of(&shape(
            &format!(r#"draw:enhanced-path="{path}""#),
            &equations(tip),
        ));
        let mut points = ends(&geometry["paths"][0]);
        points.sort_by(|a, b| a.partial_cmp(b).unwrap());
        points
    };
    let w = outline("W 0 0 21600 21600 ?f3 ?f4 ?f5 ?f6 L 10800 30000 Z N");
    let b = outline("B 0 0 21600 21600 ?f5 ?f6 ?f3 ?f4 L 10800 30000 Z N");
    assert_eq!(w.len(), b.len());
    for (p, q) in w.iter().zip(&b) {
        assert!(near(*p, *q), "{p:?} {q:?}");
    }
    // The arc goes the long way round, over the top, and leaves the bottom (21600) open for the tail.
    assert!(w.iter().any(|p| p.1 < 100.0));
    assert!(!w.iter().any(|p| p.1 > 21400.0 && p.1 < 21700.0), "{w:?}");
}

#[test]
fn reads_the_formula_functions_and_the_constants_of_the_box() {
    let geometry = geometry_of(&shape(
        r#"svg:viewBox="0 0 21600 21600" draw:enhanced-path="M ?f0 ?f1 L ?f2 ?f3 ?f4 ?f5 ?f6 ?f7 Z N""#,
        &equations(&[
            "min(width /4,8000)",
            "max(height /3,1000)",
            "sqrt(right *bottom )/2",
            "10800+8000*sin(pi /6)",
            "atan(1)*4/pi *21600",
            "tan(pi /4)*left +logwidth /logwidth *1000",
            "-?f5 +?f4 ",
            "-(2+3)*-2",
        ]),
    ));
    let points = ends(&geometry["paths"][0]);
    let expect = [
        (5400.0, 7200.0),
        (10800.0, 14800.0),
        (21600.0, 1000.0),
        (20600.0, 10.0),
    ];
    for (p, q) in points.iter().zip(expect) {
        assert!(near(*p, q), "{points:?}");
    }
}

#[test]
fn mirrors_a_drawn_shape_with_its_frame() {
    let lowered = lower(&shape(
        r#"draw:mirror-horizontal="true" draw:enhanced-path="M 0 0 L 21600 0 0 21600 Z N""#,
        "",
    ));
    assert_holds(
        lowered.named(0, "S"),
        json!({ "kind": "shape", "frame": { "flipH": true, "flipV": false }, "geometry": { "kind": "custom" } }),
    );
}

/// A shape whose geometry cannot be read stays a Foreign node, which says why; one with no path keeps its type alone.
#[test]
fn keeps_an_unreadable_geometry_as_a_foreign_node_with_the_reason() {
    let reason = |geometry: &str, formulas: &[&str]| {
        let lowered = lower(&shape(geometry, &equations(formulas)));
        let node = lowered.named(0, "S").clone();
        assert_eq!(node["kind"], "foreign", "{node:#}");
        node["payload"].clone()
    };
    assert_eq!(
        reason(
            r#"draw:type="round-callout" draw:enhanced-path="M ?f0 0 L 1 1 N""#,
            &["foo(1)"]
        ),
        json!({ "drawType": "round-callout", "reason": "the name \"foo\" is not read" })
    );
    assert_eq!(
        reason(
            r#"draw:enhanced-path="M ?f0 0 L 1 1 N""#,
            &["?f1 ", "?f0 +1"]
        )["reason"],
        "the equations refer to each other in a loop"
    );
    assert_eq!(
        reason(r#"draw:enhanced-path="M ?f9 0 L 1 1 N""#, &[])["reason"],
        "the equation \"f9\" is not defined"
    );
    assert_eq!(
        reason(r#"draw:enhanced-path="M 0 0 K 1 1 N""#, &[])["reason"],
        "the path command 'K' is not read"
    );
    assert_eq!(
        reason(r#"draw:enhanced-path="M ?f0 0 L 1 1 N""#, &["1/0"])["reason"],
        "a formula's value is not a number"
    );
    assert_eq!(
        reason(r#"draw:type="mso-spt100""#, &[]),
        json!({ "drawType": "mso-spt100" })
    );
}

#[test]
fn fails_only_the_page_whose_drawn_shape_passes_the_segment_limit() {
    let page = |path: &str| {
        PageSpec::body(format!(
            r#"<draw:custom-shape svg:width="1in" svg:height="1in"><draw:enhanced-geometry draw:enhanced-path="{path}"/></draw:custom-shape>"#
        ))
    };
    let bytes = odp(&OdpSpec::pages(vec![
        page("M 0 0 L 1 1 2 2 3 3 Z N"),
        page("M 0 0 L 1 1 Z N"),
    ]));
    let lowered = lower_with(
        &bytes,
        None,
        Limits {
            path_segments: 3,
            ..Limits::default()
        },
    );
    let failed: Vec<Value> = lowered.json["slides"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.get("failed").cloned().unwrap_or_default())
        .collect();
    assert_eq!(failed, [json!({ "code": "OD9901" }), json!(null)]);
}
