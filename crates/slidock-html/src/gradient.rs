//! Gradients as SVG paint servers, and a table cell's angled gradient as a CSS ramp. Every
//! lowered gradient is a function of the box's fractions u, so it prints in user space scaled by the paint box W × H.
//! - Stops expand before printing: `steps` into flat bands (two stops each); a gradient with a P3 stop into 8
//!   sub-stops per pair blended in P3 and printed as sRGB hex where they fit, so no stop list holds a colour that
//!   switches Chromium to OKLab interpolation; a CSS ramp with alpha into 8 unpremultiplied sub-stops per pair, since
//!   CSS interpolates premultiplied and SVG does not.
//! - `linear`: a `linearGradient` from `from` to `to`. `radial`: a `radialGradient`, a circle of radius 1 about the
//!   origin mapped onto the ellipse, its focus in those units, when it states a y radius or a focus.
//! - `rect`: a `pattern` whose content is translated by the tile's corner (a pattern draws from it) and mapped into
//!   gradient space: a vertical ramp under the horizontal one, which covers the two wedges where |x| dominates. Their
//!   edges are the mitre lines, where both ramps agree, so no seam shows. With `steps`, flat bands as the ellipse's.
//! - `ellipse`: the same pattern over flat bands, outermost first, each the contour at the band's outer edge painted
//!   over the last: one element per band.

use slidock_color::{blending_space, components_in, lerp, p3_to_srgb};
use slidock_ir::color::{ColorSpace, ResolvedColor, RgbColor};
use slidock_ir::geometry::{Path, Seg};
use slidock_ir::paint::{EllipseLowered, GradientStop, Point, RectLowered, ResolvedFill};
use slidock_ir::units::UnitRatio;

use crate::color::{css_color, literal_color};
use crate::css::{fixed, percent, ratio};
use crate::error::{HtmlError, Result};
use crate::tree::{Attrs, Element, lines};

/// Sub-stops per pair of stops where the browser's interpolation would differ.
const SUB_STOPS: usize = 8;
/// How far the rect and ellipse layers reach, in gradient units.
const REACH: f64 = 1000.0;
/// How far a pattern's tile reaches past what it paints, in user units.
const TILE_OUTSET: f64 = 1.0;
/// How far outside [0, 1] a P3 colour converted to sRGB may fall and still print as sRGB hex.
const GAMUT_TOLERANCE: f64 = 0.5 / 255.0;

/// The box a fill paints: W × H user units, and the extent its paths reach (left, top, right, bottom).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintBox {
    pub w: f64,
    pub h: f64,
    pub extent: [f64; 4],
}

impl PaintBox {
    /// A box whose paths stay inside it.
    pub fn of(w: f64, h: f64) -> PaintBox {
        PaintBox {
            w,
            h,
            extent: [0.0, 0.0, w, h],
        }
    }

    /// The box grown to the points of `paths` (control points included).
    pub fn with_paths(mut self, paths: &[Path]) -> PaintBox {
        let mut grow = |x: slidock_ir::units::Pt, y: slidock_ir::units::Pt| {
            let (x, y) = (x.to_f64(), y.to_f64());
            self.extent = [
                self.extent[0].min(x),
                self.extent[1].min(y),
                self.extent[2].max(x),
                self.extent[3].max(y),
            ];
        };
        for segment in paths.iter().flat_map(|path| &path.segments) {
            match *segment {
                Seg::M(x, y) | Seg::L(x, y) => grow(x, y),
                Seg::Q(x1, y1, x, y) => {
                    grow(x1, y1);
                    grow(x, y);
                }
                Seg::C(x1, y1, x2, y2, x, y) => {
                    grow(x1, y1);
                    grow(x2, y2);
                    grow(x, y);
                }
                Seg::Z => {}
            }
        }
        self
    }
}

/// What a printed stop paints: one of the gradient's own stops, or a colour between two of them, in the blending
/// space.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Tint {
    Stop(usize),
    Mix([f64; 4]),
}

/// A printed stop: a position on the ramp and its tint.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Mark {
    at: f64,
    tint: Tint,
}

/// A flat band of the ramp: [start, end] in one tint.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Band {
    start: f64,
    end: f64,
    tint: Tint,
}

/// A gradient's stops, with their components in the space they blend in.
struct Ramp<'a> {
    stops: &'a [GradientStop<ResolvedColor>],
    space: ColorSpace,
    components: Vec<[f64; 4]>,
}

impl<'a> Ramp<'a> {
    fn new(stops: &'a [GradientStop<ResolvedColor>]) -> Ramp<'a> {
        let space = blending_space(stops.iter().map(|stop| stop.color.rgb()));
        Ramp {
            stops,
            space,
            components: stops
                .iter()
                .map(|stop| components_in(stop.color.rgb(), space))
                .collect(),
        }
    }

    fn pos(&self, i: usize) -> f64 {
        self.stops.get(i).map_or(0.0, |stop| stop.pos.to_f64())
    }

    /// The tint `t` of the way from stop `i` to stop `i + 1`.
    fn mix(&self, i: usize, t: f64) -> Tint {
        match (self.components.get(i), self.components.get(i + 1)) {
            (Some(a), Some(b)) => Tint::Mix(lerp(*a, *b, t)),
            _ => Tint::Stop(i),
        }
    }

    fn pairs(&self) -> usize {
        self.stops.len().saturating_sub(1)
    }

    /// `steps` flat bands per pair: band k of [p_i, p_{i+1}] in the tint at k/(n − 1) of the pair (n = 1: p_i's).
    /// Pairs at one position (hard stops) have none.
    fn bands(&self, steps: u16) -> Vec<Band> {
        let n = usize::from(steps.max(1));
        let mut out = Vec::with_capacity(n * self.pairs());
        for i in 0..self.pairs() {
            let (a, b) = (self.pos(i), self.pos(i + 1));
            if b <= a {
                continue;
            }
            for k in 0..n {
                let tint = if n == 1 || k == 0 {
                    Tint::Stop(i)
                } else if k == n - 1 {
                    Tint::Stop(i + 1)
                } else {
                    self.mix(i, k as f64 / (n - 1) as f64)
                };
                out.push(Band {
                    start: a + (b - a) * k as f64 / n as f64,
                    end: a + (b - a) * (k + 1) as f64 / n as f64,
                    tint,
                });
            }
        }
        out
    }

    /// The stops to print: flat bands for `steps`; `sub` sub-stops per pair; else the stops as they are.
    fn marks(&self, steps: Option<u16>, sub: bool) -> Vec<Mark> {
        let plain = || -> Vec<Mark> {
            (0..self.stops.len())
                .map(|i| Mark {
                    at: self.pos(i),
                    tint: Tint::Stop(i),
                })
                .collect()
        };
        let mut out = Vec::new();
        if let Some(steps) = steps {
            let bands = self.bands(steps);
            if bands.is_empty() {
                return plain();
            }
            for band in bands {
                out.push(Mark {
                    at: band.start,
                    tint: band.tint,
                });
                out.push(Mark {
                    at: band.end,
                    tint: band.tint,
                });
            }
        } else if sub && self.pairs() > 0 {
            for i in 0..self.pairs() {
                let (a, b) = (self.pos(i), self.pos(i + 1));
                let count = if b > a { SUB_STOPS } else { 1 };
                for j in 0..=count {
                    let tint = match j {
                        0 => Tint::Stop(i),
                        j if j == count => Tint::Stop(i + 1),
                        j => self.mix(i, j as f64 / count as f64),
                    };
                    out.push(Mark {
                        at: a + (b - a) * j as f64 / count as f64,
                        tint,
                    });
                }
            }
        } else {
            return plain();
        }
        out.dedup();
        out
    }

    /// A tint as CSS: a stop as written unless it is P3 (then as the mixes print), a mix as sRGB hex when inside
    /// sRGB's gamut, else as a `color(display-p3 …)`.
    fn color(&self, tint: Tint) -> Result<String> {
        match tint {
            Tint::Stop(i) => match self.stops.get(i) {
                Some(stop) if stop.color.rgb().space == ColorSpace::P3 => self.color(Tint::Mix(
                    self.components.get(i).copied().unwrap_or_default(),
                )),
                Some(stop) => css_color(&stop.color),
                None => Ok("none".to_owned()),
            },
            Tint::Mix(c) => {
                let srgb = match self.space {
                    ColorSpace::Srgb => [c[0], c[1], c[2]],
                    ColorSpace::P3 => p3_to_srgb([c[0], c[1], c[2]]),
                };
                let fits = srgb
                    .iter()
                    .all(|x| (-GAMUT_TOLERANCE..=1.0 + GAMUT_TOLERANCE).contains(x));
                if fits {
                    literal_color(&rgb(ColorSpace::Srgb, srgb, c[3])?)
                } else {
                    literal_color(&rgb(ColorSpace::P3, [c[0], c[1], c[2]], c[3])?)
                }
            }
        }
    }

    fn has_p3(&self) -> bool {
        self.space == ColorSpace::P3
    }

    fn has_alpha(&self) -> bool {
        self.stops
            .iter()
            .any(|stop| stop.color.rgb().a.to_f64() < 1.0)
    }
}

fn rgb(space: ColorSpace, [r, g, b]: [f64; 3], a: f64) -> Result<RgbColor> {
    let unit = |x: f64| {
        UnitRatio::quantise(x.clamp(0.0, 1.0)).map_err(|error| HtmlError::Ir(error.to_string()))
    };
    Ok(RgbColor::new(space, unit(r)?, unit(g)?, unit(b)?, unit(a)?))
}

fn stop_element(at: f64, color: String) -> Result<Element> {
    Ok(Element::new(
        "stop",
        Attrs::new()
            .val("offset", ratio(at)?)
            .val("style", format!("stop-color:{color}")),
        Vec::new(),
    ))
}

/// The SVG stops of `marks` at the offsets `place` gives.
fn stop_elements(
    ramp: &Ramp<'_>,
    marks: &[Mark],
    place: impl Fn(f64) -> f64,
) -> Result<Vec<Element>> {
    marks
        .iter()
        .map(|mark| stop_element(place(mark.at), ramp.color(mark.tint)?))
        .collect()
}

/// A table cell's angled gradient as CSS color stops (`#rrggbb 40%`), sub-stops added for P3 or alpha.
pub fn css_stops(stops: &[GradientStop<ResolvedColor>]) -> Result<String> {
    let ramp = Ramp::new(stops);
    let marks = ramp.marks(None, ramp.has_p3() || ramp.has_alpha());
    let parts = marks
        .iter()
        .map(|mark| {
            Ok(format!(
                "{} {}",
                ramp.color(mark.tint)?,
                percent(mark.at * 100.0)?
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(parts.join(", "))
}

fn gradient(tag: &'static str, attrs: Attrs, stops: Vec<Element>) -> Element {
    Element::new(tag, attrs, lines(stops))
}

fn user_space(id: &str) -> Attrs {
    Attrs::new()
        .val("id", id.to_owned())
        .val("gradientUnits", "userSpaceOnUse".to_owned())
}

/// The gradient's definitions, which `url(#id)` paints with; `None` for a fill that is not a gradient.
pub fn svg_gradient(
    fill: &ResolvedFill,
    area: &PaintBox,
    id: &str,
) -> Result<Option<Vec<Element>>> {
    let scale =
        || -> Result<String> { Ok(format!("scale({} {})", fixed(area.w)?, fixed(area.h)?)) };
    Ok(Some(match fill {
        ResolvedFill::Linear(linear) => {
            let ramp = Ramp::new(&linear.stops);
            let marks = ramp.marks(linear.steps, ramp.has_p3());
            vec![gradient(
                "linearGradient",
                user_space(id)
                    .val("gradientTransform", scale()?)
                    .val("x1", ratio(linear.from.x.to_f64())?)
                    .val("y1", ratio(linear.from.y.to_f64())?)
                    .val("x2", ratio(linear.to.x.to_f64())?)
                    .val("y2", ratio(linear.to.y.to_f64())?),
                stop_elements(&ramp, &marks, |at| at)?,
            )]
        }
        ResolvedFill::Radial(radial) => {
            let ramp = Ramp::new(&radial.stops);
            let marks = ramp.marks(radial.steps, ramp.has_p3());
            let stops = stop_elements(&ramp, &marks, |at| at)?;
            let (cx, cy) = (radial.center.x.to_f64(), radial.center.y.to_f64());
            let attrs = if radial.radius_y.is_none() && radial.focus.is_none() {
                user_space(id)
                    .val("gradientTransform", scale()?)
                    .val("cx", ratio(cx)?)
                    .val("cy", ratio(cy)?)
                    .val("r", ratio(radial.radius.to_f64())?)
            } else {
                let (rx, ry) = (radial.radius.to_f64(), radial.ry().to_f64());
                let focus = |f: &Point| -> Result<(String, String)> {
                    Ok((
                        ratio((f.x.to_f64() - cx) / rx)?,
                        ratio((f.y.to_f64() - cy) / ry)?,
                    ))
                };
                let (fx, fy) = match &radial.focus {
                    Some(f) => {
                        let (x, y) = focus(f)?;
                        (Some(x), Some(y))
                    }
                    None => (None, None),
                };
                user_space(id)
                    .val("cx", "0".to_owned())
                    .val("cy", "0".to_owned())
                    .val("r", "1".to_owned())
                    .val("fx", fx)
                    .val("fy", fy)
                    .val(
                        "gradientTransform",
                        matrix([area.w * rx, 0.0, 0.0, area.h * ry, area.w * cx, area.h * cy])?,
                    )
            };
            vec![gradient("radialGradient", attrs, stops)]
        }
        ResolvedFill::Rect(rect) => rect_pattern(rect, area, id)?,
        ResolvedFill::Ellipse(ellipse) => ellipse_pattern(ellipse, area, id)?,
        ResolvedFill::None | ResolvedFill::Solid { .. } | ResolvedFill::Image { .. } => {
            return Ok(None);
        }
    }))
}

fn matrix(m: [f64; 6]) -> Result<String> {
    let parts = m.iter().map(|x| ratio(*x)).collect::<Result<Vec<_>>>()?;
    Ok(format!("matrix({})", parts.join(" ")))
}

/// The gradient space's map into user space: u = origin + g.x·axisX + g.y·axisY, scaled by the box.
fn gradient_matrix(
    origin: &Point,
    axis_x: &Point,
    axis_y: &Point,
    area: &PaintBox,
) -> Result<String> {
    let (w, h) = (area.w, area.h);
    matrix([
        w * axis_x.x.to_f64(),
        h * axis_x.y.to_f64(),
        w * axis_y.x.to_f64(),
        h * axis_y.y.to_f64(),
        w * origin.x.to_f64(),
        h * origin.y.to_f64(),
    ])
}

/// A pattern over the paint box's extent whose content is drawn in gradient space.
fn pattern(id: &str, area: &PaintBox, transform: String, content: Vec<Element>) -> Result<Element> {
    let [x0, y0, x1, y1] = area.extent;
    let (x, y) = (x0 - TILE_OUTSET, y0 - TILE_OUTSET);
    let (w, h) = (x1 - x0 + 2.0 * TILE_OUTSET, y1 - y0 + 2.0 * TILE_OUTSET);
    let mapped = Element::new(
        "g",
        Attrs::new().val("transform", transform),
        lines(content),
    );
    // A pattern's content starts at its tile's corner: drawn back by it, it lands in the user space of what it fills.
    let placed = Element::new(
        "g",
        Attrs::new().val(
            "transform",
            format!("translate({} {})", ratio(-x)?, ratio(-y)?),
        ),
        lines(vec![mapped]),
    );
    Ok(Element::new(
        "pattern",
        Attrs::new()
            .val("id", id.to_owned())
            .val("patternUnits", "userSpaceOnUse".to_owned())
            .val("x", ratio(x)?)
            .val("y", ratio(y)?)
            .val("width", ratio(w)?)
            .val("height", ratio(h)?),
        lines(vec![placed]),
    ))
}

/// The square that covers gradient space as far as the layers reach.
fn reach_rect(style: String) -> Result<Element> {
    Ok(Element::new(
        "rect",
        Attrs::new()
            .val("x", ratio(-REACH)?)
            .val("y", ratio(-REACH)?)
            .val("width", ratio(2.0 * REACH)?)
            .val("height", ratio(2.0 * REACH)?)
            .val("style", style),
        Vec::new(),
    ))
}

/// A unit vector along (x, y), or along `fallback` when both are 0.
fn unit(x: f64, y: f64, fallback: (f64, f64)) -> (f64, f64) {
    let (x, y) = if x == 0.0 && y == 0.0 {
        fallback
    } else {
        (x, y)
    };
    let length = (x * x + y * y).sqrt();
    (x / length, y / length)
}

fn points_data(points: &[(f64, f64)]) -> Result<String> {
    let mut out = String::new();
    for (i, (x, y)) in points.iter().enumerate() {
        out.push_str(if i == 0 { "M" } else { "L" });
        out.push_str(&format!("{} {}", ratio(*x)?, ratio(*y)?));
    }
    out.push('Z');
    Ok(out)
}

/// `rect`: t = max(tx, ty), as two layers of one pattern; with `steps`, as flat bands.
fn rect_pattern(
    rect: &RectLowered<ResolvedColor>,
    area: &PaintBox,
    id: &str,
) -> Result<Vec<Element>> {
    let ramp = Ramp::new(&rect.stops);
    let (l, t, r, b) = (
        rect.inner.l.to_f64(),
        rect.inner.t.to_f64(),
        rect.inner.r.to_f64(),
        rect.inner.b.to_f64(),
    );
    let transform = || gradient_matrix(&rect.origin, &rect.axis_x, &rect.axis_y, area);
    if let Some(steps) = rect.steps {
        // The contour at s: each inner edge moved s of the way out to its outer one.
        let content = band_layers(&ramp, steps, |s, fill| {
            let (x0, x1) = (l - s * (l + 1.0), r + s * (1.0 - r));
            let (y0, y1) = (t - s * (t + 1.0), b + s * (1.0 - b));
            if x1 <= x0 || y1 <= y0 {
                return Ok(None);
            }
            Ok(Some(Element::new(
                "rect",
                Attrs::new()
                    .val("x", ratio(x0)?)
                    .val("y", ratio(y0)?)
                    .val("width", ratio(x1 - x0)?)
                    .val("height", ratio(y1 - y0)?)
                    .val("style", format!("fill:{fill}")),
                Vec::new(),
            )))
        })?;
        return Ok(vec![pattern(id, area, transform()?, content)?]);
    }
    let marks = ramp.marks(None, ramp.has_p3());
    // One ramp per axis over [−1, 1]: from the outer edge (t = 1) to the inner one (0), flat across it, and out to the
    // other outer edge.
    let axis_ramp = |near: f64, far: f64| -> Result<Vec<Element>> {
        let mut stops = Vec::with_capacity(2 * marks.len());
        for mark in marks.iter().rev() {
            let at = near - mark.at * (near + 1.0);
            stops.push(stop_element((at + 1.0) / 2.0, ramp.color(mark.tint)?)?);
        }
        for mark in &marks {
            let at = far + mark.at * (1.0 - far);
            stops.push(stop_element((at + 1.0) / 2.0, ramp.color(mark.tint)?)?);
        }
        Ok(stops)
    };
    let (x_id, y_id) = (format!("{id}-x"), format!("{id}-y"));
    let along = |id: &str, vertical: bool, stops| {
        let ends = if vertical {
            ["0", "-1", "0", "1"]
        } else {
            ["-1", "0", "1", "0"]
        };
        gradient(
            "linearGradient",
            user_space(id)
                .val("x1", ends[0].to_owned())
                .val("y1", ends[1].to_owned())
                .val("x2", ends[2].to_owned())
                .val("y2", ends[3].to_owned()),
            stops,
        )
    };
    let far = |(x, y): (f64, f64), d: (f64, f64)| (x + REACH * d.0, y + REACH * d.1);
    let (tl, bl, tr, br) = ((l, t), (l, b), (r, t), (r, b));
    let left = [
        tl,
        far(tl, unit(-(l + 1.0), -(t + 1.0), (-1.0, -1.0))),
        (-REACH, -REACH),
        (-REACH, REACH),
        far(bl, unit(-(l + 1.0), 1.0 - b, (-1.0, 1.0))),
        bl,
    ];
    let right = [
        tr,
        far(tr, unit(1.0 - r, -(t + 1.0), (1.0, -1.0))),
        (REACH, -REACH),
        (REACH, REACH),
        far(br, unit(1.0 - r, 1.0 - b, (1.0, 1.0))),
        br,
    ];
    let wedges = Element::new(
        "path",
        Attrs::new()
            .val(
                "d",
                format!("{}{}", points_data(&left)?, points_data(&right)?),
            )
            .val("style", format!("fill:url(#{x_id})")),
        Vec::new(),
    );
    let content = vec![reach_rect(format!("fill:url(#{y_id})"))?, wedges];
    Ok(vec![
        along(&x_id, false, axis_ramp(l, r)?),
        along(&y_id, true, axis_ramp(t, b)?),
        pattern(id, area, transform()?, content)?,
    ])
}

/// Flat bands, outermost first: the last stop everywhere, then each band's tint inside the contour at its outer edge
/// (`contour(s, fill)`, none when it holds no area), then the first stop inside the contour at its position.
fn band_layers(
    ramp: &Ramp<'_>,
    steps: u16,
    contour: impl Fn(f64, String) -> Result<Option<Element>>,
) -> Result<Vec<Element>> {
    let bands = ramp.bands(steps);
    let last = Tint::Stop(ramp.stops.len().saturating_sub(1));
    let mut content = vec![reach_rect(format!("fill:{}", ramp.color(last)?))?];
    for band in bands.iter().rev() {
        content.extend(contour(band.end, ramp.color(band.tint)?)?);
    }
    // Below the first band (or everywhere when no pair has a width), the first stop.
    if bands.first().is_none_or(|band| band.tint != Tint::Stop(0)) {
        content.extend(contour(ramp.pos(0), ramp.color(Tint::Stop(0))?)?);
    }
    Ok(content)
}

/// `ellipse`: flat bands, outermost first, each the ellipse at its outer edge painted over the last.
fn ellipse_pattern(
    ellipse: &EllipseLowered<ResolvedColor>,
    area: &PaintBox,
    id: &str,
) -> Result<Vec<Element>> {
    let ramp = Ramp::new(&ellipse.stops);
    let (irx, iry) = (ellipse.inner.rx.to_f64(), ellipse.inner.ry.to_f64());
    let content = band_layers(&ramp, ellipse.steps, |s, fill| {
        let (rx, ry) = (irx + s * (1.0 - irx), iry + s * (1.0 - iry));
        if rx <= 0.0 || ry <= 0.0 {
            return Ok(None);
        }
        Ok(Some(Element::new(
            "ellipse",
            Attrs::new()
                .val("cx", "0".to_owned())
                .val("cy", "0".to_owned())
                .val("rx", ratio(rx)?)
                .val("ry", ratio(ry)?)
                .val("style", format!("fill:{fill}")),
            Vec::new(),
        )))
    })?;
    Ok(vec![pattern(
        id,
        area,
        gradient_matrix(&ellipse.origin, &ellipse.axis_x, &ellipse.axis_y, area)?,
        content,
    )?])
}

#[cfg(test)]
mod tests {
    use super::*;
    use slidock_ir::color::ColorSpace;
    use slidock_ir::units::UnitRatio;

    fn color(space: ColorSpace, r: f64, g: f64, b: f64, a: f64) -> ResolvedColor {
        let u = |x: f64| UnitRatio::quantise(x).unwrap();
        ResolvedColor::Rgb(RgbColor::new(space, u(r), u(g), u(b), u(a)))
    }

    fn stop(pos: f64, color: ResolvedColor) -> GradientStop<ResolvedColor> {
        GradientStop {
            pos: UnitRatio::quantise(pos).unwrap(),
            color,
        }
    }

    #[test]
    fn steps_make_flat_bands_in_the_colour_at_k_over_n_minus_1() {
        let stops = [
            stop(0.0, color(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0)),
            stop(1.0, color(ColorSpace::Srgb, 1.0, 1.0, 1.0, 1.0)),
        ];
        let ramp = Ramp::new(&stops);
        let bands = ramp.bands(3);
        assert_eq!(bands.len(), 3);
        assert_eq!((bands[1].start, bands[1].end), (1.0 / 3.0, 2.0 / 3.0));
        assert_eq!(bands[1].tint, Tint::Mix([0.5, 0.5, 0.5, 1.0]));
        assert_eq!(bands[2].tint, Tint::Stop(1));
        assert_eq!(ramp.bands(1).first().map(|b| b.tint), Some(Tint::Stop(0)));
        // Two stops per band, at equal offsets.
        let marks = ramp.marks(Some(3), false);
        assert_eq!(marks.len(), 6);
    }

    #[test]
    fn prints_p3_gradients_as_srgb_sub_stops_where_they_fit() {
        let stops = [
            stop(0.0, color(ColorSpace::Srgb, 0.3376, 0.7577, 1.0, 1.0)),
            stop(1.0, color(ColorSpace::P3, 0.0823, 0.2228, 0.5373, 1.0)),
        ];
        let css = css_stops(&stops).unwrap();
        assert_eq!(css.split(", ").count(), SUB_STOPS + 1);
        assert!(!css.contains("display-p3"), "{css}");
        assert!(css.starts_with("#56c1ff 0%"), "{css}");
        // Pure P3 red is outside sRGB: it stays P3.
        let red = [
            stop(0.0, color(ColorSpace::P3, 1.0, 0.0, 0.0, 1.0)),
            stop(1.0, color(ColorSpace::P3, 1.0, 0.0, 0.0, 1.0)),
        ];
        assert!(css_stops(&red).unwrap().contains("color(display-p3 1 0 0)"));
    }

    #[test]
    fn adds_sub_stops_to_css_ramps_only_with_alpha_or_p3() {
        let opaque = [
            stop(0.0, color(ColorSpace::Srgb, 0.0, 1.0, 0.0, 1.0)),
            stop(1.0, color(ColorSpace::Srgb, 1.0, 0.0, 1.0, 1.0)),
        ];
        assert_eq!(css_stops(&opaque).unwrap(), "#00ff00 0%, #ff00ff 100%");
        let alpha = [
            stop(0.0, color(ColorSpace::Srgb, 0.0, 1.0, 0.0, 0.2)),
            stop(1.0, color(ColorSpace::Srgb, 1.0, 0.0, 1.0, 1.0)),
        ];
        let css = css_stops(&alpha).unwrap();
        assert_eq!(css.split(", ").count(), SUB_STOPS + 1);
        // Halfway, unpremultiplied: (0.5, 0.5, 0.5) at alpha 0.6.
        assert!(css.contains("#80808099 50%"), "{css}");
    }
}
