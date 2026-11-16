//! Fills and strokes. A lowered gradient is a function t(u) of the point u = (x/W, y/H) of the unrotated, unflipped
//! frame box, padded past both ends, and t ↦ colour through its stops. The source phase also
//! holds the source forms lower-geometry turns into those once the element's final frame is known: DrawingML's angle
//! (`linear` without ends), its path gradients (`path`), ODF's styles (`odf`), and a box-aspect `radial` without its
//! radius. The resolved phase holds only the lowered forms, with resolved colours.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::color::{Color, ResolvedColor};
use crate::de::{from_value, literal_tag, split_tag, unknown_tag};
use crate::diagnostics::codes;
use crate::geometry::Segment;
use crate::ids::Sha256;
use crate::units::{Angle, Ratio, Size, UnitRatio};
use crate::validate::{Validate, Validator};

/// A point as fractions of the frame box.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: Ratio,
    pub y: Ratio,
}

impl Validate for Point {
    fn validate(&self, v: &mut Validator) {
        v.field("x", &self.x);
        v.field("y", &self.y);
    }
}

/// Four numbers on the sides of a box: insets from its left, top, right and bottom edges (a `path` gradient's focus
/// and tile, a `linear` one's tile, as fractions of the box), or a rectangle's sides (a `rect` gradient's inner
/// rectangle, in gradient units).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edges {
    pub l: Ratio,
    pub t: Ratio,
    pub r: Ratio,
    pub b: Ratio,
}

impl Validate for Edges {
    fn validate(&self, v: &mut Validator) {
        v.field("l", &self.l);
        v.field("t", &self.t);
        v.field("r", &self.r);
        v.field("b", &self.b);
    }
}

impl Edges {
    /// Insets that leave a box of some width and height: `l + r < 1` and `t + b < 1`.
    fn validate_insets(&self, v: &mut Validator) {
        self.validate(v);
        if self.l.to_f64() + self.r.to_f64() >= 1.0 || self.t.to_f64() + self.b.to_f64() >= 1.0 {
            v.invalid(codes::IR0008, "the insets leave an empty box");
        }
    }
}

/// An ellipse's semi-axes as fractions of the outer ellipse's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Semi {
    pub rx: UnitRatio,
    pub ry: UnitRatio,
}

impl Validate for Semi {
    fn validate(&self, v: &mut Validator) {
        v.field("rx", &self.rx);
        v.field("ry", &self.ry);
    }
}

/// A colour at a position on the ramp. A gradient interpolates between neighbouring stops in Display P3 when any
/// stop is P3, in sRGB otherwise, colour and alpha separately (unpremultiplied).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GradientStop<C> {
    pub pos: UnitRatio,
    pub color: C,
}

impl<C: Validate> Validate for GradientStop<C> {
    fn validate(&self, v: &mut Validator) {
        v.field("pos", &self.pos);
        v.field("color", &self.color);
    }
}

/// The most stops a gradient holds.
pub const MAX_STOPS: usize = 64;
/// The most flat bands per pair of stops.
pub const MAX_STEPS: u16 = 256;
/// The most flat bands a gradient draws in all (`steps` × pairs of stops).
pub const MAX_BANDS: usize = 2048;

/// 1 to 64 stops, sorted by position.
fn validate_stops<C: Validate>(stops: &[GradientStop<C>], v: &mut Validator) {
    v.at("stops", |v| {
        v.array(stops.len(), 1, MAX_STOPS);
        for (i, stop) in stops.iter().enumerate() {
            v.at(i, |v| {
                stop.validate(v);
                let before = i.checked_sub(1).and_then(|j| stops.get(j));
                if before.is_some_and(|before| before.pos > stop.pos) {
                    v.invalid(codes::IR0008, "gradient stops are sorted by position");
                }
            });
        }
    });
}

/// `steps` flat bands per pair of stops: 1 to 256, and at most 2048 bands in all.
fn validate_steps(steps: Option<u16>, stops: usize, v: &mut Validator) {
    let Some(steps) = steps else { return };
    v.at("steps", |v| {
        v.int(i64::from(steps), 1, i64::from(MAX_STEPS));
        if usize::from(steps) * stops.saturating_sub(1) > MAX_BANDS {
            v.invalid(codes::IR0008, "steps × pairs of stops exceeds 2048 bands");
        }
    });
}

/// Gradient axes that span a plane: |det(axisX, axisY)| ≥ 1e-6.
fn validate_axes(axis_x: &Point, axis_y: &Point, v: &mut Validator) {
    v.field("axisX", axis_x);
    v.field("axisY", axis_y);
    let det = axis_x.x.to_f64() * axis_y.y.to_f64() - axis_x.y.to_f64() * axis_y.x.to_f64();
    if det.abs() < 1e-6 {
        v.at("axisY", |v| {
            v.invalid(codes::IR0008, "the gradient axes are parallel")
        });
    }
}

/// A ramp of flat bands: `n` per pair of stops, band k of [p_i, p_{i+1}] in the colour at k/(n − 1) of that pair
/// (n = 1: p_i's colour).
type Steps = Option<u16>;

/// t = (u − from)·(to − from)/|to − from|²: 0 at `from`, 1 at `to`, constant across the line between them. Any
/// affine ramp of u is one of these.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinearLowered<C> {
    pub from: Point,
    pub to: Point,
    pub stops: Vec<GradientStop<C>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub steps: Steps,
}

/// t is 0 at the focus and 1 on the ellipse about `center` with semi-axes `radius` × W and `radiusY` × H (a real
/// circle when radius·W = radiusY·H): the circle family lerped from the focal point to the end ellipse. Without
/// `radiusY` the y semi-axis is `radius` × H (an ellipse with the box's aspect); without `focus` it is the centre.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RadialLowered<C> {
    pub center: Point,
    pub radius: Ratio,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub radius_y: Option<Ratio>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub focus: Option<Point>,
    pub stops: Vec<GradientStop<C>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub steps: Steps,
}

impl<C> RadialLowered<C> {
    /// The y semi-axis as a fraction of H.
    pub fn ry(&self) -> Ratio {
        self.radius_y.unwrap_or(self.radius)
    }
}

/// Rectangle contours in gradient space g, which maps to u by u = origin + g.x·axisX + g.y·axisY: the contour at
/// s ∈ [0, 1] is the rectangle lerp(inner, [−1, 1]², s), side by side, and t = s; inside `inner` t = 0. That is
/// t = max(tx, ty), each the distance past the inner rectangle over the gap to ±1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RectLowered<C> {
    pub origin: Point,
    pub axis_x: Point,
    pub axis_y: Point,
    /// The rectangle where t = 0, in gradient units: −1 ≤ l ≤ r ≤ 1, −1 ≤ t ≤ b ≤ 1 (a point or a line allowed).
    pub inner: Edges,
    pub stops: Vec<GradientStop<C>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub steps: Steps,
}

/// Ellipse contours in gradient space (mapped as [`RectLowered`]'s): the contour at s is the ellipse about the
/// origin with semi-axes lerp(inner, (1, 1), s), and t = s. Its bands are not homothetic, so it is drawn in flat
/// bands only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EllipseLowered<C> {
    pub origin: Point,
    pub axis_x: Point,
    pub axis_y: Point,
    pub inner: Semi,
    pub stops: Vec<GradientStop<C>>,
    pub steps: u16,
}

/// DrawingML's linear gradient (Keynote's angle gradients and every format's table cells write it too): an angle
/// clockwise from the x axis, `scaled` and `rotateWithShape`. lower-geometry spans the gradient line across the
/// projections of the corners of the box, or of `tile`, each side pushed out by `outset` points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LinearAngle {
    pub angle: Angle,
    pub scaled: bool,
    pub rotate_with_shape: bool,
    /// Points added on every side of the box before the corners are projected (Keynote's angle gradients: 1).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub outset: Option<Size>,
    /// OOXML `tileRect` as insets in box fractions (negative is outside the box); absent is the box.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub tile: Option<Edges>,
    pub stops: Vec<GradientStop<Color>>,
}

/// A radial gradient around a centre with the box's aspect; lower-geometry reaches it to the farthest corner.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadialSource {
    pub center: Point,
    pub stops: Vec<GradientStop<Color>>,
}

/// A DrawingML path gradient's contour family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PathShape {
    Circle,
    Rect,
}

/// DrawingML's path gradient (`a:path`): stop 0 at the focus, the last stop on the circle
/// around the tile, or on the tile's rectangle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathGradient {
    pub path: PathShape,
    /// `fillToRect`: insets of the focus rectangle from the box (fractions of the box).
    pub focus: Edges,
    /// `tileRect`: insets of the tile from the box (negative is outside the box).
    pub tile: Edges,
    pub stops: Vec<GradientStop<Color>>,
}

/// ODF's `draw:style` of a gradient.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OdfStyle {
    Linear,
    Axial,
    Radial,
    Ellipsoid,
    Square,
    Rectangular,
}

impl OdfStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            OdfStyle::Linear => "linear",
            OdfStyle::Axial => "axial",
            OdfStyle::Radial => "radial",
            OdfStyle::Ellipsoid => "ellipsoid",
            OdfStyle::Square => "square",
            OdfStyle::Rectangular => "rectangular",
        }
    }
}

/// An ODF gradient (`draw:gradient`), defined over the page-aligned bounds of the drawn outline (LibreOffice 25.2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OdfGradient {
    pub style: OdfStyle,
    /// `draw:angle`, counter-clockwise; at 0 the start colour is at the top.
    pub angle: Angle,
    /// `draw:border`: the fraction of the ramp held at the start colour.
    pub border: UnitRatio,
    /// `draw:cx`/`draw:cy`: the centre as fractions of the bounds.
    pub center: Point,
    /// Start to end, intensities applied.
    pub stops: Vec<GradientStop<Color>>,
    /// `draw:gradient-step-count`; absent is smooth (ellipsoid: LibreOffice's automatic count).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub steps: Steps,
}

/// A linear gradient held by an angle, clockwise from the x axis, that holds in the box as drawn; the gradient
/// line spans it corner to corner. Table cells only: a cell's height is known once its text is laid out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinearAngled<C> {
    pub angle: Angle,
    pub stops: Vec<GradientStop<C>>,
}

impl<C: Validate> Validate for LinearLowered<C> {
    fn validate(&self, v: &mut Validator) {
        v.field("from", &self.from);
        v.field("to", &self.to);
        if self.from == self.to {
            v.at("to", |v| {
                v.invalid(codes::IR0008, "a linear gradient's ends coincide")
            });
        }
        validate_stops(&self.stops, v);
        validate_steps(self.steps, self.stops.len(), v);
    }
}

impl<C: Validate> Validate for RadialLowered<C> {
    fn validate(&self, v: &mut Validator) {
        v.field("center", &self.center);
        v.at("radius", |v| {
            self.radius.validate(v);
            v.min(self.radius.to_f64(), 0.0, true);
        });
        if let Some(radius_y) = self.radius_y {
            v.at("radiusY", |v| {
                radius_y.validate(v);
                v.min(radius_y.to_f64(), 0.0, true);
            });
        }
        if let Some(focus) = &self.focus {
            v.at("focus", |v| {
                focus.validate(v);
                let (rx, ry) = (self.radius.to_f64(), self.ry().to_f64());
                if rx > 0.0 && ry > 0.0 {
                    let dx = (focus.x.to_f64() - self.center.x.to_f64()) / rx;
                    let dy = (focus.y.to_f64() - self.center.y.to_f64()) / ry;
                    if dx * dx + dy * dy >= 1.0 {
                        v.invalid(
                            codes::IR0008,
                            "the focus lies on or outside the end ellipse",
                        );
                    }
                }
            });
        }
        validate_stops(&self.stops, v);
        validate_steps(self.steps, self.stops.len(), v);
    }
}

/// −1 ≤ lo ≤ hi ≤ 1.
fn ordered_within_unit(lo: Ratio, hi: Ratio) -> bool {
    let (lo, hi) = (lo.to_f64(), hi.to_f64());
    -1.0 <= lo && lo <= hi && hi <= 1.0
}

impl<C: Validate> Validate for RectLowered<C> {
    fn validate(&self, v: &mut Validator) {
        v.field("origin", &self.origin);
        validate_axes(&self.axis_x, &self.axis_y, v);
        v.at("inner", |v| {
            self.inner.validate(v);
            let inner = &self.inner;
            if !ordered_within_unit(inner.l, inner.r) || !ordered_within_unit(inner.t, inner.b) {
                v.invalid(
                    codes::IR0008,
                    "the inner rectangle is ordered and within [−1, 1]",
                );
            }
        });
        validate_stops(&self.stops, v);
        validate_steps(self.steps, self.stops.len(), v);
    }
}

impl<C: Validate> Validate for EllipseLowered<C> {
    fn validate(&self, v: &mut Validator) {
        v.field("origin", &self.origin);
        validate_axes(&self.axis_x, &self.axis_y, v);
        v.field("inner", &self.inner);
        validate_stops(&self.stops, v);
        validate_steps(Some(self.steps), self.stops.len(), v);
    }
}

impl Validate for LinearAngle {
    fn validate(&self, v: &mut Validator) {
        v.field("angle", &self.angle);
        v.field("outset", &self.outset);
        if let Some(tile) = &self.tile {
            v.at("tile", |v| tile.validate_insets(v));
        }
        validate_stops(&self.stops, v);
    }
}

impl Validate for RadialSource {
    fn validate(&self, v: &mut Validator) {
        v.field("center", &self.center);
        validate_stops(&self.stops, v);
    }
}

impl Validate for PathGradient {
    fn validate(&self, v: &mut Validator) {
        v.field("focus", &self.focus);
        v.at("tile", |v| self.tile.validate_insets(v));
        validate_stops(&self.stops, v);
    }
}

impl Validate for OdfGradient {
    fn validate(&self, v: &mut Validator) {
        v.field("angle", &self.angle);
        v.at("border", |v| {
            self.border.validate(v);
            if self.border.to_f64() >= 1.0 {
                v.invalid(codes::IR0008, "the border leaves no ramp");
            }
        });
        v.field("center", &self.center);
        validate_stops(&self.stops, v);
        validate_steps(self.steps, self.stops.len(), v);
    }
}

impl<C: Validate> Validate for LinearAngled<C> {
    fn validate(&self, v: &mut Validator) {
        v.field("angle", &self.angle);
        validate_stops(&self.stops, v);
    }
}

literal_tag!(
    /// An image fill's only mode.
    Stretch = "stretch"
);

/// Every fill a presentation between passes may hold: the source forms and the lowered ones. Pattern and tiled
/// fills arrive approximated by the frontend.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Fill {
    None,
    Solid {
        color: Color,
    },
    Image {
        media: Sha256,
        mode: Stretch,
    },
    #[serde(rename = "linear")]
    LinearAngle(LinearAngle),
    #[serde(rename = "radial")]
    RadialSource(RadialSource),
    Path(PathGradient),
    Odf(OdfGradient),
    Linear(LinearLowered<Color>),
    Radial(RadialLowered<Color>),
    Rect(Box<RectLowered<Color>>),
    Ellipse(Box<EllipseLowered<Color>>),
}

/// What a resolved presentation holds: frame-local gradients and resolved colours. Written by lower-geometry. The
/// contour forms are boxed so a fill stays as small as a radial one.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ResolvedFill {
    None,
    Solid { color: ResolvedColor },
    Image { media: Sha256, mode: Stretch },
    Linear(LinearLowered<ResolvedColor>),
    Radial(RadialLowered<ResolvedColor>),
    Rect(Box<RectLowered<ResolvedColor>>),
    Ellipse(Box<EllipseLowered<ResolvedColor>>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoFields {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SolidWire<C> {
    color: C,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageWire {
    media: Sha256,
    mode: Stretch,
}

fn has(payload: &Value, key: &str) -> bool {
    payload.get(key).is_some()
}

const SOURCE_KINDS: &[&str] = &[
    "none", "solid", "image", "linear", "radial", "rect", "ellipse", "path", "odf",
];
const RESOLVED_KINDS: &[&str] = &[
    "none", "solid", "image", "linear", "radial", "rect", "ellipse",
];

impl<'de> Deserialize<'de> for Fill {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (wire, payload) = split_tag(deserializer, "kind")?;
        source_fill(&wire, payload)
    }
}

fn source_fill<E: serde::de::Error>(wire: &str, payload: Value) -> Result<Fill, E> {
    Ok(match wire {
        "none" => from_value::<NoFields, E>(payload).map(|_| Fill::None)?,
        "solid" => Fill::Solid {
            color: from_value::<SolidWire<Color>, E>(payload)?.color,
        },
        "image" => {
            let ImageWire { media, mode } = from_value(payload)?;
            Fill::Image { media, mode }
        }
        "linear" if has(&payload, "from") => Fill::Linear(from_value(payload)?),
        "linear" => Fill::LinearAngle(from_value(payload)?),
        "radial" if has(&payload, "radius") => Fill::Radial(from_value(payload)?),
        "radial" => Fill::RadialSource(from_value(payload)?),
        "rect" => Fill::Rect(Box::new(from_value(payload)?)),
        "ellipse" => Fill::Ellipse(Box::new(from_value(payload)?)),
        "path" => Fill::Path(from_value(payload)?),
        "odf" => Fill::Odf(from_value(payload)?),
        other => return Err(unknown_tag(other, "kind", SOURCE_KINDS)),
    })
}

impl<'de> Deserialize<'de> for ResolvedFill {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (wire, payload) = split_tag(deserializer, "kind")?;
        resolved_fill(&wire, payload)
    }
}

fn resolved_fill<E: serde::de::Error>(wire: &str, payload: Value) -> Result<ResolvedFill, E> {
    Ok(match wire {
        "none" => from_value::<NoFields, E>(payload).map(|_| ResolvedFill::None)?,
        "solid" => ResolvedFill::Solid {
            color: from_value::<SolidWire<ResolvedColor>, E>(payload)?.color,
        },
        "image" => {
            let ImageWire { media, mode } = from_value(payload)?;
            ResolvedFill::Image { media, mode }
        }
        "linear" => ResolvedFill::Linear(from_value(payload)?),
        "radial" => ResolvedFill::Radial(from_value(payload)?),
        "rect" => ResolvedFill::Rect(Box::new(from_value(payload)?)),
        "ellipse" => ResolvedFill::Ellipse(Box::new(from_value(payload)?)),
        other => return Err(unknown_tag(other, "kind", RESOLVED_KINDS)),
    })
}

impl Fill {
    pub fn kind(&self) -> &'static str {
        match self {
            Fill::None => "none",
            Fill::Solid { .. } => "solid",
            Fill::Image { .. } => "image",
            Fill::LinearAngle(_) | Fill::Linear(_) => "linear",
            Fill::RadialSource(_) | Fill::Radial(_) => "radial",
            Fill::Path(_) => "path",
            Fill::Odf(_) => "odf",
            Fill::Rect(_) => "rect",
            Fill::Ellipse(_) => "ellipse",
        }
    }

    /// Not a source-form gradient (an angle, a radial without its radius, a path or an ODF gradient).
    pub fn is_lowered(&self) -> bool {
        !matches!(
            self,
            Fill::LinearAngle(_) | Fill::RadialSource(_) | Fill::Path(_) | Fill::Odf(_)
        )
    }
}

impl ResolvedFill {
    pub fn kind(&self) -> &'static str {
        match self {
            ResolvedFill::None => "none",
            ResolvedFill::Solid { .. } => "solid",
            ResolvedFill::Image { .. } => "image",
            ResolvedFill::Linear(_) => "linear",
            ResolvedFill::Radial(_) => "radial",
            ResolvedFill::Rect(_) => "rect",
            ResolvedFill::Ellipse(_) => "ellipse",
        }
    }

    /// A gradient, which prints as an SVG paint server.
    pub fn is_gradient(&self) -> bool {
        matches!(
            self,
            ResolvedFill::Linear(_)
                | ResolvedFill::Radial(_)
                | ResolvedFill::Rect(_)
                | ResolvedFill::Ellipse(_)
        )
    }
}

impl Validate for Fill {
    fn validate(&self, v: &mut Validator) {
        match self {
            Fill::None | Fill::Image { .. } => {}
            Fill::Solid { color } => v.field("color", color),
            Fill::LinearAngle(g) => g.validate(v),
            Fill::RadialSource(g) => g.validate(v),
            Fill::Path(g) => g.validate(v),
            Fill::Odf(g) => g.validate(v),
            Fill::Linear(g) => g.validate(v),
            Fill::Radial(g) => g.validate(v),
            Fill::Rect(g) => g.validate(v),
            Fill::Ellipse(g) => g.validate(v),
        }
    }
}

impl Validate for ResolvedFill {
    fn validate(&self, v: &mut Validator) {
        match self {
            ResolvedFill::None | ResolvedFill::Image { .. } => {}
            ResolvedFill::Solid { color } => v.field("color", color),
            ResolvedFill::Linear(g) => g.validate(v),
            ResolvedFill::Radial(g) => g.validate(v),
            ResolvedFill::Rect(g) => g.validate(v),
            ResolvedFill::Ellipse(g) => g.validate(v),
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "kind")]
enum AngledWire<'a, C> {
    #[serde(rename = "linear")]
    Linear(&'a LinearAngled<C>),
}

/// A table cell's fills: an element's, and a linear gradient held by its angle.
#[derive(Clone, Debug, PartialEq)]
pub enum CellFill {
    Fill(Fill),
    Angled(LinearAngled<Color>),
}

/// A resolved table cell's fills.
#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedCellFill {
    Fill(ResolvedFill),
    Angled(LinearAngled<ResolvedColor>),
}

impl Serialize for CellFill {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            CellFill::Fill(fill) => fill.serialize(serializer),
            CellFill::Angled(angled) => AngledWire::Linear(angled).serialize(serializer),
        }
    }
}

impl Serialize for ResolvedCellFill {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            ResolvedCellFill::Fill(fill) => fill.serialize(serializer),
            ResolvedCellFill::Angled(angled) => AngledWire::Linear(angled).serialize(serializer),
        }
    }
}

/// A `linear` cell fill with an angle and none of DrawingML's flags is an angled one.
fn is_angled(wire: &str, payload: &Value) -> bool {
    wire == "linear"
        && !has(payload, "from")
        && !has(payload, "scaled")
        && !has(payload, "rotateWithShape")
}

impl<'de> Deserialize<'de> for CellFill {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (wire, payload) = split_tag(deserializer, "kind")?;
        if is_angled(&wire, &payload) {
            from_value(payload).map(CellFill::Angled)
        } else {
            source_fill(&wire, payload).map(CellFill::Fill)
        }
    }
}

impl<'de> Deserialize<'de> for ResolvedCellFill {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (wire, payload) = split_tag(deserializer, "kind")?;
        if wire == "linear" && !has(&payload, "from") {
            from_value(payload).map(ResolvedCellFill::Angled)
        } else {
            resolved_fill(&wire, payload).map(ResolvedCellFill::Fill)
        }
    }
}

impl CellFill {
    pub fn kind(&self) -> &'static str {
        match self {
            CellFill::Fill(fill) => fill.kind(),
            CellFill::Angled(_) => "linear",
        }
    }

    pub fn is_lowered(&self) -> bool {
        match self {
            CellFill::Fill(fill) => fill.is_lowered(),
            CellFill::Angled(_) => true,
        }
    }
}

impl Validate for CellFill {
    fn validate(&self, v: &mut Validator) {
        match self {
            CellFill::Fill(fill) => fill.validate(v),
            CellFill::Angled(angled) => angled.validate(v),
        }
    }
}

impl Validate for ResolvedCellFill {
    fn validate(&self, v: &mut Validator) {
        match self {
            ResolvedCellFill::Fill(fill) => fill.validate(v),
            ResolvedCellFill::Angled(angled) => angled.validate(v),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DashPreset {
    Solid,
    Dot,
    Dash,
    LgDash,
    DashDot,
    LgDashDot,
    LgDashDotDot,
    SysDot,
    SysDash,
    SysDashDot,
    SysDashDotDot,
}

/// A preset, or dash and gap lengths as multiples of the stroke width.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Dash {
    Preset(DashPreset),
    Custom { custom: Vec<Ratio> },
}

impl<'de> Deserialize<'de> for Dash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct CustomWire {
            custom: Vec<Ratio>,
        }
        match Value::deserialize(deserializer)? {
            value @ Value::String(_) => from_value(value).map(Dash::Preset),
            value @ Value::Object(_) => {
                from_value::<CustomWire, _>(value).map(|w| Dash::Custom { custom: w.custom })
            }
            other => Err(serde::de::Error::invalid_type(
                crate::de::unexpected(&other),
                &"a dash preset or {custom}",
            )),
        }
    }
}

impl Dash {
    pub fn is_solid(&self) -> bool {
        matches!(self, Dash::Preset(DashPreset::Solid))
    }
}

impl Validate for Dash {
    fn validate(&self, v: &mut Validator) {
        if let Dash::Custom { custom } = self {
            v.at("custom", |v| {
                v.array(custom.len(), 1, 32);
                for (i, length) in custom.iter().enumerate() {
                    v.at(i, |v| {
                        length.validate(v);
                        v.min(length.to_f64(), 0.0, false);
                    });
                }
            });
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EndSize {
    Sm,
    Med,
    Lg,
}

/// A DrawingML arrowhead shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LineEndShape {
    Triangle,
    Stealth,
    Diamond,
    Oval,
    Arrow,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineEndPreset {
    #[serde(rename = "type")]
    pub shape: LineEndShape,
    pub w: EndSize,
    pub len: EndSize,
}

literal_tag!(PathTypeTag = "path");

/// A custom arrowhead (Keynote) in end-local points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LineEndPath {
    #[serde(rename = "type")]
    pub tag: PathTypeTag,
    pub segments: Vec<Segment>,
    pub scale_with_width: bool,
}

/// An arrowhead. Its tag is `type`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum LineEnd {
    Preset(LineEndPreset),
    Path(LineEndPath),
}

impl<'de> Deserialize<'de> for LineEnd {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (wire, map) = crate::de::peek_tag(deserializer, "type")?;
        match wire.as_str() {
            "path" => from_value(Value::Object(map)).map(LineEnd::Path),
            "triangle" | "stealth" | "diamond" | "oval" | "arrow" => {
                from_value(Value::Object(map)).map(LineEnd::Preset)
            }
            other => Err(unknown_tag(
                other,
                "type",
                &["triangle", "stealth", "diamond", "oval", "arrow", "path"],
            )),
        }
    }
}

impl Validate for LineEnd {
    fn validate(&self, v: &mut Validator) {
        if let LineEnd::Path(path) = self {
            v.at("segments", |v| {
                v.array(path.segments.len(), 0, 4096);
                path.segments.validate(v);
            });
        }
    }
}

/// A stroke paints with no fill or a solid colour; gradient strokes arrive approximated by the frontend.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StrokePaint<C> {
    None,
    Solid { color: C },
}

impl<'de, C: serde::de::DeserializeOwned> Deserialize<'de> for StrokePaint<C> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (wire, payload) = split_tag(deserializer, "kind")?;
        match wire.as_str() {
            "none" => from_value::<NoFields, _>(payload).map(|_| StrokePaint::None),
            "solid" => from_value::<SolidWire<C>, _>(payload)
                .map(|w| StrokePaint::Solid { color: w.color }),
            other => Err(unknown_tag(other, "kind", &["none", "solid"])),
        }
    }
}

impl<C: Validate> Validate for StrokePaint<C> {
    fn validate(&self, v: &mut Validator) {
        if let StrokePaint::Solid { color } = self {
            v.field("color", color);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Cap {
    Flat,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Join {
    Miter,
    Round,
    Bevel,
}

/// A line: width, paint, dash, cap, join, and optional line ends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    deny_unknown_fields,
    bound(deserialize = "C: serde::de::DeserializeOwned")
)]
pub struct Stroke<C> {
    pub width: Size,
    pub fill: StrokePaint<C>,
    pub dash: Dash,
    pub cap: Cap,
    pub join: Join,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub head: Option<LineEnd>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub tail: Option<LineEnd>,
}

pub type SourceStroke = Stroke<Color>;
pub type ResolvedStroke = Stroke<ResolvedColor>;

impl<C: Validate> Validate for Stroke<C> {
    fn validate(&self, v: &mut Validator) {
        v.field("width", &self.width);
        v.field("fill", &self.fill);
        v.field("dash", &self.dash);
        v.field("head", &self.head);
        v.field("tail", &self.tail);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn red() -> Value {
        json!({ "kind": "rgb", "space": "srgb", "r": 1, "g": 0, "b": 0, "a": 1 })
    }

    fn accent() -> Value {
        json!({ "kind": "theme", "slot": "accent1", "mods": [] })
    }

    fn stops() -> Value {
        json!([{ "pos": 0, "color": red() }, { "pos": 1, "color": accent() }])
    }

    fn canon<T: Serialize>(value: &T) -> String {
        crate::canonical::to_json(value).unwrap()
    }

    #[test]
    fn accepts_source_gradients_only_in_the_source_phase() {
        let linear = json!({ "kind": "linear", "angle": 90, "scaled": true, "rotateWithShape": true, "stops": stops() });
        let fill: Fill = serde_json::from_value(linear.clone()).unwrap();
        assert!(matches!(fill, Fill::LinearAngle(_)));
        assert_eq!(canon(&fill), canon(&linear));
        assert!(serde_json::from_value::<ResolvedFill>(linear).is_err());
        let radial: Fill = serde_json::from_value(
            json!({ "kind": "radial", "center": { "x": 0.5, "y": 0.5 }, "stops": stops() }),
        )
        .unwrap();
        assert!(matches!(radial, Fill::RadialSource(_)));
        let edges = json!({ "l": 0.1, "t": 0.3, "r": 0.8, "b": 0.2 });
        let none = json!({ "l": 0, "t": 0, "r": 0, "b": 0 });
        let sources = [
            json!({ "kind": "linear", "angle": 0, "scaled": false, "rotateWithShape": true, "outset": 1,
                    "tile": none, "stops": stops() }),
            json!({ "kind": "path", "path": "circle", "focus": edges, "tile": none, "stops": stops() }),
            json!({ "kind": "path", "path": "rect", "focus": edges, "tile": none, "stops": stops() }),
            json!({ "kind": "odf", "style": "ellipsoid", "angle": 30, "border": 0.2,
                    "center": { "x": 0.5, "y": 0.5 }, "stops": stops(), "steps": 4 }),
        ];
        for source in sources {
            let fill: Fill = serde_json::from_value(source.clone()).unwrap();
            assert!(!fill.is_lowered(), "{source}");
            assert_eq!(canon(&fill), canon(&source));
            let mut resolved = source.clone();
            resolved["stops"] = resolved_stops();
            assert!(
                serde_json::from_value::<ResolvedFill>(resolved.clone()).is_err(),
                "{source}"
            );
            assert!(
                serde_json::from_value::<ResolvedCellFill>(resolved).is_err(),
                "{source}"
            );
        }
    }

    fn resolved_stops() -> Value {
        let mut resolved_accent = accent();
        resolved_accent["resolved"] = red();
        json!([{ "pos": 0, "color": red() }, { "pos": 1, "color": resolved_accent }])
    }

    #[test]
    fn keeps_todays_linear_and_radial_json_byte_for_byte() {
        let linear = json!({ "kind": "linear", "from": { "x": 0, "y": 0.5 }, "to": { "x": 1, "y": 0.5 },
                             "stops": resolved_stops() });
        let radial = json!({ "kind": "radial", "center": { "x": 0.5, "y": 0.5 }, "radius": 0.25,
                             "stops": resolved_stops() });
        for fill in [linear, radial] {
            let text = crate::canonical::to_json(&fill).unwrap();
            let parsed: ResolvedFill = serde_json::from_value(fill).unwrap();
            assert_eq!(canon(&parsed), text);
        }
    }

    #[test]
    fn round_trips_every_lowered_kind_in_both_phases() {
        let kinds = [
            json!({ "kind": "linear", "from": { "x": -0.00052, "y": 0.5 }, "to": { "x": 1.00052, "y": 0.5 },
                    "stops": resolved_stops(), "steps": 3 }),
            json!({ "kind": "radial", "center": { "x": 0.5, "y": 0.5 }, "radius": 0.5, "radiusY": 1.5,
                    "focus": { "x": 0.25, "y": 0.5 }, "stops": resolved_stops() }),
            json!({ "kind": "rect", "origin": { "x": 0.5, "y": 0.5 }, "axisX": { "x": 0.5, "y": 0 },
                    "axisY": { "x": 0, "y": 0.5 }, "inner": { "l": -0.8, "t": -0.4, "r": -0.6, "b": 0.6 },
                    "stops": resolved_stops() }),
            json!({ "kind": "ellipse", "origin": { "x": 0.5, "y": 0.5 }, "axisX": { "x": 0.6, "y": -0.2 },
                    "axisY": { "x": 0.1, "y": 0.7 }, "inner": { "rx": 0.5, "ry": 0 },
                    "stops": resolved_stops(), "steps": 12 }),
        ];
        for kind in kinds {
            let source: Fill = serde_json::from_value(kind.clone()).unwrap();
            assert!(source.is_lowered());
            assert_eq!(canon(&source), canon(&kind));
            let resolved: ResolvedFill = serde_json::from_value(kind.clone()).unwrap();
            assert_eq!(canon(&resolved), canon(&kind));
            assert!(resolved.is_gradient());
            assert!(
                issues(&resolved).is_empty(),
                "{kind}: {:?}",
                issues(&resolved)
            );
        }
        // An ellipse always states its steps.
        let ellipse = json!({ "kind": "ellipse", "origin": { "x": 0.5, "y": 0.5 }, "axisX": { "x": 1, "y": 0 },
                              "axisY": { "x": 0, "y": 1 }, "inner": { "rx": 0, "ry": 0 }, "stops": resolved_stops() });
        assert!(serde_json::from_value::<ResolvedFill>(ellipse).is_err());
    }

    fn issues<T: Validate>(value: &T) -> Vec<String> {
        let mut v = Validator::new();
        value.validate(&mut v);
        v.into_issues()
            .into_iter()
            .map(|issue| format!("{} {}", issue.code, issue.path))
            .collect()
    }

    fn invalid(fill: Value) -> Vec<String> {
        issues(&serde_json::from_value::<ResolvedFill>(fill).unwrap())
    }

    #[test]
    fn bounds_gradients_beyond_their_types() {
        let linear = |from: Value, to: Value, stops: Value| json!({ "kind": "linear", "from": from, "to": to, "stops": stops });
        let p = |x: f64, y: f64| json!({ "x": x, "y": y });
        let unsorted = json!([{ "pos": 1, "color": red() }, { "pos": 0, "color": red() }]);
        assert_eq!(
            invalid(linear(p(0.0, 0.0), p(1.0, 0.0), unsorted)),
            ["IR0008 stops[1]"]
        );
        assert_eq!(
            invalid(linear(p(0.5, 0.5), p(0.5, 0.5), resolved_stops())),
            ["IR0008 to"]
        );
        let radial = |extra: Value| {
            let mut fill = json!({ "kind": "radial", "center": p(0.5, 0.5), "radius": 0.5,
                                   "stops": resolved_stops() });
            for (key, value) in extra.as_object().unwrap() {
                fill[key] = value.clone();
            }
            fill
        };
        assert_eq!(invalid(radial(json!({ "radius": 0 }))), ["IR0008 radius"]);
        assert_eq!(invalid(radial(json!({ "radiusY": 0 }))), ["IR0008 radiusY"]);
        // On the ellipse (x reaches 0.5 = rx) and outside it; inside is fine.
        assert_eq!(
            invalid(radial(json!({ "focus": p(1.0, 0.5) }))),
            ["IR0008 focus"]
        );
        assert_eq!(
            invalid(radial(json!({ "radiusY": 0.25, "focus": p(0.5, 0.8) }))),
            ["IR0008 focus"]
        );
        assert!(invalid(radial(json!({ "radiusY": 0.25, "focus": p(0.9, 0.5) }))).is_empty());
        assert_eq!(invalid(radial(json!({ "steps": 0 }))), ["IR0008 steps"]);
        assert_eq!(invalid(radial(json!({ "steps": 257 }))), ["IR0008 steps"]);
        let many: Vec<Value> = (0..64)
            .map(|i| json!({ "pos": f64::from(i) / 63.0, "color": red() }))
            .map(|mut stop| {
                stop["pos"] = json!((stop["pos"].as_f64().unwrap() * 1e5).round() / 1e5);
                stop
            })
            .collect();
        // 33 bands × 63 pairs > 2048.
        assert_eq!(
            invalid(radial(json!({ "stops": many, "steps": 33 }))),
            ["IR0008 steps"]
        );
        let rect = |axis_y: Value, inner: Value| {
            json!({ "kind": "rect", "origin": p(0.5, 0.5), "axisX": p(0.5, 0.0), "axisY": axis_y,
                    "inner": inner, "stops": resolved_stops() })
        };
        let point = json!({ "l": 0, "t": 0, "r": 0, "b": 0 });
        assert_eq!(invalid(rect(p(1.0, 0.0), point.clone())), ["IR0008 axisY"]);
        assert_eq!(
            invalid(rect(
                p(0.0, 0.5),
                json!({ "l": -1.2, "t": 0, "r": 0, "b": 0 })
            )),
            ["IR0008 inner"]
        );
        assert_eq!(
            invalid(rect(
                p(0.0, 0.5),
                json!({ "l": 0.5, "t": 0, "r": 0.2, "b": 0 })
            )),
            ["IR0008 inner"]
        );
        assert!(invalid(rect(p(0.0, 0.5), point)).is_empty());
        let source = |fill: Value| issues(&serde_json::from_value::<Fill>(fill).unwrap());
        assert_eq!(
            source(
                json!({ "kind": "odf", "style": "linear", "angle": 0, "border": 1,
                           "center": p(0.5, 0.5), "stops": stops() })
            ),
            ["IR0008 border"]
        );
        assert_eq!(
            source(
                json!({ "kind": "path", "path": "circle", "focus": { "l": 0, "t": 0, "r": 0, "b": 0 },
                           "tile": { "l": 0.6, "t": 0, "r": 0.4, "b": 0 }, "stops": stops() })
            ),
            ["IR0008 tile"]
        );
    }

    #[test]
    fn accepts_frame_local_gradients_in_both_phases_once_colours_are_resolved() {
        let resolved_stops = resolved_stops();
        let lowered = json!({ "kind": "linear", "from": { "x": 0, "y": 0.5 }, "to": { "x": 1, "y": 0.5 }, "stops": resolved_stops });
        assert!(matches!(
            serde_json::from_value::<Fill>(lowered.clone()).unwrap(),
            Fill::Linear(_)
        ));
        assert!(serde_json::from_value::<ResolvedFill>(lowered.clone()).is_ok());
        let mut unresolved = lowered;
        unresolved["stops"] = stops();
        assert!(serde_json::from_value::<ResolvedFill>(unresolved).is_err());
        let radial = json!({ "kind": "radial", "center": { "x": 0.5, "y": 0.5 }, "radius": 0.5, "stops": resolved_stops });
        assert!(serde_json::from_value::<ResolvedFill>(radial).is_ok());
    }

    #[test]
    fn holds_image_fills_by_hash() {
        assert!(
            serde_json::from_value::<Fill>(
                json!({ "kind": "image", "media": "a".repeat(64), "mode": "stretch" })
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<Fill>(
                json!({ "kind": "image", "media": "https://example.com/a.png", "mode": "stretch" })
            )
            .is_err()
        );
    }

    #[test]
    fn holds_angled_gradients_in_cells_only() {
        let angled = json!({ "kind": "linear", "angle": 90, "stops": stops() });
        let cell: CellFill = serde_json::from_value(angled.clone()).unwrap();
        assert!(matches!(cell, CellFill::Angled(_)));
        assert_eq!(canon(&cell), canon(&angled));
        assert!(serde_json::from_value::<Fill>(angled).is_err());
    }

    #[test]
    fn strokes_with_presets_custom_dashes_and_line_ends() {
        let stroke = json!({
            "width": 2, "fill": { "kind": "solid", "color": accent() }, "dash": { "custom": [3, 1] },
            "cap": "round", "join": "miter",
            "head": { "type": "triangle", "w": "med", "len": "lg" },
            "tail": { "type": "path", "segments": [["M", 0, 0], ["L", 1, 1], ["Z"]], "scaleWithWidth": true }
        });
        let parsed: SourceStroke = serde_json::from_value(stroke.clone()).unwrap();
        assert_eq!(canon(&parsed), canon(&stroke));
        assert!(serde_json::from_value::<ResolvedStroke>(stroke.clone()).is_err());
        let mut gradient = stroke;
        gradient["fill"] = json!({ "kind": "linear", "from": {}, "to": {}, "stops": stops() });
        assert!(serde_json::from_value::<SourceStroke>(gradient).is_err());
        assert!(
            serde_json::from_value::<LineEnd>(
                json!({ "type": "triangle", "w": "xl", "len": "sm" })
            )
            .is_err()
        );
        assert!(serde_json::from_value::<Dash>(json!("lgDashDot")).is_ok());
    }
}
