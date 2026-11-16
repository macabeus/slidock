//! lower-geometry's gradients: each source form as a function t(u) of the frame-fraction point u = (x/W, y/H) of the
//! unrotated, unflipped box.
//! - DrawingML's angle (`linear`): the gradient line runs through the centre of the box, or of its tile, and spans
//!   the projections of that rectangle's corners, each side pushed out by `outset` points. Unscaled, the angle holds
//!   in the shape's real proportions; scaled, in the tile's fractions. Without `rotateWithShape` it is turned back by
//!   the frame's rotation and flips.
//! - DrawingML's path gradient (`path`): a real circle around the tile's centre through the tile's corners, its
//!   focus where the focus rectangle divides like the tile; or the tile's rectangle contours around the focus
//!   rectangle.
//! - ODF's styles (`odf`, LibreOffice 25.2): defined over B, the page-aligned bounds of the drawn fill outline (curve
//!   extrema, not control points; the box's when the outline has no area), and mapped back into the frame, so the
//!   gradient stays page-aligned and unmirrored under the printed transform. The end colour is at the centre of every
//!   style but linear and axial.
//! - A box with a side of 0 paints nothing; its fractions are found in a stand-in box with area.
//! - A focus on or outside the end ellipse moves to 0.98 of the way along its ray, which is reported.

use slidock_core_api::PassError;
use slidock_ir::color::Color;
use slidock_ir::diagnostics::{DiagnosticCode, codes};
use slidock_ir::features::FeatureId;
use slidock_ir::frame::Frame;
use slidock_ir::geometry::{Path, PathRole, Seg, Segment};
use slidock_ir::paint::{
    Edges, EllipseLowered, Fill, GradientStop, LinearAngle, LinearAngled, LinearLowered, MAX_BANDS,
    MAX_STEPS, MAX_STOPS, OdfGradient, OdfStyle, PathGradient, PathShape, Point, RadialLowered,
    RectLowered, Semi,
};
use slidock_ir::units::{Angle, Ratio, Size, UnitRatio};

use crate::container::unit_error;
use crate::normalize_frames::normal_angle;

const DEG: f64 = std::f64::consts::PI / 180.0;

/// How far along its ray a focus outside the end ellipse is moved.
const FOCUS_INSIDE: f64 = 0.98;

/// A gradient drawn otherwise than its source states, for the format's gradient code.
#[derive(Clone, Debug, PartialEq)]
pub struct Approximation {
    pub code: DiagnosticCode,
    pub feature: FeatureId,
    pub reason: String,
}

/// A fill in its frame-local form and what lowering approximated.
#[derive(Clone, Debug, PartialEq)]
pub struct LoweredFill {
    pub fill: Fill,
    pub approximations: Vec<Approximation>,
}

fn ratio(x: f64) -> Result<Ratio, PassError> {
    Ratio::quantise(x).map_err(|e| unit_error("A gradient fraction", e))
}

fn unit_ratio(x: f64) -> Result<UnitRatio, PassError> {
    UnitRatio::quantise(x.clamp(0.0, 1.0)).map_err(|e| unit_error("A gradient stop", e))
}

fn point(x: f64, y: f64) -> Result<Point, PassError> {
    Ok(Point {
        x: ratio(x)?,
        y: ratio(y)?,
    })
}

fn length(x: f64, y: f64) -> f64 {
    (x * x + y * y).sqrt()
}

/// The frame's turn and flips, between frame-local points (pt), page vectors and frame fractions u. Page vectors are
/// relative to the frame's centre: p = R(θ)·F·(q − (W/2, H/2)).
#[derive(Clone, Copy, Debug)]
struct Placement {
    w: f64,
    h: f64,
    cos: f64,
    sin: f64,
    fx: f64,
    fy: f64,
}

impl Placement {
    fn of(frame: &Frame) -> Placement {
        let turn = frame.rotation.to_f64() * DEG;
        Placement {
            w: frame.w.to_f64(),
            h: frame.h.to_f64(),
            cos: slidock_math::cos(turn),
            sin: slidock_math::sin(turn),
            fx: if frame.flip_h { -1.0 } else { 1.0 },
            fy: if frame.flip_v { -1.0 } else { 1.0 },
        }
    }

    /// A frame-local point on the page, relative to the frame's centre.
    fn page_point(&self, x: f64, y: f64) -> (f64, f64) {
        let (x, y) = ((x - self.w / 2.0) * self.fx, (y - self.h / 2.0) * self.fy);
        (self.cos * x - self.sin * y, self.sin * x + self.cos * y)
    }

    /// A page vector turned back into the frame's unflipped axes, in points.
    fn back(&self, x: f64, y: f64) -> (f64, f64) {
        let (x, y) = (self.cos * x + self.sin * y, -self.sin * x + self.cos * y);
        (self.fx * x, self.fy * y)
    }

    /// The frame fractions of a page point relative to the frame's centre.
    fn u_of(&self, x: f64, y: f64) -> (f64, f64) {
        let (x, y) = self.back(x, y);
        ((self.w / 2.0 + x) / self.w, (self.h / 2.0 + y) / self.h)
    }

    /// A page vector in frame fractions.
    fn u_vector(&self, x: f64, y: f64) -> (f64, f64) {
        let (x, y) = self.back(x, y);
        (x / self.w, y / self.h)
    }
}

/// An axis-aligned box: left, top, right, bottom.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Bounds {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Bounds {
    fn empty() -> Bounds {
        Bounds {
            x0: f64::INFINITY,
            y0: f64::INFINITY,
            x1: f64::NEG_INFINITY,
            y1: f64::NEG_INFINITY,
        }
    }

    fn add(&mut self, (x, y): (f64, f64)) {
        self.x0 = self.x0.min(x);
        self.y0 = self.y0.min(y);
        self.x1 = self.x1.max(x);
        self.y1 = self.y1.max(y);
    }

    /// No points, or points on one line across the page's axes.
    fn has_no_area(&self) -> bool {
        !(self.x0 < self.x1 && self.y0 < self.y1)
    }
}

/// The parameters in (0, 1) where a quadratic Bézier's coordinate has an extremum.
fn quad_extrema(p0: f64, p1: f64, p2: f64) -> Vec<f64> {
    let d = p0 - 2.0 * p1 + p2;
    if d == 0.0 {
        return Vec::new();
    }
    let t = (p0 - p1) / d;
    if t > 0.0 && t < 1.0 {
        vec![t]
    } else {
        Vec::new()
    }
}

/// The parameters in (0, 1) where a cubic Bézier's coordinate has an extremum.
fn cubic_extrema(p0: f64, p1: f64, p2: f64, p3: f64) -> Vec<f64> {
    // B'(t)/3 = a t² + b t + c.
    let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
    let b = 2.0 * (p0 - 2.0 * p1 + p2);
    let c = p1 - p0;
    let roots = if a.abs() < 1e-12 {
        if b == 0.0 { Vec::new() } else { vec![-c / b] }
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc < 0.0 {
            Vec::new()
        } else {
            let root = disc.sqrt();
            vec![(-b + root) / (2.0 * a), (-b - root) / (2.0 * a)]
        }
    };
    roots.into_iter().filter(|t| *t > 0.0 && *t < 1.0).collect()
}

fn quad_at(p0: f64, p1: f64, p2: f64, t: f64) -> f64 {
    let s = 1.0 - t;
    s * s * p0 + 2.0 * s * t * p1 + t * t * p2
}

fn cubic_at(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    let s = 1.0 - t;
    s * s * s * p0 + 3.0 * s * s * t * p1 + 3.0 * s * t * t * p2 + t * t * t * p3
}

/// The tight page bounds of the paths that fill, mapped by the placement: Bézier extrema, not control points.
fn page_bounds(paths: &[Path], place: &Placement) -> Bounds {
    let mut bounds = Bounds::empty();
    for path in paths
        .iter()
        .filter(|path| path.fill && path.role != Some(PathRole::LineEnd))
    {
        let mut at = (0.0, 0.0);
        let mut start = at;
        for segment in &path.segments {
            let p = |x: slidock_ir::units::Pt, y: slidock_ir::units::Pt| {
                place.page_point(x.to_f64(), y.to_f64())
            };
            match *segment {
                Seg::M(x, y) => {
                    at = p(x, y);
                    start = at;
                    bounds.add(at);
                }
                Seg::L(x, y) => {
                    at = p(x, y);
                    bounds.add(at);
                }
                Seg::Q(x1, y1, x, y) => {
                    let (c, end) = (p(x1, y1), p(x, y));
                    for t in quad_extrema(at.0, c.0, end.0) {
                        bounds.add((quad_at(at.0, c.0, end.0, t), quad_at(at.1, c.1, end.1, t)));
                    }
                    for t in quad_extrema(at.1, c.1, end.1) {
                        bounds.add((quad_at(at.0, c.0, end.0, t), quad_at(at.1, c.1, end.1, t)));
                    }
                    at = end;
                    bounds.add(at);
                }
                Seg::C(x1, y1, x2, y2, x, y) => {
                    let (c1, c2, end) = (p(x1, y1), p(x2, y2), p(x, y));
                    let point = |t: f64| {
                        (
                            cubic_at(at.0, c1.0, c2.0, end.0, t),
                            cubic_at(at.1, c1.1, c2.1, end.1, t),
                        )
                    };
                    for t in cubic_extrema(at.0, c1.0, c2.0, end.0) {
                        bounds.add(point(t));
                    }
                    for t in cubic_extrema(at.1, c1.1, c2.1, end.1) {
                        bounds.add(point(t));
                    }
                    at = end;
                    bounds.add(at);
                }
                Seg::Z => at = start,
            }
        }
    }
    bounds
}

/// The frame's own rectangle as a path.
fn box_path(w: f64, h: f64) -> Result<Path, PassError> {
    let pt =
        |x: f64| slidock_ir::units::Pt::quantise(x).map_err(|e| unit_error("A gradient box", e));
    let (w, h) = (pt(w)?, pt(h)?);
    let zero = slidock_ir::units::Pt::ZERO;
    let segments: Vec<Segment> = vec![
        Seg::M(zero, zero),
        Seg::L(w, zero),
        Seg::L(w, h),
        Seg::L(zero, h),
        Seg::Z,
    ];
    Ok(Path {
        segments,
        fill: true,
        stroke: false,
        fill_rule: slidock_ir::geometry::FillRule::Nonzero,
        role: None,
    })
}

/// The angle of a gradient in the unrotated, unflipped box: turned back by the frame unless it turns with it.
pub(crate) fn box_angle(
    angle: Angle,
    rotate_with_shape: bool,
    frame: &Frame,
) -> Result<Angle, PassError> {
    if rotate_with_shape {
        return Ok(angle);
    }
    let mut angle = angle.to_f64() - frame.rotation.to_f64();
    if frame.flip_h {
        angle = 180.0 - angle;
    }
    if frame.flip_v {
        angle = -angle;
    }
    normal_angle(angle)
}

/// A gradient line through the box centre along `direction` (box fractions), spanning the box corner to corner.
pub(crate) fn linear_ends(dx: f64, dy: f64) -> Result<(Point, Point), PassError> {
    span_ends(dx, dy, (0.5, 0.5), (1.0, 1.0))
}

/// A gradient line through `centre` along `direction` (box fractions), spanning the corners of the `size`
/// rectangle around it.
fn span_ends(
    dx: f64,
    dy: f64,
    (cx, cy): (f64, f64),
    (sw, sh): (f64, f64),
) -> Result<(Point, Point), PassError> {
    let length = length(dx, dy);
    let (ex, ey) = if length > 0.0 {
        (dx / length, dy / length)
    } else {
        (1.0, 0.0)
    };
    let half = (ex.abs() * sw + ey.abs() * sh) / 2.0;
    Ok((
        point(cx - half * ex, cy - half * ey)?,
        point(cx + half * ex, cy + half * ey)?,
    ))
}

/// The tile's rectangle in box fractions: left, top, width, height.
fn tile_rect(tile: Option<&Edges>) -> (f64, f64, f64, f64) {
    match tile {
        None => (0.0, 0.0, 1.0, 1.0),
        Some(t) => (
            t.l.to_f64(),
            t.t.to_f64(),
            1.0 - t.l.to_f64() - t.r.to_f64(),
            1.0 - t.t.to_f64() - t.b.to_f64(),
        ),
    }
}

fn lower_angle(linear: &LinearAngle, frame: &Frame) -> Result<Fill, PassError> {
    let angle = box_angle(linear.angle, linear.rotate_with_shape, frame)?.to_f64();
    let (c, s) = (
        slidock_math::cos(angle * DEG),
        slidock_math::sin(angle * DEG),
    );
    let (w, h) = (frame.w.to_f64(), frame.h.to_f64());
    let (left, top, tw, th) = tile_rect(linear.tile.as_ref());
    let unscaled = (w * c, h * s);
    let (dx, dy) = if linear.scaled || (unscaled.0 == 0.0 && unscaled.1 == 0.0) {
        (c / tw, s / th)
    } else {
        unscaled
    };
    let (from, to) = match (linear.outset, &linear.tile) {
        (None, None) => linear_ends(dx, dy)?,
        _ => {
            let outset = linear.outset.map_or(0.0, |o| o.to_f64());
            let grow = |side: f64| if side > 0.0 { 2.0 * outset / side } else { 0.0 };
            span_ends(
                dx,
                dy,
                (left + tw / 2.0, top + th / 2.0),
                (tw + grow(w), th + grow(h)),
            )?
        }
    };
    Ok(Fill::Linear(LinearLowered {
        from,
        to,
        stops: linear.stops.clone(),
        steps: None,
    }))
}

/// A real circle (or the ellipse `rx` × `ry` points) about `centre` (fractions) with its focus (fractions), as a
/// radial gradient: the y radius and the focus are left out when they add nothing. A focus on or outside the ellipse
/// moves inside it.
fn radial(
    centre: (f64, f64),
    (rx, ry): (f64, f64),
    focus: Option<(f64, f64)>,
    (w, h): (f64, f64),
    stops: Vec<GradientStop<Color>>,
    steps: Option<u16>,
) -> Result<(RadialLowered<Color>, bool), PassError> {
    let radius = ratio(rx / w)?;
    let radius_y = ratio(ry / h)?;
    let center = point(centre.0, centre.1)?;
    let (r, q) = (radius.to_f64(), radius_y.to_f64());
    let reach = |focus: &Point| {
        let (dx, dy) = (
            focus.x.to_f64() - center.x.to_f64(),
            focus.y.to_f64() - center.y.to_f64(),
        );
        if r > 0.0 && q > 0.0 {
            length(dx / r, dy / q)
        } else {
            0.0
        }
    };
    let inside = |focus: Point| -> Result<Point, PassError> {
        let k = FOCUS_INSIDE / reach(&focus);
        point(
            center.x.to_f64() + (focus.x.to_f64() - center.x.to_f64()) * k,
            center.y.to_f64() + (focus.y.to_f64() - center.y.to_f64()) * k,
        )
    };
    let mut moved = false;
    let focus = match focus {
        None => None,
        Some((fx, fy)) => {
            let mut focus = point(fx, fy)?;
            if reach(&focus) >= 1.0 {
                moved = length((fx - center.x.to_f64()) / r, (fy - center.y.to_f64()) / q) >= 1.0;
                focus = inside(focus)?;
            }
            (focus != center).then_some(focus)
        }
    };
    Ok((
        RadialLowered {
            center,
            radius,
            radius_y: (radius_y != radius).then_some(radius_y),
            focus,
            stops,
            steps,
        },
        moved,
    ))
}

fn moved_focus(code: DiagnosticCode) -> Approximation {
    Approximation {
        code,
        feature: FeatureId::FillRadial,
        reason: "the focus lies outside the circle".to_owned(),
    }
}

/// One axis of DrawingML's path focus: the point that divides the focus rectangle as it divides the tile.
fn focus_axis(t0: f64, td: f64, f0: f64, fd: f64) -> f64 {
    if td - fd <= 1e-6 {
        f0 + fd / 2.0
    } else {
        f0 + fd * (f0 - t0) / (td - fd)
    }
}

fn lower_path(
    path: &PathGradient,
    frame: &Frame,
    notes: &mut Vec<Approximation>,
) -> Result<Fill, PassError> {
    let (w, h) = (frame.w.to_f64(), frame.h.to_f64());
    let (t0x, t0y, tw, th) = tile_rect(Some(&path.tile));
    let (f0x, f0y, fw, fh) = tile_rect(Some(&path.focus));
    let centre = (t0x + tw / 2.0, t0y + th / 2.0);
    match path.path {
        PathShape::Circle => {
            let r = length(tw * w, th * h) / 2.0;
            let focus = (focus_axis(t0x, tw, f0x, fw), focus_axis(t0y, th, f0y, fh));
            let (lowered, moved) = radial(
                centre,
                (r, r),
                Some(focus),
                (w, h),
                path.stops.clone(),
                None,
            )?;
            if moved {
                notes.push(moved_focus(codes::PX2106));
            }
            Ok(Fill::Radial(lowered))
        }
        PathShape::Rect => {
            let (hw, hh) = (tw / 2.0, th / 2.0);
            let side = |f: f64, c: f64, half: f64| {
                if half > 0.0 {
                    ((f - c) / half).clamp(-1.0, 1.0)
                } else {
                    0.0
                }
            };
            let (mut l, mut r) = (side(f0x, centre.0, hw), side(f0x + fw, centre.0, hw));
            let (mut t, mut b) = (side(f0y, centre.1, hh), side(f0y + fh, centre.1, hh));
            if l > r {
                (l, r) = ((l + r) / 2.0, (l + r) / 2.0);
            }
            if t > b {
                (t, b) = ((t + b) / 2.0, (t + b) / 2.0);
            }
            Ok(Fill::Rect(Box::new(RectLowered {
                origin: point(centre.0, centre.1)?,
                axis_x: point(hw, 0.0)?,
                axis_y: point(0.0, hh)?,
                inner: Edges {
                    l: ratio(l)?,
                    t: ratio(t)?,
                    r: ratio(r)?,
                    b: ratio(b)?,
                },
                stops: path.stops.clone(),
                steps: None,
            })))
        }
    }
}

/// The stops at `pos(o)` of each source stop, in order.
fn moved_stops(
    stops: &[GradientStop<Color>],
    pos: impl Fn(f64) -> f64,
) -> Result<Vec<GradientStop<Color>>, PassError> {
    let mut out = stops
        .iter()
        .map(|stop| {
            Ok(GradientStop {
                pos: unit_ratio(pos(stop.pos.to_f64()))?,
                color: stop.color.clone(),
            })
        })
        .collect::<Result<Vec<_>, PassError>>()?;
    out.sort_by_key(|stop| stop.pos);
    Ok(out)
}

/// ODF's stops end to start: the end colour at t = 0 (the centre), the start colour at 1.
fn reversed(stops: &[GradientStop<Color>]) -> Result<Vec<GradientStop<Color>>, PassError> {
    stops
        .iter()
        .rev()
        .map(|stop| {
            Ok(GradientStop {
                pos: unit_ratio(1.0 - stop.pos.to_f64())?,
                color: stop.color.clone(),
            })
        })
        .collect()
}

/// An axial gradient's stops over s ∈ [0, 1]: the start colour on the axis at ½, each stop at ½ ∓ o(1 − b)/2. At
/// most 32 source stops are kept so the mirror fits in 64.
fn axial_stops(
    stops: &[GradientStop<Color>],
    border: f64,
    notes: &mut Vec<Approximation>,
) -> Result<Vec<GradientStop<Color>>, PassError> {
    let limit = MAX_STOPS / 2;
    let kept: Vec<&GradientStop<Color>> = if stops.len() > limit {
        notes.push(Approximation {
            code: codes::OD9103,
            feature: FeatureId::FillLinear,
            reason: format!("its {} axial stops are drawn as {limit}", stops.len()),
        });
        (0..limit)
            .filter_map(|i| stops.get(i * (stops.len() - 1) / (limit - 1)))
            .collect()
    } else {
        stops.iter().collect()
    };
    let half = (1.0 - border) / 2.0;
    let mut out = Vec::with_capacity(kept.len() * 2);
    for stop in kept.iter().rev() {
        out.push(GradientStop {
            pos: unit_ratio(0.5 - stop.pos.to_f64() * half)?,
            color: stop.color.clone(),
        });
    }
    for (i, stop) in kept.iter().enumerate() {
        if i == 0 && stop.pos.to_f64() == 0.0 {
            continue;
        }
        out.push(GradientStop {
            pos: unit_ratio(0.5 + stop.pos.to_f64() * half)?,
            color: stop.color.clone(),
        });
    }
    Ok(out)
}

/// LibreOffice's band count for an ellipsoid without one (`calculateNumberOfSteps`): ⌊maxChannelΔ·127.5 + 0.5⌋
/// per pair of stops, the largest over the pairs, at least 1; bounded so the bands stay within 2048.
fn automatic_steps(stops: &[GradientStop<Color>]) -> u16 {
    let mut most = 0.0f64;
    for pair in stops.windows(2) {
        if let [a, b] = pair
            && let (Some(a), Some(b)) = (a.color.rgb(), b.color.rgb())
        {
            for (x, y) in [(a.r, b.r), (a.g, b.g), (a.b, b.b)] {
                most = most.max((x.to_f64() - y.to_f64()).abs());
            }
        }
    }
    let steps = (most * 127.5 + 0.5)
        .floor()
        .clamp(1.0, f64::from(MAX_STEPS)) as u16;
    within_bands(steps, stops.len())
}

fn within_bands(steps: u16, stops: usize) -> u16 {
    let pairs = stops.saturating_sub(1).max(1);
    let most = (MAX_BANDS / pairs).clamp(1, usize::from(MAX_STEPS));
    steps.min(u16::try_from(most).unwrap_or(MAX_STEPS)).max(1)
}

fn lower_odf(
    odf: &OdfGradient,
    frame: &Frame,
    outline: &[Path],
    notes: &mut Vec<Approximation>,
) -> Result<Fill, PassError> {
    let place = Placement::of(frame);
    let mut bounds = page_bounds(outline, &place);
    if bounds.has_no_area() {
        bounds = page_bounds(&[box_path(place.w, place.h)?], &place);
    }
    let (bw, bh) = (bounds.x1 - bounds.x0, bounds.y1 - bounds.y0);
    let border = odf.border.to_f64();
    let alpha = odf.angle.to_f64() * DEG;
    let (sa, ca) = (slidock_math::sin(alpha), slidock_math::cos(alpha));
    let mid = ((bounds.x0 + bounds.x1) / 2.0, (bounds.y0 + bounds.y1) / 2.0);
    let centre = (
        bounds.x0 + odf.center.x.to_f64() * bw,
        bounds.y0 + odf.center.y.to_f64() * bh,
    );
    let steps = odf.steps.map(|n| within_bands(n, odf.stops.len()));
    let u = |p: (f64, f64)| place.u_of(p.0, p.1);
    let axis = |x: f64, y: f64| -> Result<Point, PassError> {
        let (x, y) = place.u_vector(x, y);
        point(x, y)
    };
    match odf.style {
        OdfStyle::Linear | OdfStyle::Axial => {
            // s = ((p − c)·d)/E + ½ along d = (sin α, cos α); t(u) is affine, so it is one line from s = 0.
            let extent = bw * sa.abs() + bh * ca.abs();
            let extent = if extent > 0.0 { extent } else { 1.0 };
            let from = u((mid.0 - extent / 2.0 * sa, mid.1 - extent / 2.0 * ca));
            let (vx, vy) = place.back(sa, ca);
            let (gx, gy) = (place.w * vx / extent, place.h * vy / extent);
            let norm = gx * gx + gy * gy;
            let to = (from.0 + gx / norm, from.1 + gy / norm);
            let stops = if odf.style == OdfStyle::Linear {
                moved_stops(&odf.stops, |o| border + o * (1.0 - border))?
            } else {
                axial_stops(&odf.stops, border, notes)?
            };
            Ok(Fill::Linear(LinearLowered {
                from: point(from.0, from.1)?,
                to: point(to.0, to.1)?,
                stops,
                steps,
            }))
        }
        OdfStyle::Radial => {
            let r = length(bw, bh) / 2.0 * (1.0 - border);
            let (lowered, _) = radial(
                u(centre),
                (r, r),
                None,
                (place.w, place.h),
                reversed(&odf.stops)?,
                steps,
            )?;
            Ok(Fill::Radial(lowered))
        }
        OdfStyle::Ellipsoid => {
            let k = std::f64::consts::SQRT_2 / 2.0 * (1.0 - border);
            let (rx, ry) = (k * bw, k * bh);
            let rm = rx.min(ry);
            let inner = |r: f64| if r > 0.0 { (r - rm) / r } else { 0.0 };
            let stops = reversed(&odf.stops)?;
            Ok(Fill::Ellipse(Box::new(EllipseLowered {
                origin: {
                    let (x, y) = u(centre);
                    point(x, y)?
                },
                axis_x: axis(rx * ca, -rx * sa)?,
                axis_y: axis(ry * sa, ry * ca)?,
                inner: Semi {
                    rx: unit_ratio(inner(rx))?,
                    ry: unit_ratio(inner(ry))?,
                },
                steps: steps.unwrap_or_else(|| automatic_steps(&odf.stops)),
                stops,
            })))
        }
        OdfStyle::Square | OdfStyle::Rectangular => {
            let turn = ca.abs() + sa.abs();
            let (hx, hy) = if odf.style == OdfStyle::Square {
                let side = bw.max(bh) * turn / 2.0 * (1.0 - border);
                (side, side)
            } else {
                (
                    (bw * ca.abs() + bh * sa.abs()) / 2.0 * (1.0 - border),
                    (bh * ca.abs() + bw * sa.abs()) / 2.0 * (1.0 - border),
                )
            };
            let hm = hx.min(hy);
            let inner = |half: f64| if half > 0.0 { (half - hm) / half } else { 0.0 };
            let (ix, iy) = (inner(hx), inner(hy));
            Ok(Fill::Rect(Box::new(RectLowered {
                origin: {
                    let (x, y) = u(centre);
                    point(x, y)?
                },
                axis_x: axis(hx * ca, -hx * sa)?,
                axis_y: axis(hy * sa, hy * ca)?,
                inner: Edges {
                    l: ratio(-ix)?,
                    t: ratio(-iy)?,
                    r: ratio(ix)?,
                    b: ratio(iy)?,
                },
                stops: reversed(&odf.stops)?,
                steps,
            })))
        }
    }
}

/// The box a gradient's fractions are found in: `frame`, a side of 0 taking the other side's length, and 1 × 1 pt when
/// both are 0. A box without area paints nothing, and a table cell declared 0 high grows with its text and stretches
/// the fractions with it, so a stand-in side only has to keep them finite.
fn with_area(frame: &Frame) -> Result<Frame, PassError> {
    let side = |x: f64| Size::quantise(x).map_err(|e| unit_error("A gradient box", e));
    let (w, h) = (frame.w.to_f64(), frame.h.to_f64());
    if w > 0.0 && h > 0.0 {
        return Ok(frame.clone());
    }
    let (w, h) = match (w > 0.0, h > 0.0) {
        (true, _) => (w, w),
        (_, true) => (h, h),
        _ => (1.0, 1.0),
    };
    Ok(Frame {
        w: side(w)?,
        h: side(h)?,
        ..frame.clone()
    })
}

/// A fill in its frame-local form, for a box placed by `frame` whose drawn fill outline is `outline` (frame-local
/// paths; the box itself when absent). Fills already lowered, and fills that are not gradients, are kept.
pub fn lower_fill(
    fill: &Fill,
    frame: &Frame,
    outline: Option<&[Path]>,
) -> Result<LoweredFill, PassError> {
    let frame = &with_area(frame)?;
    let mut approximations = Vec::new();
    let fill = match fill {
        Fill::LinearAngle(linear) => lower_angle(linear, frame)?,
        Fill::RadialSource(radial) => {
            let (cx, cy) = (radial.center.x.to_f64(), radial.center.y.to_f64());
            let dx = cx.max(1.0 - cx);
            let dy = cy.max(1.0 - cy);
            Fill::Radial(RadialLowered {
                center: radial.center.clone(),
                radius: ratio((dx * dx + dy * dy).sqrt())?,
                radius_y: None,
                focus: None,
                stops: radial.stops.clone(),
                steps: None,
            })
        }
        Fill::Path(path) => lower_path(path, frame, &mut approximations)?,
        Fill::Odf(odf) => {
            let own;
            let outline = match outline {
                Some(paths) => paths,
                None => {
                    own = [box_path(frame.w.to_f64(), frame.h.to_f64())?];
                    &own[..]
                }
            };
            lower_odf(odf, frame, outline, &mut approximations)?
        }
        other => other.clone(),
    };
    Ok(LoweredFill {
        fill,
        approximations,
    })
}

/// ODF's linear and axial stops as an angled cell fill holds them: over the cell's corner-to-corner line, which is
/// ODF's extent in an unturned box. The angle is clockwise from the x axis in the box as drawn.
pub(crate) fn odf_angled(
    odf: &OdfGradient,
    frame: &Frame,
    notes: &mut Vec<Approximation>,
) -> Result<Option<LinearAngled<Color>>, PassError> {
    if odf.steps.is_some() || !matches!(odf.style, OdfStyle::Linear | OdfStyle::Axial) {
        return Ok(None);
    }
    let border = odf.border.to_f64();
    let stops = if odf.style == OdfStyle::Linear {
        moved_stops(&odf.stops, |o| border + o * (1.0 - border))?
    } else {
        axial_stops(&odf.stops, border, notes)?
    };
    let page = normal_angle(90.0 - odf.angle.to_f64())?;
    Ok(Some(LinearAngled {
        angle: box_angle(page, false, frame)?,
        stops,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_ends_span_the_box_corner_to_corner() {
        let (from, to) = linear_ends(1.0, 1.0).unwrap();
        assert_eq!(
            [from.x, from.y, to.x, to.y].map(|r| r.to_f64()),
            [0.0, 0.0, 1.0, 1.0]
        );
        let (from, to) = linear_ends(0.0, 0.0).unwrap();
        assert_eq!(
            [from.x, from.y, to.x, to.y].map(|r| r.to_f64()),
            [0.0, 0.5, 1.0, 0.5]
        );
    }

    #[test]
    fn finds_the_extrema_of_curves_not_their_control_points() {
        // A quarter circle as one cubic: control points reach x = 1, the curve 1 too, y the same.
        assert_eq!(cubic_extrema(0.0, 1.0, 1.0, 0.0), vec![0.5]);
        assert!((cubic_at(0.0, 1.0, 1.0, 0.0, 0.5) - 0.75).abs() < 1e-12);
        assert_eq!(quad_extrema(0.0, 2.0, 0.0), vec![0.5]);
        assert!(quad_extrema(0.0, 1.0, 2.0).is_empty());
    }

    #[test]
    fn divides_the_focus_rectangle_as_it_divides_the_tile() {
        // fillToRect l10 t30 r80 b20 in the box: (0.11111, 0.6).
        assert!((focus_axis(0.0, 1.0, 0.1, 0.1) - 0.1 / 0.9).abs() < 1e-12);
        assert!((focus_axis(0.0, 1.0, 0.3, 0.5) - 0.6).abs() < 1e-12);
        // A focus rectangle as large as the tile: its centre.
        assert_eq!(focus_axis(0.0, 1.0, 0.0, 1.0), 0.5);
    }
}
