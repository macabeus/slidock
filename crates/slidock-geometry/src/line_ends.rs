//! Arrowheads as paths of their own. A stroke's head sits at the start of the shape's first open stroked path and its
//! tail at the end of the last one (DrawingML's head and tail). An end's tip is the path's end, and it points from the
//! path's point its own length back to the tip: on a curve, the chord under the end, not the tangent at the tip. The
//! path is cut (de Casteljau, in an earlier segment when the last is shorter) where an end with a body begins. Both
//! ends are placed before either cut, so neither depends on what the other's cut leaves.
//!
//! - A preset (DrawingML's, ODF's markers) measures along the path, as LibreOffice 25.2 draws it (the line-ends
//!   probe, `cargo xtask probes-lines`, within 0.1 pt of its PDF).
//! - A path end (Keynote's) measures as the crow flies, grows 0.4 + 0.6 × the stroke width, and takes the line on
//!   straight half a width into it, as Keynote 14.5's PDF draws it.

use slidock_ir::geometry::Seg;
use slidock_ir::paint::{EndSize, LineEnd, LineEndPath, LineEndPreset, LineEndShape};

use crate::formula::{js_max, js_min};
use crate::path::{Segment, ShapePath, map_segment};

type P = [f64; 2];

/// A cubic's handle length for a quarter ellipse.
const KAPPA: f64 = 0.552_284_749_8;

/// Samples per curve when looking for its point a given distance from the tip; the crossing is then bisected.
const CURVE_SAMPLES: u32 = 32;
const BISECTIONS: u32 = 52;

/// DrawingML's width and length classes, in stroke widths.
fn size(size: EndSize) -> f64 {
    match size {
        EndSize::Sm => 2.0,
        EndSize::Med => 3.0,
        EndSize::Lg => 5.0,
    }
}

/// The stroke a shape's line ends belong to.
#[derive(Clone, Copy, Debug)]
pub struct StrokeEnds<'a> {
    pub width: f64,
    pub head: Option<&'a LineEnd>,
    pub tail: Option<&'a LineEnd>,
}

/// The paths, shortened under arrowheads with a body, and the arrowheads after them.
#[derive(Clone, Debug, PartialEq)]
pub struct WithLineEnds {
    pub paths: Vec<ShapePath>,
    pub ends: Vec<ShapePath>,
}

fn draws(segment: &Segment) -> bool {
    !matches!(segment, Seg::M(..) | Seg::Z)
}

fn is_open(path: &ShapePath) -> bool {
    path.stroke && path.segments.iter().any(draws) && !matches!(path.segments.last(), Some(Seg::Z))
}

fn end_of(segment: Option<&Segment>) -> Option<P> {
    let coords = segment?.coords();
    match coords.as_slice() {
        [.., x, y] => Some([*x, *y]),
        _ => None,
    }
}

fn controls(segment: &Segment) -> Vec<P> {
    segment.coords().as_chunks::<2>().0.to_vec()
}

/// A segment like `like` through `points` (its controls and end).
fn with_points(like: &Segment, points: &[P]) -> Segment {
    match (like, points) {
        (Seg::M(..), [[x, y]]) => Seg::M(*x, *y),
        (Seg::L(..), [[x, y]]) => Seg::L(*x, *y),
        (Seg::Q(..), [[a, b], [x, y]]) => Seg::Q(*a, *b, *x, *y),
        (Seg::C(..), [[a, b], [c, d], [x, y]]) => Seg::C(*a, *b, *c, *d, *x, *y),
        _ => *like,
    }
}

/// The drawn segments of the subpath at the path's start or end, walked from its tip: each as its index and its points
/// in the path's order (the point before it, its controls, its end). Empty when the tip's segment has no point to
/// leave from (nothing before it, or a close).
struct Walk {
    at_start: bool,
    steps: Vec<(usize, Vec<P>)>,
}

impl Walk {
    fn new(segments: &[Segment], at_start: bool) -> Walk {
        let mut steps = Vec::new();
        let mut next = if at_start {
            segments.iter().position(draws)
        } else {
            segments.iter().rposition(draws)
        };
        while let Some(at) = next {
            let Some(segment) = segments.get(at).filter(|segment| draws(segment)) else {
                break;
            };
            let Some(from) = at.checked_sub(1).and_then(|i| end_of(segments.get(i))) else {
                break;
            };
            let mut points = vec![from];
            points.extend(controls(segment));
            steps.push((at, points));
            next = if at_start {
                at.checked_add(1)
            } else {
                at.checked_sub(1)
            };
        }
        Walk { at_start, steps }
    }

    /// A segment's end on the tip's side, or away from it.
    fn end(&self, points: &[P], tip_side: bool) -> Option<P> {
        if self.at_start == tip_side {
            points.first().copied()
        } else {
            points.last().copied()
        }
    }

    fn tip(&self) -> Option<P> {
        let (_, points) = self.steps.first()?;
        self.end(points, true)
    }

    /// Where the subpath starts (for a tail) or stops (for a head).
    fn far(&self) -> Option<P> {
        let (_, points) = self.steps.last()?;
        self.end(points, false)
    }

    /// A segment's parameter `along` the way from its tip side.
    fn param(&self, along: f64) -> f64 {
        if self.at_start { along } else { 1.0 - along }
    }

    /// Where the walk first comes `reach` from the tip, measured the end's way.
    fn crossing(&self, tip: P, reach: f64, measure: Measure) -> Crossing {
        match measure {
            Measure::Straight => self.straight(tip, reach),
            Measure::Along => self.along(reach),
        }
    }

    /// The step-0 line's parameter `reach` from the tip, when the line is that long: its length gives it.
    fn on_tip_line(&self, points: &[P], reach: f64) -> Option<f64> {
        let length = curve_length(points);
        (length > 0.0 && length >= reach).then(|| {
            let keep = js_max(0.0, length - reach) / length;
            if self.at_start { 1.0 - keep } else { keep }
        })
    }

    /// Where the walk first comes `reach` along it from the tip (curves measured as 32 chords, as their length is).
    fn along(&self, reach: f64) -> Crossing {
        let mut walked = 0.0;
        for (step, (_, points)) in self.steps.iter().enumerate() {
            if points.iter().flatten().any(|c| !c.is_finite()) {
                return Crossing::Unreadable;
            }
            if step == 0 && points.len() == 2 {
                if let Some(t) = self.on_tip_line(points, reach) {
                    return Crossing::At(step, t);
                }
                walked += curve_length(points);
                continue;
            }
            let length = curve_length(points);
            if length > 0.0 && walked + length >= reach {
                let want = reach - walked;
                if points.len() == 2 {
                    return Crossing::At(step, self.param(js_min(1.0, js_max(0.0, want / length))));
                }
                // The tip's side of the curve, cut at `t`, is as long as it should be.
                let tip_side = |t: f64| {
                    let (before, after) = split(points, t);
                    curve_length(if self.at_start { &before } else { &after })
                };
                let (mut low, mut high) = (0.0, 1.0);
                for _ in 0..BISECTIONS {
                    let middle = (low + high) / 2.0;
                    if tip_side(self.param(middle)) >= want {
                        high = middle;
                    } else {
                        low = middle;
                    }
                }
                return Crossing::At(step, self.param(high));
            }
            walked += length;
        }
        Crossing::Short
    }

    /// Where the walk first comes `reach` from the tip, as the crow flies.
    fn straight(&self, tip: P, reach: f64) -> Crossing {
        for (step, (_, points)) in self.steps.iter().enumerate() {
            if points.iter().flatten().any(|c| !c.is_finite()) {
                return Crossing::Unreadable;
            }
            if let [_, _] = points.as_slice() {
                let (Some(near), Some(far)) = (self.end(points, true), self.end(points, false))
                else {
                    return Crossing::Unreadable;
                };
                if step == 0 {
                    if let Some(t) = self.on_tip_line(points, reach) {
                        return Crossing::At(step, t);
                    }
                    continue;
                }
                // A line's distance from the tip has one minimum, and `near` is within reach: the larger root of
                // |near + t (far - near) - tip| = reach.
                let v = [far[0] - near[0], far[1] - near[1]];
                let w = [near[0] - tip[0], near[1] - tip[1]];
                let a = v[0] * v[0] + v[1] * v[1];
                if a > 0.0 && distance(far, tip) >= reach {
                    let b = 2.0 * (v[0] * w[0] + v[1] * w[1]);
                    let c = w[0] * w[0] + w[1] * w[1] - reach * reach;
                    let t = (-b + js_max(0.0, b * b - 4.0 * a * c).sqrt()) / (2.0 * a);
                    return Crossing::At(step, self.param(js_min(1.0, js_max(0.0, t))));
                }
                continue;
            }
            let mut inside = self.param(0.0);
            for sample in 1..=CURVE_SAMPLES {
                let outside = self.param(f64::from(sample) / f64::from(CURVE_SAMPLES));
                if distance(point_at(points, outside), tip) >= reach {
                    let (mut low, mut high) = (inside, outside);
                    for _ in 0..BISECTIONS {
                        let middle = (low + high) / 2.0;
                        if distance(point_at(points, middle), tip) >= reach {
                            high = middle;
                        } else {
                            low = middle;
                        }
                    }
                    return Crossing::At(step, high);
                }
                inside = outside;
            }
        }
        Crossing::Short
    }

    /// The tangent at the tip: from the nearest control point apart from the tip itself.
    fn tangent(&self, tip: P) -> Option<P> {
        let (_, points) = self.steps.first()?;
        let far = |[x, y]: &&P| (x - tip[0]).abs() + (y - tip[1]).abs() > 1e-9;
        let toward = if self.at_start {
            points.iter().skip(1).find(far)
        } else {
            points.iter().rev().skip(1).find(far)
        }?;
        let (dx, dy) = (tip[0] - toward[0], tip[1] - toward[1]);
        let length = (dx * dx + dy * dy).sqrt();
        Some([dx / length, dy / length])
    }

    /// The unit direction an end `extent` long points along: from the walk's point that far from the tip (its far end
    /// when the path is shorter) to the tip, so on a curve it follows the chord under the end, not the tangent at the
    /// tip. On the tip's own line, and when there is no chord, the tangent.
    fn direction(&self, tip: P, extent: f64, measure: Measure) -> Option<P> {
        if extent.is_nan() || extent <= 0.0 {
            return self.tangent(tip);
        }
        let from = match self.crossing(tip, extent, measure) {
            Crossing::At(0, _)
                if self
                    .steps
                    .first()
                    .is_some_and(|(_, points)| points.len() == 2) =>
            {
                return self.tangent(tip);
            }
            Crossing::At(step, t) => self.steps.get(step).map(|(_, points)| point_at(points, t)),
            Crossing::Short => self.far(),
            Crossing::Unreadable => None,
        };
        from.and_then(|from| unit(from, tip))
            .or_else(|| self.tangent(tip))
    }

    /// Every walked segment shrunk to the walk's far end (a head's move with them).
    fn collapse(&self, segments: &[Segment], far: P, start_move: usize) -> Vec<Segment> {
        let mut out = segments.to_vec();
        for (index, points) in &self.steps {
            if let Some(slot) = out.get_mut(*index) {
                *slot = with_points(slot, &vec![far; points.len().saturating_sub(1)]);
            }
        }
        if self.at_start
            && let Some(slot) = out.get_mut(start_move)
        {
            *slot = Seg::M(far[0], far[1]);
        }
        out
    }

    /// The path cut where the walk first comes `body` from the tip, the part nearer the tip dropped; with `into`, the
    /// line goes on that far along `toward` (the end's direction) from the cut, under the end. When the whole
    /// subpath is within `body` of the tip, every walked segment shrinks to the walk's far end; a walk that meets a
    /// coordinate that is not a number leaves the path as it is.
    fn shorten(&self, segments: &[Segment], tip: P, toward: P, reach: Reach) -> Vec<Segment> {
        let Reach {
            body,
            into,
            measure,
            ..
        } = reach;
        // A head's walk follows the move it leaves from.
        let (Some((nearest, _)), Some(far)) = (self.steps.first(), self.far()) else {
            return segments.to_vec();
        };
        let Some(start_move) = nearest.checked_sub(1) else {
            return segments.to_vec();
        };
        if body.is_nan() || body <= 0.0 {
            return segments.to_vec();
        }
        let (step, t) = match self.crossing(tip, body, measure) {
            Crossing::At(step, t) => (step, t),
            Crossing::Unreadable => return segments.to_vec(),
            Crossing::Short => return self.collapse(segments, far, start_move),
        };
        let Some((index, points)) = self.steps.get(step) else {
            return segments.to_vec();
        };
        let Some(segment) = segments.get(*index) else {
            return segments.to_vec();
        };
        let (before, after) = split(points, t);
        let mut out = Vec::with_capacity(segments.len() + 1);
        if self.at_start {
            let Some(&cut) = after.first() else {
                return segments.to_vec();
            };
            out.extend_from_slice(segments.get(..start_move).unwrap_or_default());
            if into > 0.0 {
                out.push(Seg::M(cut[0] + toward[0] * into, cut[1] + toward[1] * into));
                out.push(Seg::L(cut[0], cut[1]));
            } else {
                out.push(Seg::M(cut[0], cut[1]));
            }
            out.push(with_points(segment, after.get(1..).unwrap_or_default()));
            out.extend_from_slice(segments.get(index + 1..).unwrap_or_default());
        } else {
            let Some(&cut) = before.last() else {
                return segments.to_vec();
            };
            out.extend_from_slice(segments.get(..*index).unwrap_or_default());
            out.push(with_points(segment, before.get(1..).unwrap_or_default()));
            if into > 0.0 {
                out.push(Seg::L(cut[0] + toward[0] * into, cut[1] + toward[1] * into));
            }
            out.extend_from_slice(segments.get(nearest + 1..).unwrap_or_default());
        }
        out
    }
}

/// How an end measures its length back from the tip.
#[derive(Clone, Copy, Debug)]
enum Measure {
    /// As the crow flies (Keynote).
    Straight,
    /// Along the path (LibreOffice).
    Along,
}

/// Where a walk first comes a distance from its tip.
#[derive(Clone, Copy, Debug)]
enum Crossing {
    /// On the walk's step, at the parameter on its points.
    At(usize, f64),
    /// The whole walk is nearer the tip.
    Short,
    /// A coordinate on the way is not a number.
    Unreadable,
}

fn distance(a: P, b: P) -> f64 {
    let (dx, dy) = (a[0] - b[0], a[1] - b[1]);
    (dx * dx + dy * dy).sqrt()
}

fn unit(from: P, to: P) -> Option<P> {
    let length = distance(from, to);
    (length > 1e-9).then(|| [(to[0] - from[0]) / length, (to[1] - from[1]) / length])
}

/// de Casteljau: a line or Bézier's control points, split at `t`.
fn split(points: &[P], t: f64) -> (Vec<P>, Vec<P>) {
    let mut before = Vec::with_capacity(points.len());
    let mut after = Vec::with_capacity(points.len());
    let mut level = points.to_vec();
    while let (Some(first), Some(last)) = (level.first(), level.last()) {
        before.push(*first);
        after.push(*last);
        level = level
            .windows(2)
            .map(|pair| match pair {
                [a, b] => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
                _ => [0.0, 0.0],
            })
            .collect();
    }
    after.reverse();
    (before, after)
}

fn point_at(points: &[P], t: f64) -> P {
    split(points, t)
        .0
        .last()
        .copied()
        .unwrap_or([f64::NAN, f64::NAN])
}

fn curve_length(points: &[P]) -> f64 {
    let steps: u32 = if points.len() == 2 { 1 } else { 32 };
    let Some(&first) = points.first() else {
        return 0.0;
    };
    let mut length = 0.0;
    let mut last = first;
    for step in 1..=steps {
        let (before, _) = split(points, f64::from(step) / f64::from(steps));
        let Some(&[x, y]) = before.last() else {
            continue;
        };
        let (dx, dy) = (x - last[0], y - last[1]);
        length += (dx * dx + dy * dy).sqrt();
        last = [x, y];
    }
    length
}

/// A placed end: its tip, the unit direction it points along, and the normal across it.
struct Placement {
    tip: P,
    out: P,
    n: P,
}

impl Placement {
    fn new(tip: P, out: P) -> Placement {
        Placement {
            tip,
            out,
            n: [-out[1], out[0]],
        }
    }

    fn at(&self, along: f64, across: f64) -> P {
        [
            self.tip[0] + along * self.out[0] + across * self.n[0],
            self.tip[1] + along * self.out[1] + across * self.n[1],
        ]
    }
}

fn polygon(points: &[P], closed: bool) -> ShapePath {
    let mut segments: Vec<Segment> = points
        .iter()
        .enumerate()
        .map(|(i, &[x, y])| if i == 0 { Seg::M(x, y) } else { Seg::L(x, y) })
        .collect();
    if closed {
        segments.push(Seg::Z);
    }
    ShapePath {
        segments,
        fill: closed,
        stroke: !closed,
    }
}

/// How much a path end (Keynote's) grows with its stroke: 0.4 + 0.6 × the width, in the end's own units. Keynote 14.5
/// draws its "simple arrow" (6 units long) 9.6, 27.6, 31.2 and 45.6 pt long on strokes 2, 7, 8 and 12 pt wide.
fn path_scale(end: &LineEndPath, width: f64) -> f64 {
    if end.scale_with_width {
        0.4 + 0.6 * width
    } else {
        1.0
    }
}

fn path_segments(end: &LineEndPath) -> Vec<Segment> {
    end.segments
        .iter()
        .map(|segment| map_segment(*segment, |c| c.to_f64()))
        .collect()
}

/// How far a path end's tip is from where the line meets it (its largest y), in its own units.
fn path_reach(segments: &[Segment]) -> f64 {
    segments
        .iter()
        .flat_map(controls)
        .map(|[_, y]| y)
        .reduce(js_max)
        .unwrap_or(0.0)
}

fn custom_end(end: &LineEndPath, width: f64, frame: &Placement) -> ShapePath {
    let scale = path_scale(end, width);
    let segments = path_segments(end);
    let reach = path_reach(&segments);
    let place = |[x, y]: P| frame.at((y - reach) * scale, x * scale);
    let placed: Vec<Segment> = segments
        .iter()
        .map(|segment| match segment {
            Seg::Z => Seg::Z,
            _ => {
                let points: Vec<P> = controls(segment).into_iter().map(place).collect();
                with_points(segment, &points)
            }
        })
        .collect();
    let closed = placed.iter().any(|segment| matches!(segment, Seg::Z));
    ShapePath {
        segments: placed,
        fill: closed,
        stroke: !closed,
    }
}

fn preset_end(end: &LineEndPreset, width: f64, f: &Placement) -> ShapePath {
    let length = size(end.len) * width;
    let half = (size(end.w) * width) / 2.0;
    match end.shape {
        LineEndShape::Triangle => polygon(
            &[f.at(0.0, 0.0), f.at(-length, half), f.at(-length, -half)],
            true,
        ),
        LineEndShape::Stealth => polygon(
            &[
                f.at(0.0, 0.0),
                f.at(-length, half),
                f.at(-length / 2.0, 0.0),
                f.at(-length, -half),
            ],
            true,
        ),
        LineEndShape::Diamond => polygon(
            &[
                f.at(length / 2.0, 0.0),
                f.at(0.0, half),
                f.at(-length / 2.0, 0.0),
                f.at(0.0, -half),
            ],
            true,
        ),
        LineEndShape::Arrow => polygon(
            &[f.at(-length, half), f.at(0.0, 0.0), f.at(-length, -half)],
            false,
        ),
        LineEndShape::Oval => {
            let (a, b, k) = (length / 2.0, half, KAPPA);
            let cubic = |c1: P, c2: P, p: P| Seg::C(c1[0], c1[1], c2[0], c2[1], p[0], p[1]);
            let [mx, my] = f.at(a, 0.0);
            let segments = vec![
                Seg::M(mx, my),
                cubic(f.at(a, k * b), f.at(k * a, b), f.at(0.0, b)),
                cubic(f.at(-k * a, b), f.at(-a, k * b), f.at(-a, 0.0)),
                cubic(f.at(-a, -k * b), f.at(-k * a, -b), f.at(0.0, -b)),
                cubic(f.at(k * a, -b), f.at(a, -k * b), f.at(a, 0.0)),
                Seg::Z,
            ];
            ShapePath {
                segments,
                fill: true,
                stroke: false,
            }
        }
    }
}

/// An end's extent along the line from its tip: `extent`, how far back it reaches (what it points along), `body`,
/// where the line stops (0 for an end without one), and `into`, how far the line then goes on under it.
#[derive(Clone, Copy, Debug)]
struct Reach {
    extent: f64,
    body: f64,
    into: f64,
    measure: Measure,
}

/// A path end (Keynote's) reaches back to where the line meets it, and Keynote takes the line on half a stroke width
/// into it (at most to the tip), so no seam shows between the two. A preset reaches its length back (an oval or a
/// diamond, centred on the tip, half of it); only a triangle and a stealth have a body.
fn reach(end: &LineEnd, width: f64) -> Reach {
    match end {
        LineEnd::Path(path) => {
            let length = path_reach(&path_segments(path)) * path_scale(path, width);
            Reach {
                extent: length,
                body: length,
                into: js_max(0.0, js_min(width / 2.0, length)),
                measure: Measure::Straight,
            }
        }
        LineEnd::Preset(preset) => {
            let length = size(preset.len) * width;
            let (extent, body) = match preset.shape {
                LineEndShape::Triangle => (length, length),
                LineEndShape::Stealth => (length, length / 2.0),
                LineEndShape::Arrow => (length, 0.0),
                LineEndShape::Diamond | LineEndShape::Oval => (length / 2.0, 0.0),
            };
            Reach {
                extent,
                body,
                into: 0.0,
                measure: Measure::Along,
            }
        }
    }
}

/// An end's outline at `tip` pointing along `out`.
fn end_shape(end: &LineEnd, width: f64, tip: P, out: P) -> ShapePath {
    let frame = Placement::new(tip, out);
    match end {
        LineEnd::Path(path) => custom_end(path, width, &frame),
        LineEnd::Preset(preset) => preset_end(preset, width, &frame),
    }
}

/// The paths with the stroke's line ends added after them: the head at the start of the first open stroked path,
/// the tail at the end of the last; those paths are cut where an arrowhead's body begins. Both ends are placed on the
/// paths as drawn, before either cut, so on a path shorter than their bodies each still points along the whole path
/// (as LibreOffice 25.2 draws them), and the line under both is cut away.
pub fn with_line_ends(paths: &[ShapePath], stroke: StrokeEnds<'_>) -> WithLineEnds {
    let mut out = paths.to_vec();
    let open: Vec<usize> = out
        .iter()
        .enumerate()
        .filter(|(_, path)| is_open(path))
        .map(|(i, _)| i)
        .collect();
    let mut placed = Vec::new();
    for (end, at_start) in [(stroke.head, true), (stroke.tail, false)] {
        let target = if at_start { open.first() } else { open.last() };
        let (Some(end), Some(&target)) = (end, target) else {
            continue;
        };
        let Some(path) = out.get(target) else {
            continue;
        };
        let walk = Walk::new(&path.segments, at_start);
        let Some(tip) = walk.tip() else {
            continue;
        };
        let reach = reach(end, stroke.width);
        let Some(toward) = walk.direction(tip, reach.extent, reach.measure) else {
            continue;
        };
        placed.push((end, at_start, target, tip, toward, reach));
    }
    let mut ends = Vec::with_capacity(placed.len());
    for (end, at_start, target, tip, toward, reach) in placed {
        ends.push(end_shape(end, stroke.width, tip, toward));
        if let Some(path) = out.get_mut(target) {
            path.segments =
                Walk::new(&path.segments, at_start).shorten(&path.segments, tip, toward, reach);
        }
    }
    WithLineEnds { paths: out, ends }
}
