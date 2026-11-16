//! Shape outlines. A path source describes a shape at its natural size, and the drawable's geometry stretches it to
//! the box: the outline is fitted to the frame here and mirrored by the source's flips, which mirror the outline only
//! (the geometry's flips mirror the whole drawable, its text too). A plain or rounded rectangle stays a preset, so a
//! text box's background follows the box. Line ends keep their own units; masks are placed in their image's frame, and
//! an image's Instant Alpha outline keeps its natural size.

use std::collections::BTreeSet;
use std::convert::Infallible;
use std::f64::consts::PI;

use indexmap::IndexMap;
use slidock_canon::{Grid, js_round};
use slidock_core_api::lowering::LowerError;
use slidock_ir::geometry::{
    CustomGeometry, CustomPath, CustomTag, FillRule, Geometry, Path, PresetGeometry, PresetName,
    PresetTag, Seg, Segment,
};
use slidock_ir::units::PathUnit;
use slidock_math::{cos, sin};

use crate::geometry::flips;

mod callout;
use crate::num::{clamp, js_max, js_min, path_unit, pt, wide};
use crate::proto::{tsd, tsp};

pub(crate) type Point = (f64, f64);

/// An outline segment in unquantised coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Piece {
    M(Point),
    L(Point),
    Q(Point, Point),
    C(Point, Point, Point),
    Z,
}

impl Piece {
    pub(crate) fn last(self) -> Option<Point> {
        match self {
            Piece::M(p) | Piece::L(p) | Piece::Q(_, p) | Piece::C(_, _, p) => Some(p),
            Piece::Z => None,
        }
    }

    fn map(self, f: impl Fn(Point) -> Point) -> Piece {
        match self {
            Piece::M(p) => Piece::M(f(p)),
            Piece::L(p) => Piece::L(f(p)),
            Piece::Q(a, p) => Piece::Q(f(a), f(p)),
            Piece::C(a, b, p) => Piece::C(f(a), f(b), f(p)),
            Piece::Z => Piece::Z,
        }
    }

    fn to_ir<C>(self, round: impl Fn(f64) -> C) -> Seg<C> {
        match self {
            Piece::M((x, y)) => Seg::M(round(x), round(y)),
            Piece::L((x, y)) => Seg::L(round(x), round(y)),
            Piece::Q((a, b), (x, y)) => Seg::Q(round(a), round(b), round(x), round(y)),
            Piece::C((a, b), (c, d), (x, y)) => {
                Seg::C(round(a), round(b), round(c), round(d), round(x), round(y))
            }
            Piece::Z => Seg::Z,
        }
    }
}

/// An outline and the rectangle of its coordinates that maps onto the box.
struct Outline {
    pieces: Vec<Piece>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

// TSP.Path.ElementType
const MOVE_TO: i32 = 1;
const LINE_TO: i32 = 2;
const QUAD_CURVE_TO: i32 = 3;
const CURVE_TO: i32 = 4;
const CLOSE: i32 = 5;

// TSD.ScalarPathSourceArchive.ScalarPathSourceType
const ROUNDED_RECTANGLE: i32 = 0;
const REGULAR_POLYGON: i32 = 1;
const CHEVRON: i32 = 2;

// TSD.PointPathSourceArchive.PointPathSourceType
const LEFT_ARROW: i32 = 0;
const DOUBLE_ARROW: i32 = 10;
const STAR: i32 = 100;
const PLUS: i32 = 200;

// TSD.ConnectionLinePathSourceArchive.ConnectionLinePathSourceType
const QUADRATIC_CONNECTION: i32 = 0;

// The counts Keynote's inspector allows.
const POLYGON_SIDES: (f64, f64) = (3.0, 11.0);
const STAR_POINTS: (f64, f64) = (3.0, 20.0);

/// A quarter circle as one cubic: control points at this fraction of the radius.
const KAPPA: f64 = 0.5523;

/// ECMA's roundRect: the corner radius is `adj / 100000` of the shorter side, at most half of it.
const ROUND_RECT_MAX_ADJ: f64 = 50_000.0;

fn point(p: Option<&tsp::Point>) -> Point {
    (
        p.and_then(|p| wide(p.x)).unwrap_or(0.0),
        p.and_then(|p| wide(p.y)).unwrap_or(0.0),
    )
}

/// A custom path coordinate, in its own units.
fn unit_coordinate(x: f64) -> PathUnit {
    path_unit(clamp(x, -1e9, 1e9))
}

/// A coordinate in the frame.
fn frame_coordinate(x: f64) -> slidock_ir::units::Pt {
    pt(clamp(x, -100_000.0, 100_000.0))
}

fn rect() -> Option<Geometry> {
    preset("rect", IndexMap::new())
}

/// A preset by name; every name used here is one.
fn preset(name: &str, adjust: IndexMap<String, f64>) -> Option<Geometry> {
    Some(Geometry::Preset(PresetGeometry {
        kind: PresetTag,
        name: PresetName::parse(name)?,
        adjust,
    }))
}

fn is_rounded_rectangle(source: Option<&tsd::PathSourceArchive>) -> bool {
    match source {
        None => true,
        Some(source) => source
            .scalar_path_source
            .as_ref()
            .is_some_and(|scalar| scalar.r#type.unwrap_or(ROUNDED_RECTANGLE) == ROUNDED_RECTANGLE),
    }
}

fn scalar_of(source: Option<&tsd::PathSourceArchive>) -> f64 {
    source
        .and_then(|source| source.scalar_path_source.as_ref())
        .and_then(|scalar| wide(scalar.scalar))
        .unwrap_or(0.0)
}

/// Why a callout's tail is not drawn (its body is), when it is not: `None` for any other path source.
pub fn callout_tail_unread(
    source: Option<&tsd::PathSourceArchive>,
    width: f64,
    height: f64,
) -> Option<String> {
    let callout = source?.callout_path_source.as_ref()?;
    callout::callout_outline(callout, width, height).1
}

/// A shape's geometry in a `width` × `height` frame, `None` for an unreadable path source. A callout is its body and
/// tail as one outline.
pub fn shape_geometry(
    source: Option<&tsd::PathSourceArchive>,
    width: f64,
    height: f64,
    max_segments: usize,
) -> Result<Option<Geometry>, LowerError> {
    if is_rounded_rectangle(source) {
        return Ok(rounded_rect_preset(scalar_of(source), width, height));
    }
    let Some(source) = source else {
        return Ok(rect());
    };
    let Some(outline) = outline_of(source, width, height) else {
        return Ok(None);
    };
    let pieces = fit(&outline, width, height, source_flips(Some(source)));
    if pieces.len() > max_segments {
        return Err(LowerError::Limit(format!(
            "An outline has {} segments",
            pieces.len()
        )));
    }
    if is_box(&pieces, width, height) {
        return Ok(rect());
    }
    Ok(Some(Geometry::Custom(CustomGeometry {
        kind: CustomTag,
        paths: vec![CustomPath {
            w: unit_coordinate(js_max(0.0, width)),
            h: unit_coordinate(js_max(0.0, height)),
            fill: true,
            stroke: true,
            fill_rule: FillRule::Nonzero,
            segments: pieces
                .iter()
                .map(|piece| piece.to_ir(unit_coordinate))
                .collect(),
        }],
    })))
}

fn rounded_rect_preset(radius: f64, width: f64, height: f64) -> Option<Geometry> {
    let shorter = js_min(width, height);
    let r = js_max(0.0, js_min(radius, shorter / 2.0));
    if r == 0.0 || shorter <= 0.0 {
        return rect();
    }
    let adj = js_min(
        ROUND_RECT_MAX_ADJ,
        slidock_math::quantise((r / shorter) * 100_000.0, Grid::Thousandths),
    );
    let mut adjust = IndexMap::new();
    adjust.insert("adj".to_owned(), if adj.is_finite() { adj } else { 0.0 });
    preset("roundRect", adjust)
}

/// An image's Instant Alpha outline (`TSD.ImageArchive.instantAlphaPath`) as the picture's shape: the image is cut to
/// it. The path is in the image's natural size (its pixels, y down), so it is a custom geometry of that size, which
/// lowering stretches to the frame with the image; Keynote's own PPTX export writes it as the picture's `a:custGeom`.
/// It is read strictly: a natural size that is positive on the path grid, at most `max_segments` elements, each of a
/// known type with its exact points, every coordinate present and finite, starting with a move (a closing move back to
/// the start, as Keynote ends a path, is dropped). Otherwise the error says why the outline is not read.
pub fn instant_alpha_geometry(
    path: &tsp::Path,
    natural: Option<&tsp::Size>,
    max_segments: usize,
) -> Result<Geometry, String> {
    // On the path grid: a size that rounds to 0 would leave the outline unscaled, in the image's pixels.
    let extent = |x: Option<f32>| {
        wide(x)
            .filter(|x| x.is_finite())
            .map(unit_coordinate)
            .filter(|x| *x > PathUnit::ZERO)
    };
    let (Some(width), Some(height)) = (
        natural.and_then(|size| extent(size.width)),
        natural.and_then(|size| extent(size.height)),
    ) else {
        return Err("the image has no natural size".to_owned());
    };
    if path.elements.len() > max_segments {
        return Err(format!("it has {} elements", path.elements.len()));
    }
    let mut pieces = Vec::with_capacity(path.elements.len());
    for element in &path.elements {
        let mut points = Vec::with_capacity(element.points.len());
        for p in &element.points {
            let (Some(x), Some(y)) = (wide(p.x), wide(p.y)) else {
                return Err("a point has no x or y".to_owned());
            };
            if !x.is_finite() || !y.is_finite() {
                return Err("a point is not a number".to_owned());
            }
            points.push((x, y));
        }
        let piece = match (element.r#type, points.as_slice()) {
            (Some(MOVE_TO), [p]) => Piece::M(*p),
            (Some(LINE_TO), [p]) => Piece::L(*p),
            (Some(QUAD_CURVE_TO), [a, p]) => Piece::Q(*a, *p),
            (Some(CURVE_TO), [a, b, p]) => Piece::C(*a, *b, *p),
            (Some(CLOSE), []) => Piece::Z,
            (Some(kind @ MOVE_TO..=CLOSE), _) => {
                return Err(format!(
                    "an element of type {kind} has {} points",
                    points.len()
                ));
            }
            (Some(kind), _) => return Err(format!("an element has the unknown type {kind}")),
            (None, _) => return Err("an element has no type".to_owned()),
        };
        pieces.push(piece);
    }
    while matches!(pieces.last(), Some(Piece::M(_))) {
        pieces.pop();
    }
    match pieces.first() {
        None => return Err("it is empty".to_owned()),
        Some(Piece::M(_)) => {}
        Some(_) => return Err("it does not start with a move".to_owned()),
    }
    Ok(Geometry::Custom(CustomGeometry {
        kind: CustomTag,
        paths: vec![CustomPath {
            w: width,
            h: height,
            fill: true,
            stroke: true,
            fill_rule: FillRule::Nonzero,
            segments: pieces
                .iter()
                .map(|piece| piece.to_ir(unit_coordinate))
                .collect(),
        }],
    }))
}

/// A line end's outline in its own units, moved so the line meets it at the origin: it points along +y, and its tip
/// is its largest y (lower-geometry places, scales and trims with it).
pub fn line_end_segments(path: &tsp::Path, end_point: Option<&tsp::Point>) -> Vec<Segment> {
    let (ox, oy) = point(end_point);
    pieces_of(Some(path))
        .into_iter()
        .map(|piece| piece.map(|(x, y)| (x - ox, y - oy)).to_ir(frame_coordinate))
        .collect()
}

/// A mask's visible area in the masked image's frame: the mask's outline fitted to the mask's own geometry (which is
/// in the image's coordinates), mirrored by the outline's and the geometry's flips, and turned about its centre by its
/// angle, counter-clockwise like every Keynote angle.
/// `None` for an unreadable outline.
pub fn mask_paths(
    source: Option<&tsd::PathSourceArchive>,
    geometry: Option<&tsd::GeometryArchive>,
) -> Option<Vec<Path>> {
    let (x, y) = point(geometry.and_then(|g| g.position.as_ref()));
    let size = geometry.and_then(|g| g.size.as_ref());
    let width = size.and_then(|s| wide(s.width)).unwrap_or(0.0);
    let height = size.and_then(|s| wide(s.height)).unwrap_or(0.0);
    let outline = match source {
        Some(source) if !is_rounded_rectangle(Some(source)) => outline_of(source, width, height)?,
        _ => rounded_rect(width, height, scalar_of(source)),
    };
    let (outline_x, outline_y) = source_flips(source);
    let (mask_x, mask_y) = flips(geometry);
    let fitted = fit(
        &outline,
        width,
        height,
        (outline_x != mask_x, outline_y != mask_y),
    );
    let (cx, cy) = (width / 2.0, height / 2.0);
    let angle = geometry.and_then(|g| wide(g.angle)).unwrap_or(0.0);
    let radians = (-angle * PI) / 180.0;
    let (c, s) = (cos(radians), sin(radians));
    let place = |(px, py): Point| {
        (
            x + cx + (px - cx) * c - (py - cy) * s,
            y + cy + (px - cx) * s + (py - cy) * c,
        )
    };
    let segments = fitted
        .iter()
        .map(|piece| piece.map(place).to_ir(frame_coordinate))
        .collect();
    Some(vec![Path {
        segments,
        fill: true,
        stroke: false,
        fill_rule: FillRule::Nonzero,
        role: None,
    }])
}

/// A closed outline of straight lines through the box's four corners and nowhere else.
fn is_box(pieces: &[Piece], width: f64, height: f64) -> bool {
    let Some((Piece::Z, rest)) = pieces.split_last() else {
        return false;
    };
    if pieces.len() < 5 || pieces.len() > 6 {
        return false;
    }
    let near = |value: f64, target: f64| (value - target).abs() < 0.01;
    let mut corners = BTreeSet::new();
    for piece in rest {
        let (x, y) = match piece {
            Piece::M(p) | Piece::L(p) => *p,
            _ => return false,
        };
        if !(near(x, 0.0) || near(x, width)) || !(near(y, 0.0) || near(y, height)) {
            return false;
        }
        corners.insert((near(x, 0.0), near(y, 0.0)));
    }
    corners.len() == 4 || (corners.len() >= 2 && (near(width, 0.0) || near(height, 0.0)))
}

fn outline_of(source: &tsd::PathSourceArchive, width: f64, height: f64) -> Option<Outline> {
    if let Some(bezier) = &source.bezier_path_source {
        return Some(with_bounds(pieces_of(bezier.path.as_ref())));
    }
    if let Some(connection) = &source.connection_line_path_source
        && connection.super_.is_some()
    {
        return Some(connection_outline(connection));
    }
    if let Some(editable) = &source.editable_bezier_path_source {
        return Some(editable_outline(editable));
    }
    if let Some(points) = &source.point_path_source {
        return point_outline(points, width, height);
    }
    if let Some(callout) = &source.callout_path_source {
        return Some(callout::callout_outline(callout, width, height).0);
    }
    let scalar = source.scalar_path_source.as_ref();
    let value = scalar.and_then(|scalar| wide(scalar.scalar));
    match scalar.and_then(|scalar| scalar.r#type) {
        Some(REGULAR_POLYGON) => {
            let sides = count_in(Some(value.unwrap_or(5.0)), POLYGON_SIDES)?;
            Some(polygon(sides, width, height))
        }
        Some(CHEVRON) => Some(chevron(value.unwrap_or(width / 4.0), width, height)),
        _ => Some(rounded_rect(width, height, value.unwrap_or(0.0))),
    }
}

/// A curved connection line passes through its middle point: a quadratic curve whose control point puts it there.
fn connection_outline(source: &tsd::ConnectionLinePathSourceArchive) -> Outline {
    let pieces = pieces_of(
        source
            .super_
            .as_ref()
            .and_then(|bezier| bezier.path.as_ref()),
    );
    let quadratic = source.r#type.unwrap_or(QUADRATIC_CONNECTION) == QUADRATIC_CONNECTION;
    let (true, [Piece::M(start), Piece::L(middle), Piece::L(end)]) = (quadratic, pieces.as_slice())
    else {
        return with_bounds(pieces);
    };
    let ((x0, y0), (x1, y1), (x2, y2)) = (*start, *middle, *end);
    let control = (2.0 * x1 - (x0 + x2) / 2.0, 2.0 * y1 - (y0 + y2) / 2.0);
    // The box is the drawn curve's bounds, not the points': Keynote sizes the drawable to the curve, so the points
    // keep the box's own coordinates (retro-fidelity's kn-6284213 bulges past its middle point, and Keynote's PDF and
    // PPTX export start it at its box's right edge and end it where its points say, unstretched).
    let extremes = [
        quadratic_extreme((x0, y0), control, (x2, y2), |(x, _)| x),
        quadratic_extreme((x0, y0), control, (x2, y2), |(_, y)| y),
    ];
    let mut outline = with_bounds(
        [Piece::M(*start), Piece::L(*end)]
            .into_iter()
            .chain(extremes.into_iter().flatten().map(Piece::L))
            .collect(),
    );
    outline.pieces = vec![Piece::M(*start), Piece::Q(control, (x2, y2))];
    outline
}

/// The point where a quadratic turns back along one axis, when it does between its ends.
fn quadratic_extreme(
    start: Point,
    control: Point,
    end: Point,
    axis: impl Fn(Point) -> f64,
) -> Option<Point> {
    let (a, b, c) = (axis(start), axis(control), axis(end));
    let t = (a - b) / (a - 2.0 * b + c);
    (t > 0.0 && t < 1.0).then(|| {
        let at = |p: f64, q: f64, r: f64| {
            (1.0 - t) * (1.0 - t) * p + 2.0 * t * (1.0 - t) * q + t * t * r
        };
        (at(start.0, control.0, end.0), at(start.1, control.1, end.1))
    })
}

fn pieces_of(path: Option<&tsp::Path>) -> Vec<Piece> {
    let mut pieces = Vec::new();
    for element in path
        .map(|path| path.elements.as_slice())
        .unwrap_or_default()
    {
        let points: Vec<Point> = element.points.iter().map(|p| point(Some(p))).collect();
        let piece = match (element.r#type, points.as_slice()) {
            (Some(MOVE_TO), [p, ..]) => Piece::M(*p),
            (Some(LINE_TO), [p, ..]) => Piece::L(*p),
            (Some(QUAD_CURVE_TO), [a, p, ..]) => Piece::Q(*a, *p),
            (Some(CURVE_TO), [a, b, p, ..]) => Piece::C(*a, *b, *p),
            (Some(CLOSE), _) => Piece::Z,
            _ => continue,
        };
        pieces.push(piece);
    }
    // Keynote ends a closed path with a move back to its start, which draws nothing.
    while matches!(pieces.last(), Some(Piece::M(_))) {
        pieces.pop();
    }
    // A drawing path starts with a move.
    if let Some(first) = pieces.first()
        && !matches!(first, Piece::M(_))
        && let Some(start) = pieces.iter().find_map(|piece| piece.last())
    {
        pieces.insert(0, Piece::M(start));
    }
    pieces
}

fn editable_outline(source: &tsd::EditableBezierPathSourceArchive) -> Outline {
    let pieces: Result<_, Infallible> = editable_pieces(source, |p| Ok(point(p)));
    let Ok(pieces) = pieces;
    with_bounds(pieces)
}

/// An editable path in its own coordinates: each subpath a move to its first node, then a cubic from each node by
/// its out handle and the next node's in handle to that node (a straight line when the handles sit on the nodes); a
/// closed subpath goes on from its last node to its first the same way and closes. `point` reads each point the
/// path uses, and its first error is the path's.
pub(crate) fn editable_pieces<E>(
    source: &tsd::EditableBezierPathSourceArchive,
    point: impl Fn(Option<&tsp::Point>) -> Result<Point, E>,
) -> Result<Vec<Piece>, E> {
    let mut pieces = Vec::new();
    for subpath in &source.subpaths {
        let nodes = &subpath.nodes;
        let Some(first) = nodes.first() else {
            continue;
        };
        pieces.push(Piece::M(point(first.node_point.as_ref())?));
        let closed = subpath.closed == Some(true);
        let count = if closed { nodes.len() } else { nodes.len() - 1 };
        for (i, from) in nodes.iter().enumerate().take(count) {
            let Some(to) = nodes.get((i + 1) % nodes.len()) else {
                continue;
            };
            pieces.push(Piece::C(
                point(from.out_control_point.as_ref())?,
                point(to.in_control_point.as_ref())?,
                point(to.node_point.as_ref())?,
            ));
        }
        if closed {
            pieces.push(Piece::Z);
        }
    }
    Ok(pieces)
}

/// A drawn path keeps coordinates of its own (a unit square for a rectangle mask, a 100 pt diagonal for a line) and
/// is stretched to its natural size, which is the box: the bounds of its points map onto the box.
fn with_bounds(pieces: Vec<Piece>) -> Outline {
    let points: Vec<Point> = pieces.iter().filter_map(|piece| piece.last()).collect();
    if points.is_empty() {
        return Outline {
            pieces,
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        };
    }
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (x, y) in points {
        min_x = js_min(min_x, x);
        min_y = js_min(min_y, y);
        max_x = js_max(max_x, x);
        max_y = js_max(max_y, y);
    }
    Outline {
        pieces,
        x: min_x,
        y: min_y,
        width: max_x - min_x,
        height: max_y - min_y,
    }
}

/// `a || b` for numbers: `b` when `a` is absent, 0 or NaN.
fn or_number(a: Option<f64>, b: f64) -> f64 {
    a.filter(|a| *a != 0.0 && !a.is_nan()).unwrap_or(b)
}

/// Arrows, the star and the plus: `point` places the shape's adjustable handle in its natural size.
fn point_outline(source: &tsd::PointPathSourceArchive, width: f64, height: f64) -> Option<Outline> {
    let natural = source.natural_size.as_ref();
    let w = or_number(natural.and_then(|n| wide(n.width)), width);
    let h = or_number(natural.and_then(|n| wide(n.height)), height);
    let handle = source.point.as_ref();
    let px = handle.and_then(|p| wide(p.x)).unwrap_or(w / 2.0);
    let py = handle.and_then(|p| wide(p.y)).unwrap_or(1.0 / 3.0);
    let poly = |points: &[Point]| Outline {
        pieces: polyline(points),
        x: 0.0,
        y: 0.0,
        width: w,
        height: h,
    };
    // Arrows: x is the head's length, y the shaft's thickness as a fraction of the height.
    let shaft = (h * (1.0 - clamp(py, 0.0, 1.0))) / 2.0;
    Some(match source.r#type {
        Some(LEFT_ARROW) => poly(&[
            (0.0, h / 2.0),
            (px, 0.0),
            (px, shaft),
            (w, shaft),
            (w, h - shaft),
            (px, h - shaft),
            (px, h),
        ]),
        Some(DOUBLE_ARROW) => poly(&[
            (0.0, h / 2.0),
            (px, 0.0),
            (px, shaft),
            (w - px, shaft),
            (w - px, 0.0),
            (w, h / 2.0),
            (w - px, h),
            (w - px, h - shaft),
            (px, h - shaft),
            (px, h),
        ]),
        Some(STAR) => {
            // x is the number of points, y the inner radius as a fraction of the outer.
            let count = count_in(handle.and_then(|p| wide(p.x)), STAR_POINTS)?;
            let inner = clamp(py, 0.0, 1.0);
            let mut points = Vec::new();
            for i in 0..count * 2 {
                let radius = if i % 2 == 0 { 1.0 } else { inner };
                let angle = -PI / 2.0 + (f64::from(i) * PI) / f64::from(count);
                points.push((
                    w / 2.0 + (radius * w * cos(angle)) / 2.0,
                    h / 2.0 + (radius * h * sin(angle)) / 2.0,
                ));
            }
            poly(&points)
        }
        Some(PLUS) => {
            // The handle is the arms' inner corner.
            let ax = js_min(px, w / 2.0);
            let ay = js_min(py, h / 2.0);
            poly(&[
                (ax, 0.0),
                (w - ax, 0.0),
                (w - ax, ay),
                (w, ay),
                (w, h - ay),
                (w - ax, h - ay),
                (w - ax, h),
                (ax, h),
                (ax, h - ay),
                (0.0, h - ay),
                (0.0, ay),
                (ax, ay),
            ])
        }
        // The right arrow, also for any other type.
        _ => poly(&[
            (w, h / 2.0),
            (w - px, 0.0),
            (w - px, shaft),
            (0.0, shaft),
            (0.0, h - shaft),
            (w - px, h - shaft),
            (w - px, h),
        ]),
    })
}

fn rounded_rect(width: f64, height: f64, radius: f64) -> Outline {
    let r = js_max(0.0, js_min(js_min(radius, width / 2.0), height / 2.0));
    let boxed = |pieces| Outline {
        pieces,
        x: 0.0,
        y: 0.0,
        width,
        height,
    };
    if r == 0.0 {
        return boxed(polyline(&[
            (0.0, 0.0),
            (width, 0.0),
            (width, height),
            (0.0, height),
        ]));
    }
    let k = r * (1.0 - KAPPA);
    boxed(vec![
        Piece::M((r, 0.0)),
        Piece::L((width - r, 0.0)),
        Piece::C((width - k, 0.0), (width, k), (width, r)),
        Piece::L((width, height - r)),
        Piece::C(
            (width, height - k),
            (width - k, height),
            (width - r, height),
        ),
        Piece::L((r, height)),
        Piece::C((k, height), (0.0, height - k), (0.0, height - r)),
        Piece::L((0.0, r)),
        Piece::C((0.0, k), (k, 0.0), (r, 0.0)),
        Piece::Z,
    ])
}

fn polygon(sides: u32, width: f64, height: f64) -> Outline {
    let points: Vec<Point> = (0..sides)
        .map(|i| {
            let angle = -PI / 2.0 + (f64::from(i) * 2.0 * PI) / f64::from(sides);
            (
                width / 2.0 + (width / 2.0) * cos(angle),
                height / 2.0 + (height / 2.0) * sin(angle),
            )
        })
        .collect();
    Outline {
        pieces: polyline(&points),
        x: 0.0,
        y: 0.0,
        width,
        height,
    }
}

fn chevron(depth: f64, width: f64, height: f64) -> Outline {
    let d = js_max(0.0, js_min(depth, width));
    Outline {
        pieces: polyline(&[
            (0.0, 0.0),
            (width - d, 0.0),
            (width, height / 2.0),
            (width - d, height),
            (0.0, height),
            (d, height / 2.0),
        ]),
        x: 0.0,
        y: 0.0,
        width,
        height,
    }
}

/// A count the file sets, when it is one Keynote allows.
fn count_in(value: Option<f64>, (min, max): (f64, f64)) -> Option<u32> {
    let count = value.map_or(f64::NAN, js_round);
    // Within [3, 20], so it fits.
    (count >= min && count <= max).then_some(count as u32)
}

fn polyline(points: &[Point]) -> Vec<Piece> {
    let mut pieces: Vec<Piece> = Vec::with_capacity(points.len() + 1);
    for (i, point) in points.iter().enumerate() {
        pieces.push(if i == 0 {
            Piece::M(*point)
        } else {
            Piece::L(*point)
        });
    }
    pieces.push(Piece::Z);
    pieces
}

/// A path source's own flips, which mirror its outline: left to right, then top to bottom.
fn source_flips(source: Option<&tsd::PathSourceArchive>) -> (bool, bool) {
    (
        source.is_some_and(|s| s.horizontal_flip == Some(true)),
        source.is_some_and(|s| s.vertical_flip == Some(true)),
    )
}

/// Maps an outline's rectangle onto the box, then mirrors it in the box.
fn fit(outline: &Outline, width: f64, height: f64, (flip_x, flip_y): (bool, bool)) -> Vec<Piece> {
    let scale = |extent: f64, natural: f64| {
        if natural != 0.0 && !natural.is_nan() {
            extent / natural
        } else {
            1.0
        }
    };
    let sx = scale(width, outline.width);
    let sy = scale(height, outline.height);
    let map = |(px, py): Point| {
        let x = (px - outline.x) * sx;
        let y = (py - outline.y) * sy;
        (
            if flip_x { width - x } else { x },
            if flip_y { height - y } else { y },
        )
    };
    outline.pieces.iter().map(|piece| piece.map(map)).collect()
}
