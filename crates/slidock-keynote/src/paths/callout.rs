//! A callout (`TSD.CalloutPathSourceArchive`) as one outline: its body united with its tail, in its natural size, as
//! Keynote 14.5 builds it (`TSDCalloutPathSource`, read from TSDrawables' arm64 code):
//!
//! - The body is a rounded rectangle whose corners are circular arcs of the corner radius, or the ellipse of the box
//!   once the radius reaches half the shorter side. A file without a radius stores none: the body is then the ellipse.
//! - The tail is the triangle from the tip (`tail_position`) to two points on the body, united with it, so its sides
//!   join the outline where they first enter the body. A tip inside the body draws no tail.
//! - The tail aims at a centre: the body's centre for `center_tail` (true when unset); otherwise the point level with
//!   the tip on the body's middle line when the tip lies between the corners' arcs along x, else on the middle column
//!   when it does along y, else the centre of the nearest corner's arc (corners measured with the stored radius). The
//!   axis from the tip to that centre enters the body at the tail's base point.
//! - Its sides run from the tip through the points `|tail_size|` either side of the base point, square to the axis:
//!   `tail_size` is the half-width at the body's edge. It is at most 90 % of the narrower side's widest reach of the
//!   body seen from the tip (the body's path points and curve midpoints, scaled to the base point's distance), and that
//!   limit is at least 5.

use super::{KAPPA, Outline, Piece, Point, rounded_rect};
use crate::num::{js_max, js_min, wide};
use crate::proto::tsd;

/// The smallest limit Keynote puts on a tail's half-width.
const MIN_TAIL_LIMIT: f64 = 5.0;
/// The share of the body's reach a tail may take.
const TAIL_REACH: f64 = 0.9;
/// Sign changes of a cubic's distance to a line looked for between these many steps.
const STEPS: u32 = 32;

/// A body segment from its start point.
#[derive(Clone, Copy, Debug)]
enum Segment {
    Line(Point, Point),
    Cubic(Point, Point, Point, Point),
}

impl Segment {
    fn at(self, t: f64) -> Point {
        match self {
            Segment::Line(a, b) => lerp(a, b, t),
            Segment::Cubic(a, b, c, d) => {
                let s = 1.0 - t;
                let (k0, k1, k2, k3) = (s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t);
                (
                    k0 * a.0 + k1 * b.0 + k2 * c.0 + k3 * d.0,
                    k0 * a.1 + k1 * b.1 + k2 * c.1 + k3 * d.1,
                )
            }
        }
    }

    fn start(self) -> Point {
        match self {
            Segment::Line(a, _) | Segment::Cubic(a, ..) => a,
        }
    }

    fn end(self) -> Point {
        match self {
            Segment::Line(_, b) | Segment::Cubic(.., b) => b,
        }
    }

    /// The part from `t0` to `t1`.
    fn part(self, t0: f64, t1: f64) -> Piece {
        match self {
            Segment::Line(a, b) => Piece::L(lerp(a, b, t1)),
            Segment::Cubic(..) => {
                let (_, right) = split(self, t0);
                let local = if t0 < 1.0 {
                    (t1 - t0) / (1.0 - t0)
                } else {
                    0.0
                };
                let (left, _) = split(right, local);
                match left {
                    Segment::Cubic(_, b, c, d) => Piece::C(b, c, d),
                    Segment::Line(_, d) => Piece::L(d),
                }
            }
        }
    }
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// De Casteljau's split of a cubic at `t`.
fn split(segment: Segment, t: f64) -> (Segment, Segment) {
    match segment {
        Segment::Line(a, b) => {
            let m = lerp(a, b, t);
            (Segment::Line(a, m), Segment::Line(m, b))
        }
        Segment::Cubic(a, b, c, d) => {
            let (ab, bc, cd) = (lerp(a, b, t), lerp(b, c, t), lerp(c, d, t));
            let (abc, bcd) = (lerp(ab, bc, t), lerp(bc, cd, t));
            let m = lerp(abc, bcd, t);
            (Segment::Cubic(a, ab, abc, m), Segment::Cubic(m, bcd, cd, d))
        }
    }
}

fn sub(a: Point, b: Point) -> Point {
    (a.0 - b.0, a.1 - b.1)
}

fn dot(a: Point, b: Point) -> f64 {
    a.0 * b.0 + a.1 * b.1
}

fn cross(a: Point, b: Point) -> f64 {
    a.0 * b.1 - a.1 * b.0
}

fn length(a: Point) -> f64 {
    dot(a, a).sqrt()
}

fn unit(a: Point) -> Point {
    let l = length(a);
    if l > 0.0 {
        (a.0 / l, a.1 / l)
    } else {
        (0.0, 0.0)
    }
}

/// The body's segments, from the closed outline `pieces`.
fn segments(pieces: &[Piece]) -> Vec<Segment> {
    let mut out = Vec::new();
    let (mut start, mut at) = ((0.0, 0.0), (0.0, 0.0));
    for piece in pieces {
        match *piece {
            Piece::M(p) => {
                start = p;
                at = p;
            }
            Piece::L(p) => {
                out.push(Segment::Line(at, p));
                at = p;
            }
            Piece::Q(c, p) => {
                let c1 = (
                    at.0 + 2.0 / 3.0 * (c.0 - at.0),
                    at.1 + 2.0 / 3.0 * (c.1 - at.1),
                );
                let c2 = (p.0 + 2.0 / 3.0 * (c.0 - p.0), p.1 + 2.0 / 3.0 * (c.1 - p.1));
                out.push(Segment::Cubic(at, c1, c2, p));
                at = p;
            }
            Piece::C(a, b, p) => {
                out.push(Segment::Cubic(at, a, b, p));
                at = p;
            }
            Piece::Z => {
                if at != start {
                    out.push(Segment::Line(at, start));
                }
                at = start;
            }
        }
    }
    out
}

/// The ellipse of a `width` × `height` box: four quarter cubics from the top.
fn ellipse(width: f64, height: f64) -> Outline {
    let (rx, ry) = (width / 2.0, height / 2.0);
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    let (cx, cy) = (rx, ry);
    Outline {
        pieces: vec![
            Piece::M((cx, 0.0)),
            Piece::C((cx + kx, 0.0), (width, cy - ky), (width, cy)),
            Piece::C((width, cy + ky), (cx + kx, height), (cx, height)),
            Piece::C((cx - kx, height), (0.0, cy + ky), (0.0, cy)),
            Piece::C((0.0, cy - ky), (cx - kx, 0.0), (cx, 0.0)),
            Piece::Z,
        ],
        x: 0.0,
        y: 0.0,
        width,
        height,
    }
}

/// Where a ray from `from` along `direction` crosses the body: (segment, its parameter, the distance along the ray),
/// nearest first. A ray of length `reach` ends there.
fn crossings(
    body: &[Segment],
    from: Point,
    direction: Point,
    reach: f64,
) -> Vec<(usize, f64, f64)> {
    let mut out = Vec::new();
    for (index, segment) in body.iter().enumerate() {
        let side = |t: f64| cross(direction, sub(segment.at(t), from));
        let mut roots = Vec::new();
        match segment {
            Segment::Line(..) => {
                let (s0, s1) = (side(0.0), side(1.0));
                if s0 == s1 {
                    continue;
                }
                let t = s0 / (s0 - s1);
                if (0.0..=1.0).contains(&t) {
                    roots.push(t);
                }
            }
            Segment::Cubic(..) => {
                let mut prev = side(0.0);
                for k in 1..=STEPS {
                    let (t0, t1) = (
                        f64::from(k - 1) / f64::from(STEPS),
                        f64::from(k) / f64::from(STEPS),
                    );
                    let next = side(t1);
                    if prev == 0.0 {
                        roots.push(t0);
                    } else if prev * next < 0.0 {
                        let (mut lo, mut hi, mut f_lo) = (t0, t1, prev);
                        for _ in 0..60 {
                            let mid = (lo + hi) / 2.0;
                            let f_mid = side(mid);
                            if f_lo * f_mid <= 0.0 {
                                hi = mid;
                            } else {
                                lo = mid;
                                f_lo = f_mid;
                            }
                        }
                        roots.push((lo + hi) / 2.0);
                    }
                    prev = next;
                }
                if prev == 0.0 {
                    roots.push(1.0);
                }
            }
        }
        for t in roots {
            let along = dot(sub(segment.at(t), from), direction);
            if along >= 0.0 && along <= reach {
                out.push((index, t, along));
            }
        }
    }
    out.sort_by(|a, b| a.2.total_cmp(&b.2));
    out
}

/// Whether a point lies inside the body (crossings of a ray to +x, either way round).
fn contains(body: &[Segment], p: Point) -> bool {
    let mut winding = 0_i32;
    for segment in body {
        let points: Vec<Point> = match segment {
            Segment::Line(a, b) => vec![*a, *b],
            Segment::Cubic(..) => (0..=STEPS)
                .map(|k| segment.at(f64::from(k) / f64::from(STEPS)))
                .collect(),
        };
        for pair in points.windows(2) {
            let &[a, b] = pair else { continue };
            if (a.1 <= p.1) != (b.1 <= p.1) {
                let x = a.0 + (p.1 - a.1) / (b.1 - a.1) * (b.0 - a.0);
                if x > p.0 {
                    winding += if b.1 > a.1 { 1 } else { -1 };
                }
            }
        }
    }
    winding != 0
}

/// The tail's centre: the body's, or the tip moved onto the box's middle line or column, or a corner arc's centre.
fn tail_centre(tip: Point, radius: f64, width: f64, height: f64, at_centre: bool) -> Point {
    if at_centre {
        return (width / 2.0, height / 2.0);
    }
    let (x, y) = tip;
    if x > radius && x < width - radius {
        return (x, height / 2.0);
    }
    if y > radius && y < height - radius {
        return (width / 2.0, y);
    }
    (
        if x > radius { width - radius } else { radius },
        if y > radius { height - radius } else { radius },
    )
}

/// The widest half-width a tail may take: 90 % of the narrower side's reach, at least 5.
fn tail_limit(body: &[Segment], tip: Point, base: Point) -> f64 {
    let distance = length(sub(base, tip));
    let axis = unit(sub(base, tip));
    let (mut most, mut least) = (0.0_f64, 0.0_f64);
    let mut reach = |p: Point| {
        let v = sub(p, tip);
        let value = cross(axis, v) * distance / dot(v, axis);
        most = js_max(most, value);
        least = js_min(least, value);
    };
    for segment in body {
        reach(segment.start());
        if let Segment::Cubic(..) = segment {
            reach(segment.at(0.5));
        }
        reach(segment.end());
    }
    // Keynote takes the product to single precision before the floor.
    let limit = (js_min(most, -least) * TAIL_REACH) as f32;
    f64::from(limit.max(MIN_TAIL_LIMIT as f32))
}

/// A callout's outline at its natural size (or the box's, when it has none), body and tail as one; when the tail
/// cannot be placed, the body and the reason.
pub(super) fn callout_outline(
    callout: &tsd::CalloutPathSourceArchive,
    width: f64,
    height: f64,
) -> (Outline, Option<String>) {
    let natural = callout.natural_size.as_ref();
    let positive =
        |x: Option<f64>, fallback: f64| x.filter(|x| *x > 0.0 && x.is_finite()).unwrap_or(fallback);
    let w = positive(natural.and_then(|n| wide(n.width)), width);
    let h = positive(natural.and_then(|n| wide(n.height)), height);
    let shorter = js_min(w, h) / 2.0;
    // An unset radius is the box's width and 10: the ellipse.
    let radius = wide(callout.corner_radius).unwrap_or(w + 10.0);
    let body = if radius >= shorter {
        ellipse(w, h)
    } else {
        rounded_rect(w, h, radius)
    };
    let tip = (
        callout
            .tail_position
            .as_ref()
            .and_then(|p| wide(p.x))
            .unwrap_or(0.0),
        callout
            .tail_position
            .as_ref()
            .and_then(|p| wide(p.y))
            .unwrap_or(0.0),
    );
    let half = wide(callout.tail_size).unwrap_or(0.0).abs();
    if !(w > 0.0 && h > 0.0 && tip.0.is_finite() && tip.1.is_finite() && half.is_finite()) {
        return (body, Some("its size or tail is not a number".to_owned()));
    }
    let edges = segments(&body.pieces);
    if contains(&edges, tip) || half == 0.0 {
        return (body, None);
    }
    let centre = tail_centre(tip, radius, w, h, callout.center_tail.unwrap_or(true));
    let to_centre = sub(centre, tip);
    let base = crossings(&edges, tip, unit(to_centre), length(to_centre))
        .last()
        .map_or(centre, |&(index, t, _)| {
            edges.get(index).map_or(centre, |segment| segment.at(t))
        });
    let half = js_min(half, tail_limit(&edges, tip, base));
    let axis = unit(sub(base, tip));
    let across = (-axis.1 * half, axis.0 * half);
    let side = |sign: f64| {
        let through = (base.0 + sign * across.0, base.1 + sign * across.1);
        let direction = unit(sub(through, tip));
        crossings(&edges, tip, direction, f64::INFINITY)
            .first()
            .copied()
    };
    let (Some(a), Some(b), Some(&(base_index, base_t, _))) = (
        side(1.0),
        side(-1.0),
        crossings(&edges, tip, axis, f64::INFINITY).first(),
    ) else {
        return (
            body,
            Some("the tail is wider than the body seen from its tip".to_owned()),
        );
    };
    let place = |(index, t, _): (usize, f64, f64)| index as f64 + t;
    let (pa, pb, pc) = (place(a), place(b), place((base_index, base_t, 0.0)));
    // The tail covers the stretch of the body between its sides that holds the base point; the outline keeps the rest.
    let (first, last) = if pa <= pb { (a, b) } else { (b, a) };
    let covered_inside = (place(first)..=place(last)).contains(&pc) && pa != pb;
    let (from, to) = if covered_inside {
        (last, first)
    } else {
        (first, last)
    };
    let mut pieces = Vec::new();
    let start = edges.get(from.0).map_or(tip, |s| s.at(from.1));
    pieces.push(Piece::M(start));
    walk(&edges, (from.0, from.1), (to.0, to.1), &mut pieces);
    pieces.push(Piece::L(tip));
    pieces.push(Piece::Z);
    (
        Outline {
            pieces,
            x: 0.0,
            y: 0.0,
            width: w,
            height: h,
        },
        None,
    )
}

/// The body's outline forward from one place on it to another, wrapping past its start.
fn walk(edges: &[Segment], from: (usize, f64), to: (usize, f64), out: &mut Vec<Piece>) {
    let count = edges.len();
    let (mut index, mut t) = from;
    for _ in 0..=count {
        let Some(segment) = edges.get(index).copied() else {
            return;
        };
        if index == to.0 && to.1 >= t {
            if to.1 > t {
                out.push(segment.part(t, to.1));
            }
            return;
        }
        if t < 1.0 {
            out.push(segment.part(t, 1.0));
        }
        index = (index + 1) % count;
        t = 0.0;
    }
}
