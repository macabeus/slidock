//! Colours, fills and strokes from their archives, as IR values. Colours stay in the space they were written in
//! (sRGB or Display P3); CMYK converts naively, as Keynote stores no profile with it. Defaults are the archives' own.

use slidock_color::{blending_space, components_in, lerp};
use slidock_ir::color::{Color, ColorSpace, RgbColor};
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::features::FeatureId;
use slidock_ir::ids::NodeId;
use slidock_ir::paint::{
    Cap, Dash, DashPreset, Fill, GradientStop, Join, LineEnd, LineEndPath, LinearAngle,
    LinearLowered, MAX_STOPS, PathTypeTag, Point, RadialLowered, SourceStroke, Stretch,
    StrokePaint,
};
use slidock_ir::shadow::{Shadow, SourceShadow};
use slidock_ir::units::{Ratio, UnitRatio};
use slidock_math::{cos, sin};

use crate::context::{DataId, Lowering};
use crate::num::{
    clamp, finite_or, js_max, js_min, normalised_angle, pt, ratio, size, unit_ratio, wide,
};
use crate::paths::line_end_segments;
use crate::proto::{tsd, tsp};

/// A component in [0, 1].
fn component(x: f64) -> UnitRatio {
    unit_ratio(clamp(x, 0.0, 1.0))
}

/// `TSP.Color.ColorModel`.
const CMYK: i32 = 2;
const WHITE: i32 = 3;
/// `TSP.Color.RGBColorSpace`.
const P3: i32 = 2;

pub fn color(archive: Option<&tsp::Color>) -> Option<RgbColor> {
    let archive = archive?;
    let get = |x: Option<f32>| wide(x).unwrap_or(0.0);
    let a = component(wide(archive.a).unwrap_or(1.0));
    Some(match archive.model {
        Some(WHITE) => {
            let w = component(get(archive.w));
            RgbColor::new(ColorSpace::Srgb, w, w, w, a)
        }
        Some(CMYK) => {
            let k = get(archive.k);
            let ink = |value: Option<f32>| component((1.0 - get(value)) * (1.0 - k));
            RgbColor::new(
                ColorSpace::Srgb,
                ink(archive.c),
                ink(archive.m),
                ink(archive.y),
                a,
            )
        }
        _ => RgbColor::new(
            if archive.rgbspace == Some(P3) {
                ColorSpace::P3
            } else {
                ColorSpace::Srgb
            },
            component(get(archive.r)),
            component(get(archive.g)),
            component(get(archive.b)),
            a,
        ),
    })
}

pub fn black() -> RgbColor {
    let zero = UnitRatio::ZERO;
    RgbColor::new(ColorSpace::Srgb, zero, zero, zero, unit_ratio(1.0))
}

/// A colour as the IR writes it, black when the archive is absent.
pub fn color_or_black(archive: Option<&tsp::Color>) -> Color {
    Color::Rgb(color(archive).unwrap_or_else(black))
}

/// `GradientArchive.GradientType`.
const LINEAR: i32 = 0;
const RADIAL: i32 = 1;

/// How far Keynote pushes out every side of a shape's or background's box before an angle gradient spans it, in
/// points (its PDF export: G01's 600 pt box spans 602 pt, G42's 20 pt bar 22 pt, a background 962 × 542 pt).
const ANGLE_OUTSET: f64 = 1.0;

/// Where a fill is drawn, which decides how Keynote's gradients span it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FillBox {
    /// A shape's natural box or the slide, `w` × `h` points.
    Frame { w: f64, h: f64 },
    /// A table cell, which Keynote paints with `paintRect:inContext:`: an angle gradient spans the cell itself and a
    /// radial one is the cell's own ellipse.
    Cell,
}

/// A stop as the archive holds it: its fraction, its colour (opacity folded into the alpha) and the inflection of
/// the segment that starts at it.
struct SourceStop {
    fraction: f64,
    color: RgbColor,
    inflection: f64,
}

/// The stops in fraction order, at most [`MAX_STOPS`], each colour's alpha times the gradient's `opacity`.
fn source_stops(gradient: &tsd::GradientArchive) -> Vec<SourceStop> {
    let opacity = clamp(finite_or(wide(gradient.opacity), 1.0), 0.0, 1.0);
    let mut stops: Vec<SourceStop> = gradient
        .stops
        .iter()
        .take(MAX_STOPS)
        .map(|stop| {
            let mut color = color(stop.color.as_ref()).unwrap_or_else(black);
            color.a = component(color.a.to_f64() * opacity);
            SourceStop {
                fraction: clamp(finite_or(wide(stop.fraction), 0.0), 0.0, 1.0),
                color,
                inflection: finite_or(wide(stop.inflection), 0.5),
            }
        })
        .collect();
    stops.sort_by(|a, b| a.fraction.total_cmp(&b.fraction));
    stops
}

/// The stops as the IR draws them. Keynote's inflection bends a segment into two straight ones that meet, at
/// `inflection` of the way along it, in the even mix of its ends (in the gradient's blending space, alpha averaged;
/// retro's 0.3914 fits Keynote's export at 0.0013 alpha). A segment of inflection ½ is already straight. Midpoints
/// past [`MAX_STOPS`] are left out; the second value counts them.
fn ir_stops(stops: &[SourceStop]) -> (Vec<GradientStop<Color>>, usize) {
    let space = blending_space(stops.iter().map(|stop| &stop.color));
    let stop = |pos: f64, color: RgbColor| GradientStop {
        pos: component(pos),
        color: Color::Rgb(color),
    };
    let mut room = MAX_STOPS.saturating_sub(stops.len());
    let mut dropped = 0;
    let mut out = Vec::with_capacity(stops.len());
    for (i, at) in stops.iter().enumerate() {
        out.push(stop(at.fraction, at.color.clone()));
        let Some(next) = stops.get(i + 1) else {
            continue;
        };
        let span = next.fraction - at.fraction;
        if (at.inflection - 0.5).abs() <= INFLECTION_TOLERANCE || span <= 0.0 {
            continue;
        }
        if room == 0 {
            dropped += 1;
            continue;
        }
        room -= 1;
        let [r, g, b, a] = lerp(
            components_in(&at.color, space),
            components_in(&next.color, space),
            0.5,
        );
        let mix = RgbColor::new(
            space,
            component(r),
            component(g),
            component(b),
            component(a),
        );
        out.push(stop(
            at.fraction + span * clamp(at.inflection, 0.0, 1.0),
            mix,
        ));
    }
    (out, dropped)
}

/// An inflection this close to ½ draws a straight segment.
const INFLECTION_TOLERANCE: f64 = 1e-4;

/// A fraction the IR holds, or `None` when it is out of its range.
fn fraction(x: f64) -> Option<Ratio> {
    Ratio::quantise(x).ok().filter(|r| r.in_range())
}

fn point(x: f64, y: f64) -> Option<Point> {
    Some(Point {
        x: fraction(x)?,
        y: fraction(y)?,
    })
}

/// A gradient as the IR draws it, and the reasons it is drawn otherwise than Keynote draws it (KN1301).
struct Drawn {
    fill: Fill,
    reasons: Vec<String>,
}

/// Which geometry a gradient draws: `+[TSDGradient instanceWithArchive:]` makes a `TSDAngleGradient` when the
/// archive holds an angle, else a `TSDTransformGradient` when it holds a transform. `advancedGradient` is the
/// inspector's flag and plays no part.
enum Geometry<'a> {
    Angle(f64),
    Transform(&'a tsd::TransformGradientArchive),
}

/// Keynote's gradient over `place`, from its sorted stops and its type. Shapes and backgrounds: an angle linear
/// gradient spans the box pushed out 1 pt (lowered with the frame, as DrawingML's angle is); an angle radial one is
/// a real circle through the outset box's corners whose centre turns with the angle about the point between the
/// box's centre and its outset bottom-right corner (the centre at 315°, the corner at 135°: G18–G23); a transform
/// gradient is the line or circle from its start to its end in its base box, stretched with the frame. Cells: an
/// angle spans the cell, a transform draws top to bottom, and a radial gradient is the cell's centred ellipse
/// through its corners (G38).
fn gradient(
    archive: &tsd::GradientArchive,
    stops: Vec<GradientStop<Color>>,
    place: FillBox,
) -> Drawn {
    let mut reasons = Vec::new();
    let kind = archive.r#type.unwrap_or(LINEAR);
    if kind != LINEAR && kind != RADIAL {
        reasons.push(format!("unknown gradient type {kind}"));
        return Drawn {
            fill: Fill::None,
            reasons,
        };
    }
    let radial = kind == RADIAL;
    let geometry = if let Some(angle) = &archive.anglegradient {
        Geometry::Angle(finite_or(wide(angle.gradientangle), 0.0))
    } else if let Some(transform) = &archive.transformgradient {
        Geometry::Transform(transform)
    } else {
        reasons.push("it has neither an angle nor a transform".to_owned());
        Geometry::Angle(0.0)
    };
    let fill = match (place, geometry) {
        (FillBox::Cell, _) if radial => Fill::Radial(cell_radial(stops)),
        (FillBox::Cell, Geometry::Angle(radians)) => angle_linear(radians, None, stops),
        (FillBox::Cell, Geometry::Transform(_)) => {
            angle_linear(-std::f64::consts::FRAC_PI_2, None, stops)
        }
        (FillBox::Frame { w, h }, Geometry::Angle(radians)) => {
            if radial {
                Fill::Radial(angle_radial(radians, w, h, stops))
            } else {
                angle_linear(radians, Some(ANGLE_OUTSET), stops)
            }
        }
        (FillBox::Frame { w, h }, Geometry::Transform(transform)) => {
            match transformed(transform, radial, &stops) {
                Ok(fill) => fill,
                Err(Degenerate::Base) => {
                    reasons.push("its base size is empty".to_owned());
                    if radial {
                        Fill::Radial(angle_radial(0.0, w, h, stops))
                    } else {
                        angle_linear(0.0, Some(ANGLE_OUTSET), stops)
                    }
                }
                Err(Degenerate::Ends) => {
                    reasons.push("its start and end points coincide".to_owned());
                    let last = stops.last().map(|stop| stop.color.clone());
                    last.map_or(Fill::None, |color| Fill::Solid { color })
                }
            }
        }
    };
    Drawn { fill, reasons }
}

/// Keynote's counter-clockwise angle in radians as the IR's clockwise one in degrees; the angle holds in the box's
/// real proportions and turns with it.
fn angle_linear(radians: f64, outset: Option<f64>, stops: Vec<GradientStop<Color>>) -> Fill {
    Fill::LinearAngle(LinearAngle {
        angle: normalised_angle(-(radians * 180.0) / std::f64::consts::PI),
        scaled: false,
        rotate_with_shape: true,
        outset: outset.map(size),
        tile: None,
        stops,
    })
}

fn half() -> Ratio {
    ratio(0.5)
}

/// The cell's centred ellipse through its corners: semi-axes W/√2 and H/√2.
fn cell_radial(stops: Vec<GradientStop<Color>>) -> RadialLowered<Color> {
    RadialLowered {
        center: Point {
            x: half(),
            y: half(),
        },
        radius: ratio(std::f64::consts::FRAC_1_SQRT_2),
        radius_y: None,
        focus: None,
        stops,
        steps: None,
    }
}

/// An angle radial gradient over a `w` × `h` box: R = ½·hypot(W + 2, H + 2); its centre is the box's centre turned
/// by 7π/4 − `radians` (y down) about P, halfway between the centre and the outset bottom-right corner (W + 1, H + 1).
fn angle_radial(
    radians: f64,
    w: f64,
    h: f64,
    stops: Vec<GradientStop<Color>>,
) -> RadialLowered<Color> {
    if !(w > 0.0 && h > 0.0) {
        return cell_radial(stops);
    }
    let (ow, oh) = (w + 2.0 * ANGLE_OUTSET, h + 2.0 * ANGLE_OUTSET);
    let r = (ow * ow + oh * oh).sqrt() / 2.0;
    let mid = (w / 2.0, h / 2.0);
    let corner = (w + ANGLE_OUTSET, h + ANGLE_OUTSET);
    let p = ((mid.0 + corner.0) / 2.0, (mid.1 + corner.1) / 2.0);
    let turn = 7.0 * std::f64::consts::FRAC_PI_4 - radians;
    let (c, s) = (cos(turn), sin(turn));
    let (vx, vy) = (mid.0 - p.0, mid.1 - p.1);
    let centre = (p.0 + c * vx - s * vy, p.1 + s * vx + c * vy);
    let radius = ratio(r / w);
    let radius_y = ratio(r / h);
    RadialLowered {
        center: Point {
            x: ratio(centre.0 / w),
            y: ratio(centre.1 / h),
        },
        radius,
        radius_y: (radius_y != radius).then_some(radius_y),
        focus: None,
        stops,
        steps: None,
    }
}

/// Why a transform gradient cannot be drawn as stated.
enum Degenerate {
    Base,
    Ends,
}

/// A transform gradient in frame fractions. Its start s and end e are points of its base box (bw × bh), which
/// stretches with the frame. Linear: its iso-lines are perpendicular to d = e − s in the base box, so t is affine in
/// u with gradient G = (bw·d.x, bh·d.y)/|d|², from = s/(bw, bh), to = from + G/|G|². Radial: a circle of radius |d|
/// about s in the base box, an ellipse in fractions.
fn transformed(
    transform: &tsd::TransformGradientArchive,
    radial: bool,
    stops: &[GradientStop<Color>],
) -> Result<Fill, Degenerate> {
    let base = transform.base_natural_size.as_ref();
    let bw = finite_or(base.and_then(|b| wide(b.width)), 0.0);
    let bh = finite_or(base.and_then(|b| wide(b.height)), 0.0);
    if !(bw > 0.0 && bh > 0.0) {
        return Err(Degenerate::Base);
    }
    let at = |p: Option<&tsp::Point>| {
        (
            finite_or(p.and_then(|p| wide(p.x)), 0.0),
            finite_or(p.and_then(|p| wide(p.y)), 0.0),
        )
    };
    let (s, e) = (at(transform.start.as_ref()), at(transform.end.as_ref()));
    let d = (e.0 - s.0, e.1 - s.1);
    let length2 = d.0 * d.0 + d.1 * d.1;
    if length2 <= 0.0 || !length2.is_finite() {
        return Err(Degenerate::Ends);
    }
    let from = point(s.0 / bw, s.1 / bh).ok_or(Degenerate::Ends)?;
    if radial {
        let length = length2.sqrt();
        let radius = fraction(length / bw).filter(|r| r.steps() > 0);
        let radius_y = fraction(length / bh).filter(|r| r.steps() > 0);
        let (Some(radius), Some(radius_y)) = (radius, radius_y) else {
            return Err(Degenerate::Ends);
        };
        return Ok(Fill::Radial(RadialLowered {
            center: from,
            radius,
            radius_y: (radius_y != radius).then_some(radius_y),
            focus: None,
            stops: stops.to_vec(),
            steps: None,
        }));
    }
    let g = (bw * d.0 / length2, bh * d.1 / length2);
    let g2 = g.0 * g.0 + g.1 * g.1;
    let to = point(s.0 / bw + g.0 / g2, s.1 / bh + g.1 / g2)
        .filter(|to| *to != from)
        .ok_or(Degenerate::Ends)?;
    Ok(Fill::Linear(LinearLowered {
        from,
        to,
        stops: stops.to_vec(),
        steps: None,
    }))
}

/// Image fill techniques: 0 natural size, 1 stretch, 2 tile, 3 scale to fill, 4 scale to fit.
const STRETCH: i32 = 1;
const TILE: i32 = 2;
const TECHNIQUES: [&str; 5] = ["natural size", "stretch", "tile", "fill", "fit"];

/// A fill as the IR writes it over `place`; the caller records the feature, since a fill is a slide's background, a
/// shape's or a cell's. A gradient is drawn as [`gradient`] says, with its inflections as stops; what it cannot draw
/// as Keynote does is reported with the reason (KN1301). Image fills stretch over the box: other techniques are
/// approximated (KN1305), a tint is dropped (KN1302), and a missing image becomes `none` (KN1306).
pub fn fill(
    archive: Option<&tsd::FillArchive>,
    place: FillBox,
    ctx: &mut Lowering<'_>,
    node: Option<&NodeId>,
) -> Option<Fill> {
    let archive = archive?;
    let site = || DiagnosticSite::new().node(node.cloned());
    if archive.color.is_some() {
        return Some(Fill::Solid {
            color: color_or_black(archive.color.as_ref()),
        });
    }
    if let Some(archive) = &archive.gradient {
        let sorted = source_stops(archive);
        if sorted.is_empty() {
            return Some(Fill::None);
        }
        let (stops, dropped) = ir_stops(&sorted);
        let mut drawn = gradient(archive, stops, place);
        if dropped > 0 {
            drawn.reasons.push(format!(
                "{dropped} midpoints do not fit in {MAX_STOPS} stops"
            ));
        }
        let feature = match archive.r#type.unwrap_or(LINEAR) {
            LINEAR => Some(FeatureId::FillLinear),
            RADIAL => Some(FeatureId::FillRadial),
            _ => None,
        };
        for reason in drawn.reasons {
            let mut at = site().data("reason", reason);
            if let Some(feature) = feature {
                at = at.feature(feature);
            }
            ctx.report(codes::KN1301, at);
        }
        return Some(drawn.fill);
    }
    let image = archive.image.as_ref();
    let Some(data) = image.and_then(|image| DataId::of(image.imagedata.as_ref())) else {
        return Some(Fill::None);
    };
    let Some(media) = ctx.media.media(Some(data)) else {
        ctx.report(codes::KN1306, site().data("data", data.to_string()));
        return Some(Fill::None);
    };
    let image = image?;
    let technique = image.technique.unwrap_or(0);
    if technique == TILE {
        ctx.report(
            codes::KN1305,
            site()
                .feature(FeatureId::FillTile)
                .data("technique", "tile"),
        );
    } else if technique != STRETCH {
        let name = usize::try_from(technique)
            .ok()
            .and_then(|i| TECHNIQUES.get(i))
            .map_or_else(|| technique.to_string(), |name| (*name).to_owned());
        ctx.report(codes::KN1305, site().data("technique", name));
    }
    if image.tint.is_some() {
        ctx.report(codes::KN1302, site().feature(FeatureId::ImageRecolor));
    }
    Some(Fill::Image {
        media,
        mode: Stretch,
    })
}

/// `StrokeArchive.LineCap` and `TSD.LineJoin`, by value.
const CAPS: [Cap; 3] = [Cap::Flat, Cap::Round, Cap::Square];
const JOINS: [Join; 3] = [Join::Miter, Join::Round, Join::Bevel];
/// `StrokePatternArchive` types: 0 a dash pattern, 1 solid, 2 none.
const SOLID_PATTERN: i32 = 1;
const EMPTY_PATTERN: i32 = 2;
const MAX_DASHES: usize = 32;

fn by_value<T: Copy>(table: &[T], value: Option<i32>, fallback: T) -> T {
    usize::try_from(value.unwrap_or(0))
        .ok()
        .and_then(|i| table.get(i))
        .copied()
        .unwrap_or(fallback)
}

/// A line end in its own units, along +y with its attachment at `end_point`, scaled with the stroke's width.
fn line_end(archive: Option<&tsd::LineEndArchive>) -> Option<LineEnd> {
    let archive = archive?;
    let segments = line_end_segments(archive.path.as_ref()?, archive.end_point.as_ref());
    (!segments.is_empty()).then_some(LineEnd::Path(LineEndPath {
        tag: PathTypeTag,
        segments,
        scale_with_width: true,
    }))
}

/// The line ends a stroke draws, as Keynote names them.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ends<'a> {
    pub head: Option<&'a tsd::LineEndArchive>,
    pub tail: Option<&'a tsd::LineEndArchive>,
}

/// A stroke: `None` when the archive is absent, `Some(None)` for an explicit "no stroke" (so nothing inherits one).
/// The dash pattern is in multiples of the width, like the IR's. Keynote's head line end sits at the path's end and
/// its tail at the start: the IR's head and tail are DrawingML's, the other way round.
pub fn stroke(
    archive: Option<&tsd::StrokeArchive>,
    ends: Ends<'_>,
    ctx: &mut Lowering<'_>,
) -> Option<Option<SourceStroke>> {
    let archive = archive?;
    let width = wide(archive.width).unwrap_or(1.0);
    let pattern = archive.pattern.as_ref();
    if pattern.and_then(|p| p.r#type) == Some(EMPTY_PATTERN) || width <= 0.0 {
        return Some(None);
    }
    // `pattern` is a fixed-size array; `count` says how much of it is the dash pattern.
    let dashes: Vec<Ratio> = match pattern {
        Some(p) if p.r#type == Some(SOLID_PATTERN) => Vec::new(),
        None => Vec::new(),
        Some(p) => {
            let count = (p.count.unwrap_or(0) as usize).min(MAX_DASHES);
            p.pattern
                .iter()
                .take(count)
                .map(|dash| ratio(js_max(0.0, f64::from(*dash))))
                .collect()
        }
    };
    let dashed = dashes.iter().any(|dash| dash.steps() > 0);
    let head = line_end(ends.tail);
    let tail = line_end(ends.head);
    ctx.see(FeatureId::Stroke);
    if dashed {
        ctx.see(FeatureId::StrokeDash);
    }
    if head.is_some() || tail.is_some() {
        ctx.see(FeatureId::StrokeLineEnd);
    }
    Some(Some(SourceStroke {
        width: size(js_min(width, 100_000.0)),
        fill: StrokePaint::Solid {
            color: color_or_black(archive.color.as_ref()),
        },
        dash: if dashed {
            Dash::Custom { custom: dashes }
        } else {
            Dash::Preset(DashPreset::Solid)
        },
        cap: by_value(&CAPS, archive.cap, Cap::Flat),
        join: by_value(&JOINS, archive.join, Join::Miter),
        head,
        tail,
    }))
}

/// Keynote's shadow blur: alpha falls to 0 at `radius` along a Gaussian of standard deviation 0.51 · radius (fitted to
/// Keynote's exports at radii 5, 14, 16 and 25: 0.509–0.52).
pub const SIGMA_PER_RADIUS: f64 = 0.51;
/// The largest radius whose σ the IR holds.
const MAX_RADIUS: i32 = 980;

/// `ShadowArchive.ShadowType`.
const DROP_SHADOW: i32 = 0;
const CONTACT_SHADOW: i32 = 1;
const CURVED_SHADOW: i32 = 2;

/// A shadow as a style chain leaves it.
#[derive(Clone, Debug, PartialEq)]
pub enum ShadowRead {
    /// Nothing is drawn: no shadow, a disabled or empty one, or one whose colour paints nothing.
    Off,
    On(SourceShadow),
    /// A shadow is on but the IR does not draw it, for the reason given.
    NotDrawn(String),
}

/// A shadow, on only when it says so (Keynote writes an empty or disabled shadow for "no shadow"). A drop shadow's
/// offset is `offset` points at `angle` degrees clockwise from +x on the page (y down), whatever the drawable's turn
/// or flips; its colour's alpha is the colour's times `opacity`. Every field of an enabled shadow must be present
/// and finite: the proto's defaults are never written by Keynote, so a missing one is reported, not assumed.
/// Contact and curved shadows are custom geometry the IR does not model.
pub fn shadow(archive: Option<&tsd::ShadowArchive>) -> ShadowRead {
    let Some(archive) = archive.filter(|shadow| shadow.is_enabled == Some(true)) else {
        return ShadowRead::Off;
    };
    let missing = |field: &str| ShadowRead::NotDrawn(format!("shadow without its {field}"));
    let finite = |x: Option<f32>| wide(x).filter(|x| x.is_finite());
    match archive.r#type {
        None => return missing("type"),
        Some(DROP_SHADOW) => {}
        Some(CONTACT_SHADOW) => return ShadowRead::NotDrawn("contact shadow".to_owned()),
        Some(CURVED_SHADOW) => return ShadowRead::NotDrawn("curved shadow".to_owned()),
        Some(other) => return ShadowRead::NotDrawn(format!("shadow of type {other}")),
    }
    let Some(mut rgb) = color(archive.color.as_ref()) else {
        return missing("color");
    };
    let Some(angle) = finite(archive.angle) else {
        return missing("angle");
    };
    let Some(offset) = finite(archive.offset) else {
        return missing("offset");
    };
    let Some(radius) = archive.radius else {
        return missing("radius");
    };
    let Some(opacity) = finite(archive.opacity) else {
        return missing("opacity");
    };
    if !(0..=MAX_RADIUS).contains(&radius) {
        return ShadowRead::NotDrawn(format!("shadow radius {radius}"));
    }
    rgb.a = component(rgb.a.to_f64() * clamp(opacity, 0.0, 1.0));
    if rgb.a.steps() == 0 {
        return ShadowRead::Off;
    }
    let radians = angle * std::f64::consts::PI / 180.0;
    let along = |x: f64| pt(clamp(offset * x, -100_000.0, 100_000.0));
    ShadowRead::On(Shadow {
        dx: along(cos(radians)),
        dy: along(sin(radians)),
        sigma: size(SIGMA_PER_RADIUS * f64::from(radius)),
        color: Color::Rgb(rgb),
        rotate_with_shape: false,
        overflow_casts: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn black_archive() -> tsp::Color {
        tsp::Color {
            model: Some(1),
            r: Some(0.0),
            g: Some(0.0),
            b: Some(0.0),
            a: Some(1.0),
            ..Default::default()
        }
    }

    /// A drop shadow as Keynote 14.5 writes one: every field set.
    fn archive(angle: f32, offset: f32, radius: i32, opacity: f32) -> tsd::ShadowArchive {
        tsd::ShadowArchive {
            color: Some(black_archive()),
            angle: Some(angle),
            offset: Some(offset),
            radius: Some(radius),
            opacity: Some(opacity),
            is_enabled: Some(true),
            r#type: Some(DROP_SHADOW),
        }
    }

    fn on(read: ShadowRead) -> SourceShadow {
        match read {
            ShadowRead::On(shadow) => shadow,
            other => panic!("not drawn: {other:?}"),
        }
    }

    fn offset(angle: f32, distance: f32) -> (f64, f64) {
        let s = on(shadow(Some(&archive(angle, distance, 0, 1.0))));
        (s.dx.to_f64(), s.dy.to_f64())
    }

    #[test]
    fn turns_the_offset_clockwise_from_x_on_the_page() {
        // The retro deck's shadows, measured on Keynote's exports.
        assert_eq!(offset(90.0, 3.2874), (0.0, 3.287));
        assert_eq!(offset(60.0, 3.7839), (1.892, 3.277));
        assert_eq!(offset(128.0505, 9.5422), (-5.881, 7.514));
        assert_eq!(offset(0.0, 20.0), (20.0, 0.0));
        assert_eq!(offset(180.0, 20.0), (-20.0, 0.0));
        assert_eq!(offset(270.0, 20.0), (0.0, -20.0));
        assert_eq!(offset(45.0, 20.0), (14.142, 14.142));
    }

    #[test]
    fn fits_the_blur_with_a_gaussian_of_half_the_radius_and_folds_the_opacity_into_the_alpha() {
        let s = on(shadow(Some(&archive(90.0, 3.287, 14, 0.5331))));
        assert_eq!(s.sigma.to_f64(), 7.14);
        let Color::Rgb(rgb) = &s.color else {
            panic!("{:?}", s.color)
        };
        assert_eq!(rgb.a.to_f64(), 0.5331);
        assert!(!s.rotate_with_shape);
        let mut translucent = archive(90.0, 3.287, 25, 0.5);
        translucent.color.as_mut().unwrap().a = Some(0.5);
        let s = on(shadow(Some(&translucent)));
        assert_eq!(s.sigma.to_f64(), 12.75);
        let Color::Rgb(rgb) = &s.color else {
            panic!("{:?}", s.color)
        };
        assert_eq!(rgb.a.to_f64(), 0.25);
    }

    #[test]
    fn draws_nothing_for_an_empty_a_disabled_or_an_invisible_shadow() {
        assert_eq!(shadow(None), ShadowRead::Off);
        assert_eq!(
            shadow(Some(&tsd::ShadowArchive::default())),
            ShadowRead::Off
        );
        let mut disabled = archive(90.0, 3.0, 5, 1.0);
        disabled.is_enabled = Some(false);
        assert_eq!(shadow(Some(&disabled)), ShadowRead::Off);
        assert_eq!(shadow(Some(&archive(90.0, 3.0, 5, 0.0))), ShadowRead::Off);
    }

    #[test]
    fn reports_contact_curved_unknown_and_incomplete_shadows() {
        let reason = |archive: tsd::ShadowArchive| match shadow(Some(&archive)) {
            ShadowRead::NotDrawn(reason) => reason,
            other => panic!("{other:?}"),
        };
        let with = |edit: fn(&mut tsd::ShadowArchive)| {
            let mut a = archive(90.0, 3.0, 5, 1.0);
            edit(&mut a);
            reason(a)
        };
        assert_eq!(with(|a| a.r#type = Some(1)), "contact shadow");
        assert_eq!(with(|a| a.r#type = Some(2)), "curved shadow");
        assert_eq!(with(|a| a.r#type = Some(7)), "shadow of type 7");
        assert_eq!(with(|a| a.r#type = None), "shadow without its type");
        assert_eq!(with(|a| a.color = None), "shadow without its color");
        assert_eq!(with(|a| a.angle = None), "shadow without its angle");
        assert_eq!(
            with(|a| a.offset = Some(f32::NAN)),
            "shadow without its offset"
        );
        assert_eq!(with(|a| a.radius = None), "shadow without its radius");
        assert_eq!(with(|a| a.opacity = None), "shadow without its opacity");
        assert_eq!(with(|a| a.radius = Some(-1)), "shadow radius -1");
    }
}
