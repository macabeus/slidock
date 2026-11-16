//! DrawingML fills (§20.1.8): none, solid, gradient (linear or path), picture, pattern and group. Gradients stay in
//! DrawingML's own terms (angle, `scaled`, `rotWithShape`, `fillToRect` and `tileRect` insets); lower-geometry makes
//! them frame-local. A pattern becomes its foreground colour and a tiled picture a stretched
//! one, each with a lossy diagnostic. What a gradient draws otherwise is PX2106 with its reason:
//! - a `shape` path follows the box's rectangle, not the outline (PowerPoint's outline contours are unmeasured);
//! - a tile smaller than the box (any `tileRect` inset above 0) is drawn once over the tile, padded, not repeated or
//!   mirrored (`flip`); an empty tile is drawn over the box;
//! - an unknown path type draws nothing (fail closed).

use std::rc::Rc;

use slidock_core_api::lowering::LowerError;
use slidock_ir::color::Color;
use slidock_ir::diagnostics::{DiagnosticCode, DiagnosticSite, codes};
use slidock_ir::features::FeatureId;
use slidock_ir::ids::{NodeId, Sha256};
use slidock_ir::paint::{Edges, Fill, GradientStop, LinearAngle, PathGradient, PathShape, Stretch};
use slidock_ir::units::Ratio;

use crate::clip;
use crate::color::{color_element, fill_placeholder, read_color};
use crate::context::Lowering;
use crate::element::blip_media;
use crate::report::Report;
use crate::units::{angle, ratio, unit_ratio};
use crate::xml::{El, Element, Ns};

const FILL_ELEMENTS: [&str; 6] = [
    "noFill",
    "solidFill",
    "gradFill",
    "blipFill",
    "pattFill",
    "grpFill",
];

/// How paint readers report while lowering a container (on one of its elements, or on the container itself),
/// resolve pictures and fill in `grpFill`.
pub(crate) struct PaintCx<'l, 'a> {
    pub ctx: &'l mut Lowering<'a>,
    pub node: Option<NodeId>,
    /// The feature a picture fill reports under; `None` reads no pictures.
    pub picture: Option<FeatureId>,
    /// What `grpFill` stands for: the enclosing group's fill.
    pub group_fill: Option<Fill>,
}

impl<'l, 'a> PaintCx<'l, 'a> {
    /// Paint on an element, or on the container when `node` is `None`, reading no pictures.
    pub fn new(ctx: &'l mut Lowering<'a>, node: Option<NodeId>) -> Self {
        PaintCx {
            ctx,
            node,
            picture: None,
            group_fill: None,
        }
    }

    /// The same paint, reading pictures under `feature`.
    pub fn with_pictures(mut self, feature: FeatureId) -> Self {
        self.picture = Some(feature);
        self
    }

    pub fn with_group_fill(mut self, fill: Option<Fill>) -> Self {
        self.group_fill = fill;
        self
    }

    fn picture(&mut self, blip_fill: &Element) -> Result<Option<Sha256>, LowerError> {
        match self.picture {
            Some(feature) => blip_media(blip_fill, self.ctx, self.node.as_ref(), feature),
            None => Ok(None),
        }
    }
}

impl Report for PaintCx<'_, '_> {
    fn report(&mut self, code: DiagnosticCode, site: DiagnosticSite) {
        let site = match &self.node {
            Some(node) => site.node(node.clone()),
            None => site,
        };
        self.ctx.reporter.report(code, site);
    }

    fn dropped(&mut self, modifier: &str) {
        Report::dropped(&mut self.ctx.reporter, modifier);
    }
}

/// The fill choice among an element's children (`p:spPr`, `p:bgPr`, a style list entry).
pub(crate) fn fill_element<'a>(parent: impl El<'a>) -> Option<&'a Element> {
    parent.element().and_then(fill_element_rc).map(|rc| &**rc)
}

/// [`fill_element`], as the shared handle a style keeps.
pub(crate) fn fill_element_rc(parent: &Element) -> Option<&Rc<Element>> {
    parent
        .kids()
        .find(|kid| kid.ns == Ns::A && FILL_ELEMENTS.contains(&&*kid.local))
}

/// The colour of a fill element's colour child, with `phClr` standing for the style reference's colour.
pub(crate) fn fill_color<'a>(
    parent: impl El<'a>,
    report: &mut dyn Report,
    ph_color: Option<&Color>,
) -> Option<Color> {
    color_element(parent).map(|element| fill_placeholder(read_color(element, report), ph_color))
}

fn unit(value: f64) -> slidock_ir::units::UnitRatio {
    unit_ratio(value.clamp(0.0, 1.0))
}

fn stops(
    grad_fill: &Element,
    report: &mut dyn Report,
    ph_color: Option<&Color>,
) -> Vec<GradientStop<Color>> {
    let mut out: Vec<GradientStop<Color>> = grad_fill
        .child(Ns::A, "gsLst")
        .children(Ns::A, "gs")
        .into_iter()
        .filter_map(|gs| {
            let color = fill_color(gs, report, ph_color)?;
            let pos = gs.int_attr("pos").unwrap_or(0) as f64 / 100_000.0;
            Some(GradientStop {
                pos: unit(pos),
                color,
            })
        })
        .collect();
    out.sort_by_key(|stop| stop.pos);
    out.truncate(64);
    out
}

/// The insets of an OOXML relative rectangle (`fillToRect`, `tileRect`) as box fractions; absent sides are 0.
fn insets<'a>(rect: impl El<'a>) -> Edges {
    let rect = rect.element();
    let side = |name: &str| ratio(rect.int_attr(name).unwrap_or(0) as f64 / 100_000.0);
    Edges {
        l: side("l"),
        t: side("t"),
        r: side("r"),
        b: side("b"),
    }
}

/// What a gradient draws otherwise, as PX2106 under the feature it fills.
fn approximated(report: &mut dyn Report, feature: FeatureId, reason: impl Into<String>) {
    report.report(
        codes::PX2106,
        DiagnosticSite::new()
            .feature(feature)
            .data("reason", reason.into()),
    );
}

/// `tileRect`: absent when it is the box. A tile smaller than the box on any side is drawn once (reported), and an
/// empty one is the box (reported).
fn tile(grad_fill: &Element, report: &mut dyn Report, feature: FeatureId) -> Option<Edges> {
    let tile = insets(grad_fill.child(Ns::A, "tileRect"));
    let sides = [tile.l, tile.t, tile.r, tile.b];
    if sides.iter().all(|side| *side == Ratio::ZERO) {
        return None;
    }
    if tile.l.to_f64() + tile.r.to_f64() >= 1.0 || tile.t.to_f64() + tile.b.to_f64() >= 1.0 {
        approximated(report, feature, "an empty tile is drawn as the shape");
        return None;
    }
    if sides.iter().any(|side| side.to_f64() > 0.0) {
        approximated(
            report,
            feature,
            "a tile smaller than the shape is drawn once, not repeated",
        );
    }
    Some(tile)
}

fn gradient(grad_fill: &Element, report: &mut dyn Report, ph_color: Option<&Color>) -> Fill {
    let list = stops(grad_fill, report, ph_color);
    if list.is_empty() {
        return Fill::None;
    }
    if let Some(shade) = grad_fill.child(Ns::A, "path") {
        let kind = shade.attr("path").unwrap_or("circle");
        let path = match kind {
            "circle" => PathShape::Circle,
            "rect" => PathShape::Rect,
            "shape" => {
                approximated(
                    report,
                    FeatureId::FillPath,
                    "a path gradient that follows the outline is drawn as a rectangular one",
                );
                PathShape::Rect
            }
            other => {
                approximated(
                    report,
                    FeatureId::FillPath,
                    format!("unknown path type {}", clip(other, 64)),
                );
                return Fill::None;
            }
        };
        let feature = match path {
            PathShape::Circle => FeatureId::FillRadial,
            PathShape::Rect => FeatureId::FillPath,
        };
        return Fill::Path(PathGradient {
            path,
            focus: insets(shade.child(Ns::A, "fillToRect")),
            tile: tile(grad_fill, report, feature).unwrap_or_else(|| insets(None::<&Element>)),
            stops: list,
        });
    }
    let lin = grad_fill.child(Ns::A, "lin");
    Fill::LinearAngle(LinearAngle {
        angle: angle(lin.int_attr("ang").unwrap_or(0)),
        scaled: lin.bool_attr("scaled").unwrap_or(false),
        rotate_with_shape: grad_fill.bool_attr("rotWithShape").unwrap_or(true),
        outset: None,
        tile: tile(grad_fill, report, FeatureId::FillLinear),
        stops: list,
    })
}

/// One fill element as an IR fill; `None` for an unreadable picture.
pub(crate) fn read_fill(
    element: &Element,
    paint: &mut PaintCx<'_, '_>,
    ph_color: Option<&Color>,
) -> Result<Option<Fill>, LowerError> {
    Ok(match &*element.local {
        "noFill" => Some(Fill::None),
        "solidFill" => Some(
            fill_color(element, paint, ph_color).map_or(Fill::None, |color| Fill::Solid { color }),
        ),
        "gradFill" => Some(gradient(element, paint, ph_color)),
        "pattFill" => {
            paint.report(
                codes::PX2103,
                DiagnosticSite::new().feature(FeatureId::FillPattern),
            );
            Some(
                fill_color(element.child(Ns::A, "fgClr"), paint, ph_color)
                    .map_or(Fill::None, |color| Fill::Solid { color }),
            )
        }
        "grpFill" => Some(paint.group_fill.clone().unwrap_or(Fill::None)),
        "blipFill" => {
            if element.child(Ns::A, "tile").is_some() {
                paint.report(
                    codes::PX2104,
                    DiagnosticSite::new().feature(FeatureId::FillTile),
                );
            }
            paint.picture(element)?.map(|media| Fill::Image {
                media,
                mode: Stretch,
            })
        }
        _ => None,
    })
}

/// The features a shape's own fill element carries in the source.
pub(crate) fn fill_features(element: Option<&Element>) -> Vec<FeatureId> {
    let Some(element) = element else {
        return Vec::new();
    };
    match &*element.local {
        "solidFill" => vec![FeatureId::FillSolid],
        "gradFill" => {
            if element
                .child(Ns::A, "gsLst")
                .children(Ns::A, "gs")
                .is_empty()
            {
                Vec::new()
            } else if let Some(path) = element.child(Ns::A, "path") {
                match path.attr("path").unwrap_or("circle") {
                    "circle" => vec![FeatureId::FillRadial],
                    _ => vec![FeatureId::FillPath],
                }
            } else {
                vec![FeatureId::FillLinear]
            }
        }
        "pattFill" => vec![FeatureId::FillPattern],
        "blipFill" => {
            if element.child(Ns::A, "tile").is_some() {
                vec![FeatureId::FillTile]
            } else {
                vec![FeatureId::FillImage]
            }
        }
        _ => Vec::new(),
    }
}
