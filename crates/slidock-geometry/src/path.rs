//! Paths as numbers, before quantisation: what presets and custom geometry trace, and what line ends work on.

use slidock_ir::geometry::Seg;

use crate::arc::{Point, arc_to};

/// A path segment with unquantised coordinates.
pub type Segment = Seg<f64>;

/// A traced path in the shape's box.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapePath {
    pub segments: Vec<Segment>,
    pub fill: bool,
    pub stroke: bool,
}

/// Traces DrawingML path commands, keeping the current point and the subpath start that `arcTo` and `close`
/// continue from.
#[derive(Clone, Debug, Default)]
pub struct PathBuilder {
    segments: Vec<Segment>,
    current: Point,
    start: Point,
}

impl PathBuilder {
    pub fn new() -> PathBuilder {
        PathBuilder::default()
    }

    pub fn move_to(&mut self, p: Point) {
        self.current = p;
        self.start = p;
        self.segments.push(Seg::M(p.x, p.y));
    }

    pub fn line_to(&mut self, p: Point) {
        self.current = p;
        self.segments.push(Seg::L(p.x, p.y));
    }

    pub fn quad_to(&mut self, c: Point, end: Point) {
        self.segments.push(Seg::Q(c.x, c.y, end.x, end.y));
        self.current = end;
    }

    pub fn cubic_to(&mut self, c1: Point, c2: Point, end: Point) {
        self.segments
            .push(Seg::C(c1.x, c1.y, c2.x, c2.y, end.x, end.y));
        self.current = end;
    }

    /// An `arcTo` from the current point, as cubics; returns how many it pushed.
    pub fn arc_to(&mut self, w_r: f64, h_r: f64, st_ang: f64, sw_ang: f64) -> usize {
        let arc = arc_to(self.current, w_r, h_r, st_ang, sw_ang);
        for [x1, y1, x2, y2, x, y] in &arc.cubics {
            self.segments.push(Seg::C(*x1, *y1, *x2, *y2, *x, *y));
        }
        self.current = arc.end;
        arc.cubics.len()
    }

    pub fn close(&mut self) {
        self.segments.push(Seg::Z);
        self.current = self.start;
    }

    /// The segments traced so far.
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn len(&self) -> usize {
        self.segments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn into_segments(self) -> Vec<Segment> {
        self.segments
    }
}

/// `segment` with every x scaled by `sx` and every y by `sy`.
pub fn scale_segment(segment: Segment, sx: f64, sy: f64) -> Segment {
    match segment {
        Seg::M(x, y) => Seg::M(x * sx, y * sy),
        Seg::L(x, y) => Seg::L(x * sx, y * sy),
        Seg::Q(a, b, c, d) => Seg::Q(a * sx, b * sy, c * sx, d * sy),
        Seg::C(a, b, c, d, e, f) => Seg::C(a * sx, b * sy, c * sx, d * sy, e * sx, f * sy),
        Seg::Z => Seg::Z,
    }
}

/// `segment` with `f` applied to every coordinate.
pub fn map_segment<A: Copy, B>(segment: Seg<A>, mut f: impl FnMut(A) -> B) -> Seg<B> {
    match segment {
        Seg::M(x, y) => Seg::M(f(x), f(y)),
        Seg::L(x, y) => Seg::L(f(x), f(y)),
        Seg::Q(a, b, c, d) => Seg::Q(f(a), f(b), f(c), f(d)),
        Seg::C(a, b, c, d, e, g) => Seg::C(f(a), f(b), f(c), f(d), f(e), f(g)),
        Seg::Z => Seg::Z,
    }
}
