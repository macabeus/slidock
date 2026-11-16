//! Line ends (the rules of lower-geometry's line ends, at the geometry level), and the committed vectors: open preset
//! paths and synthetic paths (some with NaN coordinates) under every end shape, size and width, hashed.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use indexmap::IndexMap;
use serde_json::{Value, json};
use slidock_geometry::{ShapePath, StrokeEnds, WithLineEnds, preset_paths, with_line_ends};
use slidock_ir::geometry::{PresetName, Seg};
use slidock_ir::paint::LineEnd;
use support::{digest, num, paths_text, rounded, rows, segment};

fn end(value: Value) -> LineEnd {
    serde_json::from_value(value).unwrap()
}

fn preset(name: &str, w: f64, h: f64) -> Vec<ShapePath> {
    preset_paths(PresetName::parse(name).unwrap(), w, h, &IndexMap::new())
}

fn ends(
    paths: &[ShapePath],
    width: f64,
    head: Option<&LineEnd>,
    tail: Option<&LineEnd>,
) -> WithLineEnds {
    with_line_ends(paths, StrokeEnds { width, head, tail })
}

#[test]
fn adds_arrowheads_and_stops_the_line_under_a_body() {
    // DrawingML's head is the start: a small oval centred there; the tail a medium triangle at the end.
    let head = end(json!({ "type": "oval", "w": "sm", "len": "sm" }));
    let tail = end(json!({ "type": "triangle", "w": "med", "len": "med" }));
    let out = ends(
        &preset("straightConnector1", 240.0, 0.0),
        2.0,
        Some(&head),
        Some(&tail),
    );
    assert_eq!(
        rounded(&out.paths[0].segments),
        [Seg::M(0.0, 0.0), Seg::L(234.0, 0.0)]
    );
    let [oval, triangle] = out.ends.as_slice() else {
        panic!("{:?}", out.ends)
    };
    assert!(oval.fill && !oval.stroke);
    assert_eq!(rounded(&oval.segments)[0], Seg::M(-2.0, 0.0));
    assert_eq!(
        rounded(&triangle.segments),
        [
            Seg::M(240.0, 0.0),
            Seg::L(234.0, 3.0),
            Seg::L(234.0, -3.0),
            Seg::Z
        ]
    );
    assert!(triangle.fill && !triangle.stroke);
}

#[test]
fn places_a_path_end_at_its_reach_scaled_with_the_stroke() {
    // A 6-unit reach scaled by 0.4 + 0.6 × 3 cuts the line, 305.94 pt long, 13.2 pt from its end, and the line goes
    // on half the stroke (1.5 pt) under the head.
    let tail = end(json!({
        "type": "path",
        "segments": [["M", -4, 0], ["L", 0, 6], ["L", 4, 0], ["Z"]],
        "scaleWithWidth": true
    }));
    let out = ends(&preset("line", 300.0, 60.0), 3.0, None, Some(&tail));
    assert_eq!(
        rounded(&out.paths[0].segments),
        [
            Seg::M(0.0, 0.0),
            Seg::L(287.056, 57.411),
            Seg::L(288.527, 57.705)
        ]
    );
    assert!(rounded(&out.ends[0].segments).contains(&Seg::L(300.0, 60.0)));
    assert!(out.ends[0].fill && !out.ends[0].stroke);
}

/// Keynote's "simple arrow": 6 units long, met by the line at the middle of its base.
fn simple_arrow() -> LineEnd {
    end(json!({
        "type": "path",
        "segments": [["M", -3, 0], ["L", 0, 6], ["L", 3, 0], ["Z"]],
        "scaleWithWidth": true
    }))
}

fn close_to(a: [f64; 2], b: [f64; 2], tolerance: f64) -> bool {
    (a[0] - b[0]).abs() <= tolerance && (a[1] - b[1]).abs() <= tolerance
}

/// Each segment's end point.
fn ends_of(segments: &[Seg<f64>]) -> Vec<[f64; 2]> {
    segments
        .iter()
        .filter_map(|segment| match segment.coords().as_slice() {
            [.., x, y] => Some([*x, *y]),
            _ => None,
        })
        .collect()
}

#[test]
fn points_a_keynote_head_on_a_curve_along_the_chord_under_it_as_keynote_draws_it() {
    // retro-fidelity's slide 6 (kn-6284213): a curved connection line from (38.324, 0) through (0.777, 58.520) to
    // (19.609, 132.428), 7 pt wide, with the simple arrow at its end. Keynote 14.5's PDF of the slide (in the shape's
    // frame) draws the line, a cubic with the control points of the quadratic below raised, to (7.622, 107.561), then
    // straight on to (9.146, 110.717), and the head (-4.807, 113.557), (19.611, 132.428), (20.056, 101.572): 27.6 pt
    // long, its base on the line and its axis the chord from there to the tip.
    let (start, middle, tip) = (
        [38.324_24, 0.0],
        [0.776_549_34, 58.520_195],
        [19.609_104, 132.428_28],
    );
    let control = [
        2.0 * middle[0] - (start[0] + tip[0]) / 2.0,
        2.0 * middle[1] - (start[1] + tip[1]) / 2.0,
    ];
    let curve = ShapePath {
        segments: vec![
            Seg::M(start[0], start[1]),
            Seg::Q(control[0], control[1], tip[0], tip[1]),
        ],
        fill: false,
        stroke: true,
    };
    let out = ends(&[curve], 7.0, None, Some(&simple_arrow()));
    let [Seg::M(..), Seg::Q(cx, cy, x, y), Seg::L(on_x, on_y)] = out.paths[0].segments.as_slice()
    else {
        panic!("{:?}", out.paths[0].segments)
    };
    assert!(close_to([*x, *y], [7.622, 107.561], 0.01), "{x} {y}");
    assert!(
        close_to([*on_x, *on_y], [9.146, 110.717], 0.01),
        "{on_x} {on_y}"
    );
    // Keynote's cubic (1.380, 28.565), (-8.854, 64.416) is this quadratic's control raised.
    assert!(close_to([*cx, *cy], [-17.093, 42.848], 0.02), "{cx} {cy}");
    let head = ends_of(&out.ends[0].segments);
    for keynote in [[-4.807, 113.557], [19.611, 132.428], [20.056, 101.572]] {
        assert!(
            head.iter().any(|point| close_to(*point, keynote, 0.01)),
            "{keynote:?} not in {head:?}"
        );
    }
}

#[test]
fn sizes_a_keynote_head_by_the_stroke_and_takes_the_line_half_a_width_under_it() {
    // retro's kn-3809172: a straight line 171.87 pt long, 8 pt wide, with the simple arrow. Keynote 14.5's PDF draws
    // the head 31.2 pt long (6 units × (0.4 + 0.6 × 8)) and 31.2 pt wide, and the line to 144.669, 4 pt under it.
    let line = ShapePath {
        segments: vec![Seg::M(0.0, 0.0), Seg::L(171.87, 0.0)],
        fill: false,
        stroke: true,
    };
    let out = ends(&[line], 8.0, None, Some(&simple_arrow()));
    assert_eq!(
        ends_of(&rounded(&out.paths[0].segments)),
        [[0.0, 0.0], [140.67, 0.0], [144.67, 0.0]]
    );
    let head = ends_of(&rounded(&out.ends[0].segments));
    for keynote in [[140.669, 15.602], [171.869, 0.0], [140.669, -15.602]] {
        assert!(
            head.iter().any(|point| close_to(*point, keynote, 0.005)),
            "{keynote:?} not in {head:?}"
        );
    }
}

#[test]
fn points_a_preset_end_along_the_path_as_libreoffice_draws_it() {
    // The line-ends probe (`cargo xtask probes-lines`), 4 pt wide with a large triangle (20 pt by 20) at its end, as
    // LibreOffice 25.2 draws the ODP to PDF (in the drawing's frame): the triangle points from the path's point 20 pt
    // back along it to the tip, and the line stops 20 pt back along it.
    let triangle = end(json!({ "type": "triangle", "w": "lg", "len": "lg" }));
    let path = |segments: Vec<Seg<f64>>| ShapePath {
        segments,
        fill: false,
        stroke: true,
    };
    // "Short last segment arrow": 12 pt of the 20 are on the first segment, so the head leans 33.7°.
    let short = path(vec![
        Seg::M(0.0, 0.0),
        Seg::L(300.0, 0.0),
        Seg::L(300.0, 8.0),
    ]);
    let hooked = path(vec![
        Seg::M(0.0, 0.0),
        Seg::C(200.0, 0.0, 200.0, 120.0, 100.0, 120.0),
    ]);
    for (drawing, libreoffice) in [
        (
            short,
            [[300.027, 8.025], [277.833, 5.217], [288.973, -11.421]],
        ),
        (
            hooked,
            [[100.017, 120.028], [119.095, 108.348], [120.823, 128.275]],
        ),
    ] {
        let out = ends(&[drawing], 4.0, None, Some(&triangle));
        let head = ends_of(&out.ends[0].segments);
        for point in libreoffice {
            assert!(
                head.iter().any(|ours| close_to(*ours, point, 0.1)),
                "{point:?} not in {head:?}"
            );
        }
    }
}

#[test]
fn cuts_into_the_segment_before_when_the_head_is_longer_than_the_last() {
    // The last segment is 2 pt long and the head 10: the line stops 10 pt back along it, on the segment before, and
    // the head points from there to the tip.
    let triangle = end(json!({ "type": "triangle", "w": "lg", "len": "lg" }));
    let bent = ShapePath {
        segments: vec![Seg::M(0.0, 0.0), Seg::L(100.0, 0.0), Seg::L(100.0, 2.0)],
        fill: false,
        stroke: true,
    };
    let out = ends(&[bent], 2.0, None, Some(&triangle));
    assert_eq!(
        rounded(&out.paths[0].segments),
        [Seg::M(0.0, 0.0), Seg::L(92.0, 0.0)]
    );
    let head = ends_of(&rounded(&out.ends[0].segments));
    // The base's middle, 10 pt from the tip on the line from (92, 0).
    let base = [
        (head[1][0] + head[2][0]) / 2.0,
        (head[1][1] + head[2][1]) / 2.0,
    ];
    assert_eq!(head[0], [100.0, 2.0]);
    assert!(close_to(base, [90.299, -0.425], 0.002), "{base:?}");
}

#[test]
fn places_both_ends_on_the_whole_path_when_their_bodies_overlap() {
    // The line-ends probe's two-headed arrows, 4 pt wide with a large triangle (20 pt by 20) at each end, as
    // LibreOffice 25.2 draws them (ODP and PPTX to PDF, in the drawing's frame): each triangle is placed on the whole
    // path, not on what the other one's cut leaves of it, and the line, wholly under the two, is not drawn.
    let triangle = end(json!({ "type": "triangle", "w": "lg", "len": "lg" }));
    let path = |segments: Vec<Seg<f64>>| ShapePath {
        segments,
        fill: false,
        stroke: true,
    };
    // "Short bend, two heads" (18 pt long): each triangle points from the path's far end to its tip.
    let short = path(vec![Seg::M(0.0, 0.0), Seg::L(10.0, 0.0), Seg::L(10.0, 8.0)]);
    // "Bend under two heads" (30 pt long): the tail points from 20 pt back along the whole path, (10, 0).
    let under = path(vec![Seg::M(0.0, 0.0), Seg::L(24.0, 0.0), Seg::L(24.0, 6.0)]);
    for (drawing, libreoffice) in [
        (
            under,
            [
                [[-0.002, 0.005], [20.007, -10.002], [20.007, 10.007]],
                [[24.035, 6.042], [1.699, 7.289], [9.633, -11.081]],
            ],
        ),
        (
            short,
            [
                [[-0.002, -0.005], [21.88, 4.672], [9.379, 20.317]],
                [[10.032, 8.015], [-11.854, 3.339], [0.648, -12.31]],
            ],
        ),
    ] {
        let out = ends(&[drawing], 4.0, Some(&triangle), Some(&triangle));
        assert_eq!(out.ends.len(), 2, "{:?}", out.ends);
        for (ours, theirs) in out.ends.iter().zip(libreoffice) {
            let ours = ends_of(&ours.segments);
            for point in theirs {
                assert!(
                    ours.iter().any(|p| close_to(*p, point, 0.1)),
                    "{point:?} not in {ours:?}"
                );
            }
        }
        let drawn = &out.paths[0].segments;
        let at = ends_of(drawn);
        assert!(
            at.windows(2).all(|pair| close_to(pair[0], pair[1], 1e-9)),
            "a line is drawn: {drawn:?}"
        );
    }
    // Shorter than one end, presets and Keynote's alike, curved, bent or straight: both ends are drawn.
    let triangle = end(json!({ "type": "triangle", "w": "med", "len": "med" }));
    for name in [
        "bentConnector2",
        "bentConnector3",
        "curvedConnector3",
        "line",
    ] {
        let h = if name == "line" { 0.0 } else { 8.0 };
        for end in [&triangle, &simple_arrow()] {
            let out = ends(&preset(name, 10.0, h), 6.0, Some(end), Some(end));
            assert_eq!(out.ends.len(), 2, "{name} {end:?}");
        }
    }
}

fn synthetic(value: &Value) -> Vec<ShapePath> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|path| ShapePath {
            segments: path["segments"]
                .as_array()
                .unwrap()
                .iter()
                .map(segment)
                .collect(),
            fill: path["fill"].as_bool().unwrap(),
            stroke: path["stroke"].as_bool().unwrap(),
        })
        .collect()
}

#[test]
fn matches_the_committed_vectors() {
    let rows = rows(support::LINE_ENDS);
    let (inputs, cases) = rows.split_first().unwrap();
    let inputs: Vec<Vec<ShapePath>> = inputs.as_array().unwrap().iter().map(synthetic).collect();
    let mut misses = Vec::new();
    for case in cases {
        let input = &case[0];
        let paths = match input[0].as_str().unwrap() {
            "preset" => preset(input[1].as_str().unwrap(), num(&input[2]), num(&input[3])),
            _ => inputs[input[1].as_u64().unwrap() as usize].clone(),
        };
        let width = num(&case[1]);
        let head = (!case[2].is_null()).then(|| end(case[2].clone()));
        let tail = (!case[3].is_null()).then(|| end(case[3].clone()));
        let out = ends(&paths, width, head.as_ref(), tail.as_ref());
        let text = format!("{}#{}", paths_text(&out.paths), paths_text(&out.ends));
        if digest(&text) != case[4].as_str().unwrap()
            || out.ends.len() as u64 != case[5].as_u64().unwrap()
        {
            misses.push(format!(
                "{input} width {width} head {} tail {}",
                case[2], case[3]
            ));
        }
    }
    assert_eq!(misses, Vec::<String>::new());
    assert!(cases.len() > 800, "{}", cases.len());
}
