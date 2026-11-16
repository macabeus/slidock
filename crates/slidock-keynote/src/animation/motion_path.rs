//! An action build's motion path (`BuildAttributesArchive.action_motionPathSource`, a `TSD.PathSourceArchive`) as a
//! tier-2 translate track: the path's points relative to its first, timed by distance along the path. Keynote moves
//! the object from where it sits, so the first point is the static position. Keynote stores the path editable
//! (`editable_bezier_path_source`), read as a shape's editable outline is ([`paths::editable_pieces`]) but never
//! fitted to a box; a plain bezier path source wins when both are stored. Its coordinates are the slide's, as the
//! element frames' are: points, played in slide pixels. The path is untrusted: its elements are bounded, and a point
//! it plays that is absent, lacks a coordinate, is not finite or leaves the slide's coordinate range fails the whole
//! path, as does a second subpath; the step then plays its fallback. `motionPathString`, an undocumented string form, is not parsed: a path stored only as a string plays
//! the fallback.

use slidock_ir::effects::{BuildEffect, Frame, Keyframes, Offset, Track, limits};
use slidock_ir::units::{Ms, Px, UnitRatio, pt_to_px};

use crate::num::wide;
use crate::paths::{self, Piece, Point};
use crate::proto::{kn, tsd, tsp};

/// `TSP.Path.ElementType`.
pub const MOVE_TO: i32 = 1;
pub const LINE_TO: i32 = 2;
pub const QUAD_CURVE_TO: i32 = 3;
pub const CURVE_TO: i32 = 4;
pub const CLOSE: i32 = 5;

/// Points sampled along each curve.
const CURVE_SAMPLES: usize = 8;

/// A point the path plays lacks a coordinate.
const MISSING_POINT: &str =
    "the motion path lacks a point or a coordinate it plays; the fallback plays";

/// How Keynote moves between subpaths is not known.
const SUBPATHS: &str = "the motion path has several subpaths, and how Keynote moves between them is not known; the fallback plays";

/// The largest coordinate a path may hold, in points (the IR's point range).
const COORDINATE: f64 = 100_000.0;

/// A stored point, both coordinates present.
fn read(p: Option<&tsp::Point>) -> Result<Point, &'static str> {
    p.and_then(|p| wide(p.x).zip(wide(p.y)))
        .ok_or(MISSING_POINT)
}

fn checked((x, y): Point) -> Result<Point, &'static str> {
    if x.is_finite() && y.is_finite() && x.abs() <= COORDINATE && y.abs() <= COORDINATE {
        Ok((x, y))
    } else {
        Err("the motion path leaves the slide's coordinates; the fallback plays")
    }
}

fn cubic(p0: Point, p1: Point, p2: Point, p3: Point, t: f64) -> Point {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (
        a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
        a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
    )
}

fn quadratic(p0: Point, p1: Point, p2: Point, t: f64) -> Point {
    let u = 1.0 - t;
    let (a, b, c) = (u * u, 2.0 * u * t, t * t);
    (
        a * p0.0 + b * p1.0 + c * p2.0,
        a * p0.1 + b * p1.1 + c * p2.1,
    )
}

/// A stored motion path's pieces.
enum Stored<'a> {
    Path(&'a tsp::Path),
    Editable(&'a tsd::EditableBezierPathSourceArchive),
}

impl Stored<'_> {
    /// The path's pieces, each element as it is stored: a path that does not start with a move, or holds an element
    /// with the wrong number of points, fails.
    fn pieces(&self) -> Result<Vec<Piece>, &'static str> {
        const UNREADABLE: &str =
            "the motion path holds an element it cannot read; the fallback plays";
        const TOO_LONG: &str = "the motion path has too many segments; the fallback plays";
        match self {
            Stored::Path(path) => {
                if path.elements.len() > limits::PATH_SEGMENTS {
                    return Err(TOO_LONG);
                }
                let mut pieces = Vec::with_capacity(path.elements.len());
                for element in &path.elements {
                    let points = element
                        .points
                        .iter()
                        .map(|p| read(Some(p)))
                        .collect::<Result<Vec<Point>, _>>()?;
                    pieces.push(match (element.r#type, points.as_slice()) {
                        (Some(MOVE_TO), [p]) => Piece::M(*p),
                        (Some(LINE_TO), [p]) => Piece::L(*p),
                        (Some(QUAD_CURVE_TO), [c, p]) => Piece::Q(*c, *p),
                        (Some(CURVE_TO), [c1, c2, p]) => Piece::C(*c1, *c2, *p),
                        (Some(CLOSE), _) => Piece::Z,
                        _ => return Err(UNREADABLE),
                    });
                }
                Ok(pieces)
            }
            Stored::Editable(source) => {
                // A move per subpath, a cubic per node and a close.
                let pieces = source.subpaths.iter().fold(0_usize, |n, s| {
                    n.saturating_add(s.nodes.len()).saturating_add(2)
                });
                if pieces > limits::PATH_SEGMENTS {
                    return Err(TOO_LONG);
                }
                paths::editable_pieces(source, read)
            }
        }
    }

    fn has_curves(&self) -> bool {
        self.pieces().is_ok_and(|pieces| {
            let mut from = None;
            pieces.iter().any(|piece| {
                let curved = match (*piece, from) {
                    (Piece::Q(..), _) => true,
                    (Piece::C(c1, c2, p), Some(from)) => c1 != from || c2 != p,
                    _ => false,
                };
                from = piece.last().or(from);
                curved
            })
        })
    }
}

/// The points the path passes through, curves sampled, in order: a cubic whose handles sit on its ends is a line,
/// and a close returns to its subpath's first point. A path is one subpath: a second move fails, as Keynote's way
/// between subpaths is not known.
fn polyline(pieces: &[Piece]) -> Result<Vec<Point>, &'static str> {
    let mut out: Vec<Point> = Vec::new();
    let mut start: Option<Point> = None;
    for piece in pieces {
        let current = out.last().copied();
        match (*piece, current) {
            (Piece::M(_), Some(_)) => return Err(SUBPATHS),
            (Piece::M(p), None) => {
                let p = checked(p)?;
                start = Some(p);
                out.push(p);
            }
            (Piece::L(p), Some(_)) => out.push(checked(p)?),
            (Piece::Q(c, p), Some(from)) => {
                let (c, p) = (checked(c)?, checked(p)?);
                for k in 1..=CURVE_SAMPLES {
                    out.push(quadratic(from, c, p, k as f64 / CURVE_SAMPLES as f64));
                }
            }
            (Piece::C(c1, c2, p), Some(from)) => {
                let (c1, c2, p) = (checked(c1)?, checked(c2)?, checked(p)?);
                if c1 == from && c2 == p {
                    out.push(p);
                } else {
                    for k in 1..=CURVE_SAMPLES {
                        out.push(cubic(from, c1, c2, p, k as f64 / CURVE_SAMPLES as f64));
                    }
                }
            }
            (Piece::Z, Some(last)) => {
                if let Some(start) = start
                    && start != last
                {
                    out.push(start);
                }
            }
            _ => return Err("the motion path holds an element it cannot read; the fallback plays"),
        }
    }
    Ok(out)
}

/// At most `n` points, evenly picked by index, keeping the first and the last.
fn thinned(points: Vec<Point>, n: usize) -> Vec<Point> {
    if points.len() <= n || n < 2 {
        return points;
    }
    let last = points.len() - 1;
    (0..n)
        .filter_map(|k| points.get(k * last / (n - 1)).copied())
        .collect()
}

fn stored(attributes: &kn::BuildAttributesArchive) -> Option<Stored<'_>> {
    let source = attributes.action_motion_path_source.as_ref()?;
    if let Some(path) = source
        .bezier_path_source
        .as_ref()
        .and_then(|bezier| bezier.path.as_ref())
    {
        return Some(Stored::Path(path));
    }
    source
        .editable_bezier_path_source
        .as_ref()
        .map(Stored::Editable)
}

/// Whether the stored motion path holds a curve segment.
pub fn has_curves(attributes: &kn::BuildAttributesArchive) -> bool {
    stored(attributes).is_some_and(|path| path.has_curves())
}

/// The keyframes of an action's motion path over `duration`, or why it cannot play.
pub fn keyframes(
    attributes: &kn::BuildAttributesArchive,
    duration: Ms,
) -> Result<BuildEffect, &'static str> {
    let source = attributes.action_motion_path_source.as_ref();
    let Some(path) = stored(attributes) else {
        return Err(if attributes.motion_path_string.is_some() {
            "the motion path is stored only as a string, which is not read; the fallback plays"
        } else {
            "the motion path is not stored as a path; the fallback plays"
        });
    };
    let flip_x = source.and_then(|s| s.horizontal_flip) == Some(true);
    let flip_y = source.and_then(|s| s.vertical_flip) == Some(true);
    let points = thinned(polyline(&path.pieces()?)?, limits::KEYFRAMES);
    let Some(&(x0, y0)) = points.first() else {
        return Err("the motion path is empty; the fallback plays");
    };
    let mut lengths = Vec::with_capacity(points.len());
    let mut total = 0.0;
    let mut previous = (x0, y0);
    for &p in &points {
        total += slidock_math::hypot(p.0 - previous.0, p.1 - previous.1);
        lengths.push(total);
        previous = p;
    }
    if total.is_nan() || total <= 0.0 {
        return Err("the motion path does not move; the fallback plays");
    }
    let sign = |flip: bool| if flip { -1.0 } else { 1.0 };
    let mut frames: Vec<Frame<Offset>> = Vec::with_capacity(points.len());
    for (&(x, y), &length) in points.iter().zip(&lengths) {
        let px = |v: f64| Px::quantise(pt_to_px(v)).map_err(|_| "the motion path is out of range");
        let offset =
            UnitRatio::quantise(length / total).map_err(|_| "the motion path is out of range")?;
        if frames
            .last()
            .is_some_and(|last| last.offset == offset && offset.steps() != 0)
        {
            continue;
        }
        frames.push(Frame {
            offset,
            value: Offset {
                x: px(sign(flip_x) * (x - x0))?,
                y: px(sign(flip_y) * (y - y0))?,
            },
        });
    }
    if let Some(last) = frames.last_mut() {
        last.offset = UnitRatio::from_steps(100_000);
    }
    Ok(BuildEffect::Behaviors(vec![Keyframes::Translate(Track {
        begin_ms: Ms(0),
        duration_ms: duration,
        easing: None,
        frames,
    })]))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;

    fn element(ty: i32, points: &[(f32, f32)]) -> tsp::path::Element {
        tsp::path::Element {
            r#type: Some(ty),
            points: points
                .iter()
                .map(|&(x, y)| tsp::Point {
                    x: Some(x),
                    y: Some(y),
                })
                .collect(),
        }
    }

    fn attributes(elements: Vec<tsp::path::Element>) -> kn::BuildAttributesArchive {
        kn::BuildAttributesArchive {
            action_motion_path_source: Some(tsd::PathSourceArchive {
                bezier_path_source: Some(tsd::BezierPathSourceArchive {
                    path: Some(tsp::Path { elements }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    type Node = ((f32, f32), (f32, f32), (f32, f32));

    fn tsp_point((x, y): (f32, f32)) -> Option<tsp::Point> {
        Some(tsp::Point {
            x: Some(x),
            y: Some(y),
        })
    }

    /// An editable path of one subpath: each node as (in handle, node, out handle).
    fn editable(nodes: &[Node], closed: bool) -> tsd::EditableBezierPathSourceArchive {
        use tsd::editable_bezier_path_source_archive::{Node, Subpath};
        tsd::EditableBezierPathSourceArchive {
            subpaths: vec![Subpath {
                nodes: nodes
                    .iter()
                    .map(|&(i, p, o)| Node {
                        in_control_point: tsp_point(i),
                        node_point: tsp_point(p),
                        out_control_point: tsp_point(o),
                        r#type: Some(1),
                    })
                    .collect(),
                closed: Some(closed),
            }],
            ..Default::default()
        }
    }

    fn with_editable(source: tsd::EditableBezierPathSourceArchive) -> kn::BuildAttributesArchive {
        kn::BuildAttributesArchive {
            action_motion_path_source: Some(tsd::PathSourceArchive {
                editable_bezier_path_source: Some(source),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn sharp(x: f32, y: f32) -> Node {
        ((x, y), (x, y), (x, y))
    }

    /// The track's frames as (offset, x, y).
    fn frames(attributes: &kn::BuildAttributesArchive) -> Vec<(f64, f64, f64)> {
        let BuildEffect::Behaviors(tracks) = keyframes(attributes, Ms(500)).unwrap() else {
            panic!()
        };
        let Keyframes::Translate(track) = &tracks[0] else {
            panic!()
        };
        assert_eq!((track.begin_ms, track.duration_ms), (Ms(0), Ms(500)));
        assert_eq!(track.easing, None);
        track
            .frames
            .iter()
            .map(|f| (f.offset.to_f64(), f.value.x.to_f64(), f.value.y.to_f64()))
            .collect()
    }

    #[test]
    fn an_editable_path_plays_its_nodes_relative_to_the_first_and_closes_by_a_line() {
        // Keynote's own form (field 8): sharp nodes are a polygon, played from the first node, the way back to it
        // a straight line; each leg's share of the time is its share of the length.
        let square = with_editable(editable(
            &[
                sharp(30.0, 60.0),
                sharp(60.0, 60.0),
                sharp(60.0, 90.0),
                sharp(30.0, 90.0),
            ],
            true,
        ));
        assert_eq!(
            frames(&square),
            [
                (0.0, 0.0, 0.0),
                (0.25, 40.0, 0.0),
                (0.5, 40.0, 40.0),
                (0.75, 0.0, 40.0),
                (1.0, 0.0, 0.0),
            ]
        );
        assert!(!has_curves(&square));
        let open = with_editable(editable(&[sharp(0.0, 0.0), sharp(0.0, 135.0)], false));
        assert_eq!(frames(&open), [(0.0, 0.0, 0.0), (1.0, 0.0, 180.0)]);
    }

    #[test]
    fn an_editable_curve_plays_the_cubic_from_each_out_handle_to_the_next_in_handle() {
        // Slide 65 of the build movies: a sharp node, a smooth one and a bezier end.
        let curve = with_editable(editable(
            &[
                ((0.0, 0.0), (0.0, 0.0), (48.0, 0.0)),
                ((96.0, 13.5), (138.0, 37.125), (180.0, 60.75)),
                ((216.0, 94.5), (240.0, 135.0), (240.0, 135.0)),
            ],
            false,
        ));
        assert!(has_curves(&curve));
        let frames = frames(&curve);
        assert_eq!(frames.len(), 2 * CURVE_SAMPLES + 1);
        let px = |p: Point| (pt_to_px(p.0), pt_to_px(p.1));
        let at = |k: usize| (frames[k].1, frames[k].2);
        let close = |a: Point, b: Point| (a.0 - b.0).abs() < 0.001 && (a.1 - b.1).abs() < 0.001;
        let first = cubic((0.0, 0.0), (48.0, 0.0), (96.0, 13.5), (138.0, 37.125), 0.5);
        assert!(close(at(CURVE_SAMPLES / 2), px(first)));
        assert_eq!(at(CURVE_SAMPLES), (184.0, 49.5));
        let second = cubic(
            (138.0, 37.125),
            (180.0, 60.75),
            (216.0, 94.5),
            (240.0, 135.0),
            0.5,
        );
        assert!(close(at(CURVE_SAMPLES * 3 / 2), px(second)));
        assert_eq!(at(2 * CURVE_SAMPLES), (320.0, 180.0));
    }

    #[test]
    fn a_bezier_path_source_wins_over_an_editable_one() {
        let mut both = attributes(vec![
            element(MOVE_TO, &[(0.0, 0.0)]),
            element(LINE_TO, &[(30.0, 0.0)]),
        ]);
        if let Some(source) = both.action_motion_path_source.as_mut() {
            source.editable_bezier_path_source =
                Some(editable(&[sharp(0.0, 0.0), sharp(0.0, 30.0)], false));
        }
        assert_eq!(frames(&both), [(0.0, 0.0, 0.0), (1.0, 40.0, 0.0)]);
    }

    #[test]
    fn hostile_editable_paths_fail_closed() {
        let nan = with_editable(editable(&[sharp(0.0, 0.0), sharp(f32::NAN, 1.0)], false));
        assert!(keyframes(&nan, Ms(1)).is_err());
        let far = with_editable(editable(&[sharp(0.0, 0.0), sharp(1e9, 1.0)], false));
        assert!(keyframes(&far, Ms(1)).is_err());
        let still = with_editable(editable(&[sharp(3.0, 3.0)], true));
        assert!(keyframes(&still, Ms(1)).is_err());
        let empty = with_editable(tsd::EditableBezierPathSourceArchive::default());
        assert!(keyframes(&empty, Ms(1)).is_err());
        let nodes: Vec<Node> = (0..limits::PATH_SEGMENTS)
            .map(|k| sharp(k as f32, 0.0))
            .collect();
        assert_eq!(
            keyframes(&with_editable(editable(&nodes, false)), Ms(1)),
            Err("the motion path has too many segments; the fallback plays")
        );
    }

    #[test]
    fn an_editable_path_missing_a_point_it_plays_fails_closed() {
        let mut no_handles = editable(&[sharp(100.0, 100.0), sharp(200.0, 100.0)], false);
        for node in &mut no_handles.subpaths[0].nodes {
            node.in_control_point = None;
            node.out_control_point = None;
        }
        let no_handles = with_editable(no_handles);
        assert_eq!(keyframes(&no_handles, Ms(1)), Err(MISSING_POINT));
        assert!(!has_curves(&no_handles));
        let mut no_node = editable(&[sharp(100.0, 100.0), sharp(200.0, 100.0)], false);
        no_node.subpaths[0].nodes[0].node_point = None;
        assert_eq!(
            keyframes(&with_editable(no_node), Ms(1)),
            Err(MISSING_POINT)
        );
        let mut no_y = editable(&[sharp(100.0, 100.0), sharp(200.0, 100.0)], false);
        no_y.subpaths[0].nodes[1].in_control_point = Some(tsp::Point {
            x: Some(200.0),
            y: None,
        });
        assert_eq!(keyframes(&with_editable(no_y), Ms(1)), Err(MISSING_POINT));
        // The handles an open path does not play (the first node's in, the last node's out) may be absent.
        let mut ends = editable(&[sharp(100.0, 100.0), sharp(130.0, 100.0)], false);
        ends.subpaths[0].nodes[0].in_control_point = None;
        ends.subpaths[0].nodes[1].out_control_point = None;
        assert_eq!(
            frames(&with_editable(ends)),
            [(0.0, 0.0, 0.0), (1.0, 40.0, 0.0)]
        );
    }

    #[test]
    fn a_stored_point_missing_a_coordinate_fails_closed() {
        let mut path = attributes(vec![
            element(MOVE_TO, &[(0.0, 0.0)]),
            element(LINE_TO, &[(30.0, 0.0)]),
        ]);
        if let Some(element) = path
            .action_motion_path_source
            .as_mut()
            .and_then(|s| s.bezier_path_source.as_mut())
            .and_then(|b| b.path.as_mut())
            .and_then(|p| p.elements.get_mut(1))
        {
            element.points[0].y = None;
        }
        assert_eq!(keyframes(&path, Ms(1)), Err(MISSING_POINT));
    }

    #[test]
    fn a_path_of_several_subpaths_fails_closed() {
        let mut two = editable(&[sharp(0.0, 0.0), sharp(30.0, 0.0)], false);
        let second = editable(&[sharp(0.0, 90.0), sharp(30.0, 90.0)], false);
        two.subpaths.extend(second.subpaths);
        assert_eq!(keyframes(&with_editable(two), Ms(1)), Err(SUBPATHS));
        let stored = attributes(vec![
            element(MOVE_TO, &[(0.0, 0.0)]),
            element(LINE_TO, &[(30.0, 0.0)]),
            element(MOVE_TO, &[(0.0, 90.0)]),
            element(LINE_TO, &[(30.0, 90.0)]),
        ]);
        assert_eq!(keyframes(&stored, Ms(1)), Err(SUBPATHS));
        // An empty subpath holds nothing to play and is not counted; a close then a line goes on from the start.
        let mut with_empty = editable(&[sharp(0.0, 0.0), sharp(30.0, 0.0)], false);
        with_empty.subpaths.push(Default::default());
        assert_eq!(
            frames(&with_editable(with_empty)),
            [(0.0, 0.0, 0.0), (1.0, 40.0, 0.0)]
        );
        let reopened = attributes(vec![
            element(MOVE_TO, &[(0.0, 0.0)]),
            element(LINE_TO, &[(30.0, 0.0)]),
            element(CLOSE, &[]),
            element(LINE_TO, &[(0.0, 30.0)]),
        ]);
        assert_eq!(
            frames(&reopened),
            [
                (0.0, 0.0, 0.0),
                (0.33333, 40.0, 0.0),
                (0.66667, 0.0, 0.0),
                (1.0, 0.0, 40.0)
            ]
        );
    }

    #[test]
    fn a_line_moves_from_the_static_position_in_slide_pixels() {
        let path = attributes(vec![
            element(MOVE_TO, &[(10.0, 10.0)]),
            element(LINE_TO, &[(40.0, 10.0)]),
            element(LINE_TO, &[(40.0, 40.0)]),
        ]);
        let BuildEffect::Behaviors(tracks) = keyframes(&path, Ms(900)).unwrap() else {
            panic!()
        };
        let Keyframes::Translate(track) = &tracks[0] else {
            panic!()
        };
        assert_eq!(track.duration_ms, Ms(900));
        let frames: Vec<(f64, f64, f64)> = track
            .frames
            .iter()
            .map(|f| (f.offset.to_f64(), f.value.x.to_f64(), f.value.y.to_f64()))
            .collect();
        assert_eq!(
            frames,
            [(0.0, 0.0, 0.0), (0.5, 40.0, 0.0), (1.0, 40.0, 40.0)]
        );
    }

    #[test]
    fn curves_are_sampled_and_long_paths_thinned() {
        let mut elements = vec![element(MOVE_TO, &[(0.0, 0.0)])];
        for k in 0..200 {
            let x = k as f32 * 3.0;
            elements.push(element(
                CURVE_TO,
                &[(x + 1.0, 5.0), (x + 2.0, 5.0), (x + 3.0, 0.0)],
            ));
        }
        let BuildEffect::Behaviors(tracks) = keyframes(&attributes(elements), Ms(1000)).unwrap()
        else {
            panic!()
        };
        let Keyframes::Translate(track) = &tracks[0] else {
            panic!()
        };
        assert!(track.frames.len() <= limits::KEYFRAMES);
        assert_eq!(track.frames.first().unwrap().offset.steps(), 0);
        assert_eq!(track.frames.last().unwrap().offset.steps(), 100_000);
        assert_eq!(track.frames.last().unwrap().value.x.to_f64(), 800.0);
    }

    #[test]
    fn hostile_paths_fail_closed() {
        let nan = attributes(vec![
            element(MOVE_TO, &[(0.0, 0.0)]),
            element(LINE_TO, &[(f32::NAN, 1.0)]),
        ]);
        assert!(keyframes(&nan, Ms(1)).is_err());
        let far = attributes(vec![
            element(MOVE_TO, &[(0.0, 0.0)]),
            element(LINE_TO, &[(1e9, 1.0)]),
        ]);
        assert!(keyframes(&far, Ms(1)).is_err());
        let line_first = attributes(vec![element(LINE_TO, &[(1.0, 1.0)])]);
        assert!(keyframes(&line_first, Ms(1)).is_err());
        let still = attributes(vec![
            element(MOVE_TO, &[(3.0, 3.0)]),
            element(LINE_TO, &[(3.0, 3.0)]),
        ]);
        assert!(keyframes(&still, Ms(1)).is_err());
        let huge = attributes(vec![
            element(LINE_TO, &[(1.0, 1.0)]);
            limits::PATH_SEGMENTS + 1
        ]);
        assert!(keyframes(&huge, Ms(1)).is_err());
        assert!(keyframes(&kn::BuildAttributesArchive::default(), Ms(1)).is_err());
    }
}
