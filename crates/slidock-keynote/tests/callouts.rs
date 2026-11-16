//! Callouts as one outline, body and tail, as Keynote 14.5's `TSDCalloutPathSource` builds them (`paths/callout.rs`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::{Value, json};
use slidock_keynote::paths::{callout_tail_unread, shape_geometry};
use slidock_keynote::proto::tsd;
use support::json::encode;

type Point = (f64, f64);

fn source(callout: Value) -> tsd::PathSourceArchive {
    use prost::Message;
    tsd::PathSourceArchive::decode(
        encode(
            "TSD.PathSourceArchive",
            &json!({ "callout_path_source": callout }),
        )
        .as_slice(),
    )
    .unwrap()
}

/// The outline's segments in a box of the natural size, as JSON arrays.
fn outline(callout: Value, w: f64, h: f64) -> Vec<Vec<Value>> {
    let source = source(callout);
    let geometry = shape_geometry(Some(&source), w, h, 100_000)
        .unwrap()
        .unwrap();
    let json = serde_json::to_value(&geometry).unwrap();
    assert_eq!(json["kind"], "custom", "{json}");
    assert_eq!(json["paths"].as_array().unwrap().len(), 1);
    json["paths"][0]["segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_array().unwrap().clone())
        .collect()
}

fn end(segment: &[Value]) -> Option<Point> {
    let n = segment.len();
    (n >= 3).then(|| {
        (
            segment[n - 2].as_f64().unwrap(),
            segment[n - 1].as_f64().unwrap(),
        )
    })
}

fn sq(x: f64) -> f64 {
    x * x
}

fn near(a: Point, b: Point) -> bool {
    (a.0 - b.0).abs() < 0.002 && (a.1 - b.1).abs() < 0.002
}

/// The points before and after the tip, the tail's sides' feet on the body.
fn feet(segments: &[Vec<Value>], tip: Point) -> (Point, Point) {
    let at = segments
        .iter()
        .position(|s| end(s).is_some_and(|p| near(p, tip)))
        .unwrap_or_else(|| panic!("no tip in {segments:?}"));
    assert_eq!(segments[at][0], "L");
    let before = end(&segments[at - 1]).unwrap();
    // The outline closes from the tip back to its start.
    assert_eq!(segments[at + 1][0], "Z");
    let after = end(&segments[0]).unwrap();
    (before, after)
}

fn sorted(a: Point, b: Point) -> (Point, Point) {
    if (a.0, a.1) <= (b.0, b.1) {
        (a, b)
    } else {
        (b, a)
    }
}

/// A tip below the straight bottom edge, between the corners: the tail aims up the tip's own column, and its sides
/// meet the edge `tail_size` either side of it.
#[test]
fn joins_a_tail_to_the_edge_under_its_tip_at_its_half_width_either_side() {
    let segments = outline(
        json!({ "natural_size": { "width": 200, "height": 100 }, "tail_position": { "x": 60, "y": 140 },
                "tail_size": 8, "corner_radius": 20, "center_tail": false }),
        200.0,
        100.0,
    );
    let (a, b) = sorted(
        feet(&segments, (60.0, 140.0)).0,
        feet(&segments, (60.0, 140.0)).1,
    );
    assert!(
        near(a, (52.0, 100.0)) && near(b, (68.0, 100.0)),
        "{a:?} {b:?}"
    );
    // The body keeps its four rounded corners: one closed outline, no segment through the tail's base.
    assert_eq!(
        segments.iter().filter(|s| s[0] == "C").count(),
        4,
        "{segments:?}"
    );
}

/// `center_tail` (true when unset) aims the tail at the body's centre; otherwise past the corners' arcs on both axes
/// it aims at the nearest corner's centre, and its base is on that arc.
#[test]
fn aims_the_tail_at_the_centre_or_at_the_nearest_corners_arc() {
    let tip = (-30.0, 130.0);
    let aimed = |center: Option<bool>| {
        let mut callout = json!({ "natural_size": { "width": 200, "height": 100 }, "tail_position": { "x": -30, "y": 130 },
                                  "tail_size": 6, "corner_radius": 30 });
        if let Some(center) = center {
            callout["center_tail"] = json!(center);
        }
        let segments = outline(callout, 200.0, 100.0);
        feet(&segments, tip)
    };
    let (a, b) = aimed(Some(false));
    // Both feet on the bottom-left corner's arc, centre (30, 70), radius 30.
    for p in [a, b] {
        let r = (sq(p.0 - 30.0) + sq(p.1 - 70.0)).sqrt();
        assert!((r - 30.0).abs() < 0.02, "{p:?} {r}");
    }
    // Square to the axis from the tip to the arc's centre, 6 either side where the axis meets the arc.
    let axis = (
        (30.0 - tip.0) / (sq(60.0) + sq(60.0)).sqrt(),
        (70.0 - tip.1) / (sq(60.0) + sq(60.0)).sqrt(),
    );
    let base = (30.0 - 30.0 * axis.0, 70.0 - 30.0 * axis.1);
    let distance = (sq(base.0 - tip.0) + sq(base.1 - tip.1)).sqrt();
    for p in [a, b] {
        let v = (p.0 - tip.0, p.1 - tip.1);
        let across = (axis.0 * v.1 - axis.1 * v.0) / (axis.0 * v.0 + axis.1 * v.1) * distance;
        assert!((across.abs() - 6.0).abs() < 0.01, "{p:?} {across}");
    }
    // Past the top-right corner's arc on both axes: that corner's centre (170, 30).
    let segments = outline(
        json!({ "natural_size": { "width": 200, "height": 100 }, "tail_position": { "x": 230, "y": -30 },
                "tail_size": 6, "corner_radius": 30, "center_tail": false }),
        200.0,
        100.0,
    );
    let (c, d) = feet(&segments, (230.0, -30.0));
    for p in [c, d] {
        let r = (sq(p.0 - 170.0) + sq(p.1 - 30.0)).sqrt();
        assert!((r - 30.0).abs() < 0.02, "{p:?} {r}");
    }
    let centred = aimed(None);
    assert_eq!(centred, aimed(Some(true)));
    assert!(!near(centred.0, a) && !near(centred.0, b));
}

/// The tail's half-width is at most 90 % of the body's narrower reach seen from the tip.
#[test]
fn limits_the_tail_to_nine_tenths_of_the_bodys_reach() {
    let tip = (100.0, 200.0);
    let segments = outline(
        json!({ "natural_size": { "width": 200, "height": 100 }, "tail_position": { "x": 100, "y": 200 },
                "tail_size": 500, "corner_radius": 0, "center_tail": false }),
        200.0,
        100.0,
    );
    let (a, b) = feet(&segments, tip);
    let (a, b) = sorted(a, b);
    // Seen from (100, 200) the corners (0, 0) and (200, 0) reach 50 at the base (100, 100) and (0, 100) reaches 100:
    // 90 % of 100.
    assert!(
        near(a, (10.0, 100.0)) && near(b, (190.0, 100.0)),
        "{a:?} {b:?}"
    );
    // A negative size is limited by its magnitude.
    let negative = outline(
        json!({ "natural_size": { "width": 200, "height": 100 }, "tail_position": { "x": 100, "y": 200 },
                "tail_size": -500, "corner_radius": 0, "center_tail": false }),
        200.0,
        100.0,
    );
    assert_eq!(negative, segments);
}

/// A negative tail size is its magnitude; a corner radius of half the shorter side or more makes the body the
/// ellipse of the box, and so does an unset one.
#[test]
fn reads_a_negative_tail_size_and_a_radius_past_half_the_side_as_keynote_does() {
    let tip = (100.0, 160.0);
    let callout = |size: f64| {
        outline(
            json!({ "natural_size": { "width": 200, "height": 100 }, "tail_position": { "x": 100, "y": 160 },
                    "tail_size": size, "corner_radius": 50, "center_tail": false }),
            200.0,
            100.0,
        )
    };
    let (positive, negative) = (callout(10.0), callout(-10.0));
    assert_eq!(positive, negative);
    let (a, b) = sorted(feet(&positive, tip).0, feet(&positive, tip).1);
    // On the ellipse x²/100² + y²/50² = 1 about (100, 50).
    for p in [a, b] {
        let e = sq((p.0 - 100.0) / 100.0) + sq((p.1 - 50.0) / 50.0);
        assert!((e - 1.0).abs() < 0.001, "{p:?} {e}");
    }
    assert!(a.0 < 100.0 && b.0 > 100.0);
    let unset = outline(
        json!({ "natural_size": { "width": 200, "height": 100 }, "tail_position": { "x": 100, "y": 50 },
                "tail_size": 10 }),
        200.0,
        100.0,
    );
    assert_eq!(
        unset[1],
        json!(["C", 155.23, 0.0, 200.0, 22.385, 200.0, 50.0])
            .as_array()
            .unwrap()
            .clone()
    );
}

/// A tip inside the body draws the body alone, as Keynote does; a tail wider than the body seen from its tip is not
/// drawn and says why.
#[test]
fn draws_the_body_alone_for_a_tip_inside_it_and_reports_a_tail_it_cannot_place() {
    let inside = outline(
        json!({ "natural_size": { "width": 200, "height": 100 }, "tail_position": { "x": 50, "y": 50 },
                "tail_size": 8, "corner_radius": 20, "center_tail": false }),
        200.0,
        100.0,
    );
    assert!(inside.iter().all(|s| {
        end(s).is_none_or(|p| (0.0..=200.0).contains(&p.0) && (0.0..=100.0).contains(&p.1))
    }));
    let inside = source(
        json!({ "natural_size": { "width": 200, "height": 100 }, "tail_position": { "x": 50, "y": 50 },
                               "tail_size": 8, "corner_radius": 20, "center_tail": false }),
    );
    assert_eq!(callout_tail_unread(Some(&inside), 200.0, 100.0), None);
    let tiny = source(
        json!({ "natural_size": { "width": 4, "height": 4 }, "tail_position": { "x": 20, "y": 2 },
                             "tail_size": 5, "corner_radius": 0, "center_tail": false }),
    );
    assert_eq!(
        callout_tail_unread(Some(&tiny), 4.0, 4.0).as_deref(),
        Some("the tail is wider than the body seen from its tip")
    );
    let drawn = source(
        json!({ "natural_size": { "width": 200, "height": 100 }, "tail_position": { "x": 60, "y": 140 },
                              "tail_size": 8, "corner_radius": 20, "center_tail": false }),
    );
    assert_eq!(callout_tail_unread(Some(&drawn), 200.0, 100.0), None);
}

/// The outline is drawn at the natural size and stretched to the frame with the rest of the shape.
#[test]
fn stretches_the_natural_size_to_the_frame() {
    let natural = outline(
        json!({ "natural_size": { "width": 200, "height": 100 }, "tail_position": { "x": 60, "y": 140 },
                "tail_size": 8, "corner_radius": 20, "center_tail": false }),
        200.0,
        100.0,
    );
    let doubled = outline(
        json!({ "natural_size": { "width": 200, "height": 100 }, "tail_position": { "x": 60, "y": 140 },
                "tail_size": 8, "corner_radius": 20, "center_tail": false }),
        400.0,
        50.0,
    );
    let (a, _) = feet(&doubled, (120.0, 70.0));
    let (n, _) = feet(&natural, (60.0, 140.0));
    assert!(near(a, (n.0 * 2.0, n.1 / 2.0)), "{a:?} {n:?}");
}
