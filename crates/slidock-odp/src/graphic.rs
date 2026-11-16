//! Paint of graphic and drawing-page properties. Area fills (`draw:fill`): none, solid with its opacity, gradients as
//! the IR's `odf` source gradient with their transparency gradient folded in where it can be (gradient.rs), bitmaps as
//! image fills, and hatches as one colour. Lines (`draw:stroke`): width, colour, dashes as multiples of the width, cap,
//! join and markers as line ends. Shadows (`draw:shadow`): offset, colour, opacity and blur, as LibreOffice draws them.

use slidock_canon::js_number;
use slidock_core_api::lowering::LowerError;
use slidock_ir::Nullable;
use slidock_ir::color::{Color, RgbColor};
use slidock_ir::diagnostics::{DiagnosticCode, DiagnosticSite, codes};
use slidock_ir::features::FeatureId;
use slidock_ir::ids::{NodeId, Sha256};
use slidock_ir::paint::{
    Cap, Dash, DashPreset, EndSize, Fill, Join, LineEnd, LineEndPreset, LineEndShape, SourceStroke,
    Stretch, StrokePaint,
};
use slidock_ir::shadow::{MAX_SIGMA, SourceShadow};
use slidock_math::{Grid, quantise};

use crate::context::Lowering;
use crate::document::CONTENT;
use crate::gradient;
use crate::names::{QName, qn};
use crate::odf::{Element, decode_style_name};
use crate::picture::picture_media;
use crate::styles::{CommonStyles, PropertySet, get};
use crate::units::{
    hex_color, is_space, length_pt, percent, pt, ratio_of, raw_length, size, size_pt, to_pt,
    with_alpha,
};

/// How paint readers report and read pictures while lowering a container, on one of its elements or on itself.
#[derive(Clone)]
pub(crate) struct Paint<'a, 'd> {
    pub ctx: Lowering<'a, 'd>,
    pub node: Option<NodeId>,
}

impl<'a, 'd> Paint<'a, 'd> {
    pub(crate) fn of(ctx: Lowering<'a, 'd>, node: Option<&NodeId>) -> Self {
        Paint {
            ctx,
            node: node.cloned(),
        }
    }

    pub(crate) fn common(&self) -> &'d CommonStyles<'a> {
        self.ctx.styles.common
    }

    pub(crate) fn report(&self, code: DiagnosticCode, site: DiagnosticSite) {
        let site = match &self.node {
            Some(node) => DiagnosticSite {
                node: Some(node.clone()),
                ..site
            },
            None => site,
        };
        self.ctx.report(code, site);
    }

    /// A package file as media, hashed and recorded; `None` after the reader declines it.
    pub(crate) fn picture(
        &self,
        href: &str,
        feature: FeatureId,
    ) -> Result<Option<Sha256>, LowerError> {
        picture_media(href, &self.ctx, feature, self.node.as_ref(), false)
    }
}

/// LibreOffice's default area colour, for a fill that omits its colour.
const DEFAULT_FILL: &str = "#729fcf";

/// `draw:opacity` (`100%` opaque) as a factor.
pub(crate) fn opacity_of(value: Option<&str>) -> f64 {
    percent(value).map_or(1.0, |pct| pct.clamp(0.0, 1.0))
}

fn black() -> RgbColor {
    crate::units::rgb(0.0, 0.0, 0.0, 1.0)
}

/// A fill as written, with the features it holds in the source.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ReadFill {
    pub fill: Option<Fill>,
    pub features: Vec<FeatureId>,
}

impl ReadFill {
    fn of(fill: Fill, features: Vec<FeatureId>) -> Self {
        ReadFill {
            fill: Some(fill),
            features,
        }
    }
}

/// The fill of a property set (`style:graphic-properties`, `style:drawing-page-properties`); no fill when
/// `draw:fill` is absent, so a slide keeps its master page's background.
pub(crate) fn read_fill(
    props: &PropertySet,
    paint: &Paint<'_, '_>,
) -> Result<ReadFill, LowerError> {
    let kind = get(props, qn!("draw:fill"));
    let opacity = opacity_of(get(props, qn!("draw:opacity")));
    let common = paint.common();
    let tenths = common.unitless_angle_tenths;
    // A uniform transparence other than 0 wins over the transparency gradient (`createPolyPolygonFillPrimitive`).
    let transparency = get(props, qn!("draw:opacity-name"))
        .filter(|name| !name.is_empty() && transparence(opacity) == 0.0)
        .and_then(|name| common.opacities.get(name))
        .map(|element| gradient::read_definition(element, tenths, true));
    let solid = |color: RgbColor| Fill::Solid {
        color: Color::Rgb(with_alpha(&color, opacity)),
    };
    let folded = |folded: gradient::Folded| {
        gradient::report(folded.notes, paint);
        ReadFill {
            fill: Some(folded.fill),
            features: folded.features,
        }
    };
    let over_solid = |color: RgbColor, features: Vec<FeatureId>| match &transparency {
        None => ReadFill::of(solid(color), features),
        Some(transparency) => {
            let mut read = folded(gradient::solid_fill(
                &with_alpha(&color, opacity),
                transparency.clone(),
            ));
            for feature in features {
                if !read.features.contains(&feature) {
                    read.features.push(feature);
                }
            }
            read
        }
    };
    let default_fill = || hex_color(Some(DEFAULT_FILL)).unwrap_or_else(black);
    Ok(match kind {
        None => ReadFill::default(),
        Some("none") => ReadFill::of(Fill::None, Vec::new()),
        Some("solid") => {
            let color = hex_color(Some(
                get(props, qn!("draw:fill-color")).unwrap_or(DEFAULT_FILL),
            ))
            .unwrap_or_else(default_fill);
            match &transparency {
                None => ReadFill::of(solid(color), vec![FeatureId::FillSolid]),
                Some(_) => over_solid(color, Vec::new()),
            }
        }
        Some("gradient") => {
            let name = get(props, qn!("draw:fill-gradient-name")).unwrap_or("");
            match common.gradients.get(name) {
                None => ReadFill::of(Fill::None, Vec::new()),
                Some(element) => {
                    let steps = get(props, qn!("draw:gradient-step-count"))
                        .and_then(|value| crate::units::plain_number(Some(value)))
                        .filter(|n| n.is_finite() && *n >= 1.0)
                        .map_or(0, |n| n.min(f64::from(u16::MAX)) as u32);
                    folded(gradient::gradient_fill(
                        gradient::read_definition(element, tenths, false),
                        steps,
                        opacity,
                        transparency,
                    ))
                }
            }
        }
        Some("bitmap") => {
            let image = common
                .fill_images
                .get(get(props, qn!("draw:fill-image-name")).unwrap_or(""));
            let href = image
                .and_then(|image| image.attr(qn!("xlink:href")))
                .filter(|href| !href.is_empty());
            let repeat = get(props, qn!("style:repeat")).unwrap_or("repeat");
            let feature = if repeat == "repeat" {
                FeatureId::FillTile
            } else {
                FeatureId::FillImage
            };
            if repeat != "stretch" {
                let mut site = DiagnosticSite::new();
                if repeat == "repeat" {
                    site = site.feature(feature);
                }
                paint.report(codes::OD9107, site.data("repeat", repeat));
            }
            let media = match href {
                Some(href) => paint.picture(href, feature)?,
                None => None,
            };
            let image = |media| Fill::Image {
                media,
                mode: Stretch,
            };
            match (media, &transparency) {
                (Some(media), None) => ReadFill::of(image(media), vec![feature]),
                (Some(media), Some((transparency, _))) => {
                    let mut read = folded(gradient::unfolded(
                        image(media),
                        transparency,
                        "a transparency gradient over a picture fill",
                    ));
                    read.features.retain(|f| *f != FeatureId::FillImage);
                    read.features.insert(0, feature);
                    read
                }
                (None, _) => ReadFill::default(),
            }
        }
        Some("hatch") => {
            paint.report(
                codes::OD9102,
                DiagnosticSite::new().feature(FeatureId::FillPattern),
            );
            let hatch = common
                .hatches
                .get(get(props, qn!("draw:fill-hatch-name")).unwrap_or(""));
            // A hatch over its area colour shows mostly that colour; a bare hatch, its lines'.
            let is_solid = get(props, qn!("draw:fill-hatch-solid")) == Some("true");
            let color = hex_color(if is_solid {
                Some(get(props, qn!("draw:fill-color")).unwrap_or(DEFAULT_FILL))
            } else {
                hatch.and_then(|hatch| hatch.attr(qn!("draw:color")))
            });
            match color {
                Some(color) => over_solid(color, vec![FeatureId::FillPattern]),
                None => ReadFill::of(Fill::None, vec![FeatureId::FillPattern]),
            }
        }
        Some(_) => ReadFill::of(Fill::None, Vec::new()),
    })
}

/// LibreOffice draws a zero-width line as a hairline: one pixel, here three quarters of a point.
const HAIRLINE: f64 = 0.75;

fn cap_of(value: Option<&str>) -> Option<Cap> {
    match value? {
        "butt" => Some(Cap::Flat),
        "round" => Some(Cap::Round),
        "square" => Some(Cap::Square),
        _ => None,
    }
}

fn join_of(value: Option<&str>) -> Option<Join> {
    match value? {
        "miter" | "middle" => Some(Join::Miter),
        "round" => Some(Join::Round),
        "bevel" => Some(Join::Bevel),
        _ => None,
    }
}

/// A dash or gap length as a multiple of the line width: absolute, or a percentage of the width.
fn dash_length(value: Option<&str>, width: f64, fallback: f64) -> f64 {
    if let Some(pct) = percent(value) {
        return pct;
    }
    match raw_length(value) {
        Some(pt) if width > 0.0 => pt / width,
        _ => fallback,
    }
}

/// A `draw:stroke-dash`: `dots1` dashes of one length, `dots2` of another, each followed by `distance`.
fn dash_of(element: Option<&Element>, width: f64) -> Dash {
    let Some(element) = element else {
        return Dash::Preset(DashPreset::Solid);
    };
    let count = |name: QName| element.int_attr(name).unwrap_or(0).clamp(0, 16);
    // A dash of unstated length is a dot as long as the line is wide.
    let first = dash_length(element.attr(qn!("draw:dots1-length")), width, 1.0);
    let second = dash_length(element.attr(qn!("draw:dots2-length")), width, 1.0);
    let gap = dash_length(element.attr(qn!("draw:distance")), width, 1.0);
    let mut lengths: Vec<f64> = Vec::new();
    for _ in 0..count(qn!("draw:dots1")) {
        lengths.extend([first, gap]);
    }
    for _ in 0..count(qn!("draw:dots2")) {
        lengths.extend([second, gap]);
    }
    let multiples: Vec<_> = lengths
        .into_iter()
        .take(32)
        .map(|length| {
            ratio_of(quantise(
                length.clamp(0.0, 1000.0),
                Grid::HundredThousandths,
            ))
        })
        .collect();
    if multiples.is_empty() {
        Dash::Preset(DashPreset::Solid)
    } else {
        Dash::Custom { custom: multiples }
    }
}

const SIZES: [EndSize; 3] = [EndSize::Sm, EndSize::Med, EndSize::Lg];

/// A length relative to the line width as DrawingML's size classes (2, 3 and 5 widths).
fn size_class(multiple: f64) -> EndSize {
    if multiple <= 2.5 {
        EndSize::Sm
    } else if multiple <= 4.0 {
        EndSize::Med
    } else {
        EndSize::Lg
    }
}

fn ms_marker(name: &str) -> Option<LineEndShape> {
    match name {
        "msArrowEnd" => Some(LineEndShape::Triangle),
        "msArrowOpenEnd" => Some(LineEndShape::Arrow),
        "msArrowStealthEnd" => Some(LineEndShape::Stealth),
        "msArrowDiamondEnd" => Some(LineEndShape::Diamond),
        "msArrowOvalEnd" => Some(LineEndShape::Oval),
        _ => None,
    }
}

/// `^(msArrow\w*End)\s+([1-9])$`: the marker's base name and its size index.
fn ms_name(display: &str) -> Option<(&str, u32)> {
    let mut chars = display.chars();
    let digit = chars.next_back()?.to_digit(10).filter(|&d| d >= 1)?;
    let rest = chars.as_str();
    let base = rest.trim_end_matches(is_space);
    if base.len() == rest.len() {
        return None;
    }
    let word = base.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    (word && base.starts_with("msArrow") && base.ends_with("End") && base.len() >= 10)
        .then_some((base, digit))
}

/// A line end by its marker's name. OOXML's ends keep their kind and size in the name LibreOffice gives them
/// (`msArrowEnd 5`: a triangle, the 5th of the width × length classes); other markers map by name, sized by the
/// marker's width against the line's and its drawing's aspect.
fn line_end(
    name: Option<&str>,
    marker_width: Option<&str>,
    line_width: f64,
    paint: &Paint<'_, '_>,
) -> Option<LineEnd> {
    let name = name.filter(|s| !s.is_empty())?;
    let display = decode_style_name(name);
    if let Some((base, digit)) = ms_name(&display)
        && let Some(shape) = ms_marker(base)
    {
        let index = (digit - 1) as usize;
        return Some(LineEnd::Preset(LineEndPreset {
            shape,
            w: SIZES.get(index / 3).copied().unwrap_or(EndSize::Lg),
            len: SIZES.get(index % 3).copied().unwrap_or(EndSize::Lg),
        }));
    }
    let lower = display.to_ascii_lowercase();
    let shape = if lower.contains("circle") || lower.contains("oval") {
        LineEndShape::Oval
    } else if lower.contains("diamond") || lower.contains("square 45") {
        LineEndShape::Diamond
    } else if lower.contains("line arrow") || lower.contains("open") {
        LineEndShape::Arrow
    } else {
        LineEndShape::Triangle
    };
    if shape == LineEndShape::Triangle && !lower.contains("arrow") {
        paint.report(
            codes::OD9108,
            DiagnosticSite::new().data(
                "marker",
                slidock_canon::utf16::clip(&display, 64).to_owned(),
            ),
        );
    }
    let marker = paint.common().markers.get(name);
    let view_box = marker
        .and_then(|marker| marker.attr(qn!("svg:viewBox")))
        .unwrap_or("");
    let numbers: Vec<f64> = split_list(slidock_canon::js_trim(view_box))
        .into_iter()
        .map(js_number)
        .collect();
    let aspect = match numbers.as_slice() {
        [_, _, w, h] if *w > 0.0 && *h > 0.0 => h / w,
        _ => 1.0,
    };
    let width = raw_length(marker_width).unwrap_or(0.3 * (72.0 / 2.54));
    let of = |length: f64| {
        if line_width > 0.0 {
            length / line_width
        } else {
            5.0
        }
    };
    Some(LineEnd::Preset(LineEndPreset {
        shape,
        w: size_class(of(width)),
        len: size_class(of(width * aspect)),
    }))
}

/// `text.split(/[\s,]+/)`.
pub(crate) fn split_list(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_separator = false;
    for (at, c) in text.char_indices() {
        let separator = c == ',' || is_space(c);
        if separator && !in_separator {
            out.push(text.get(start..at).unwrap_or_default());
        }
        if !separator && in_separator {
            start = at;
        }
        in_separator = separator;
    }
    out.push(if in_separator {
        ""
    } else {
        text.get(start..).unwrap_or_default()
    });
    out
}

/// A stroke as written, with the features it holds in the source: `None` when the style says nothing of its line,
/// `Some(None)` for no line.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ReadStroke {
    pub stroke: Option<Option<SourceStroke>>,
    pub features: Vec<FeatureId>,
}

/// The line of a graphic property set: none, or its width, paint, dash, cap, join and line ends.
pub(crate) fn read_stroke(props: &PropertySet, paint: &Paint<'_, '_>) -> ReadStroke {
    let Some(kind) = get(props, qn!("draw:stroke")) else {
        return ReadStroke::default();
    };
    if kind == "none" {
        return ReadStroke {
            stroke: Some(None),
            features: Vec::new(),
        };
    }
    let color = hex_color(Some(
        get(props, qn!("svg:stroke-color")).unwrap_or("#000000"),
    ))
    .unwrap_or_else(black);
    let opacity = opacity_of(get(props, qn!("svg:stroke-opacity")));
    // Dashes and markers measure against the width as written, before it rounds to the IR's grid.
    let written = raw_length(get(props, qn!("svg:stroke-width")))
        .unwrap_or(0.0)
        .max(0.0);
    let exact = if written > 0.0 { written } else { HAIRLINE };
    let width = size(to_pt(exact));
    let dashes = &paint.common().dashes;
    let dash_element = dashes
        .get(get(props, qn!("draw:stroke-dash")).unwrap_or(""))
        .copied();
    let dash = if kind == "dash" {
        dash_of(dash_element, exact)
    } else {
        Dash::Preset(DashPreset::Solid)
    };
    let dash_style = dash_element.and_then(|element| element.attr(qn!("draw:style")));
    let cap = cap_of(get(props, qn!("svg:stroke-linecap"))).unwrap_or(
        if kind == "dash" && dash_style == Some("round") {
            Cap::Round
        } else {
            Cap::Flat
        },
    );
    let head = line_end(
        get(props, qn!("draw:marker-start")),
        get(props, qn!("draw:marker-start-width")),
        exact,
        paint,
    );
    let tail = line_end(
        get(props, qn!("draw:marker-end")),
        get(props, qn!("draw:marker-end-width")),
        exact,
        paint,
    );
    let mut features = vec![FeatureId::Stroke];
    if !dash.is_solid() {
        features.push(FeatureId::StrokeDash);
    }
    if head.is_some() || tail.is_some() {
        features.push(FeatureId::StrokeLineEnd);
    }
    let stroke = SourceStroke {
        width,
        fill: StrokePaint::Solid {
            color: Color::Rgb(with_alpha(&color, opacity)),
        },
        dash,
        cap,
        join: join_of(get(props, qn!("draw:stroke-linejoin"))).unwrap_or(Join::Round),
        head,
        tail,
    };
    ReadStroke {
        stroke: Some(Some(stroke)),
        features,
    }
}

/// LibreOffice blurs a shadow to nothing at `loext:shadow-blur`; the Gaussian that best fits its falloff has a
/// standard deviation of 0.43 blurs (measured 0.427–0.431 from 0.25 cm, 0.447 at 0.1 cm).
const SIGMA_PER_BLUR: f64 = 0.43;

/// An opacity as the whole-percent transparence LibreOffice keeps of it.
fn transparence(opacity: f64) -> f64 {
    ((1.0 - opacity) * 100.0).round()
}

/// The shadow of a graphic property set as LibreOffice draws it: `None` when no style states `draw:shadow`,
/// `Some(None)` when hidden or painting nothing. The offset is page-fixed; the colour takes `draw:shadow-opacity` as
/// its alpha, except that a shadow as transparent as the fill (`draw:opacity`) is drawn opaque, the fill's own alpha
/// then showing through it (`createNewSdrShadowAttribute`). Text overflowing an opaque box casts too (measured).
pub(crate) fn read_shadow(props: &PropertySet) -> Nullable<SourceShadow> {
    if get(props, qn!("draw:shadow"))? != "visible" {
        return Some(None);
    }
    let opacity = opacity_of(get(props, qn!("draw:shadow-opacity")));
    let shadow_transparence = transparence(opacity);
    let fill_transparence = transparence(opacity_of(get(props, qn!("draw:opacity"))));
    let opacity = if shadow_transparence != 0.0 && shadow_transparence == fill_transparence {
        1.0
    } else {
        opacity
    };
    let color = hex_color(get(props, qn!("draw:shadow-color"))).unwrap_or_else(black);
    let color = with_alpha(&color, opacity);
    if color.a.steps() == 0 {
        return Some(None);
    }
    let offset = |name: QName| pt(length_pt(get(props, name)).unwrap_or(0.0));
    let blur = size_pt(get(props, qn!("loext:shadow-blur"))).unwrap_or(0.0);
    Some(Some(SourceShadow {
        dx: offset(qn!("draw:shadow-offset-x")),
        dy: offset(qn!("draw:shadow-offset-y")),
        sigma: size(quantise(
            (SIGMA_PER_BLUR * blur).min(MAX_SIGMA),
            Grid::Thousandths,
        )),
        color: Color::Rgb(color),
        rotate_with_shape: false,
        overflow_casts: true,
    }))
}

/// A drawable's shadow field: a shadow it draws, else nothing, except on a slide's placeholder, the one element the
/// IR's inheritance fills from another (its layout's placeholder): there a hidden shadow its own style chain states is
/// stated as `null`, so it is not inherited.
pub(crate) fn shadow_field(
    read: Nullable<SourceShadow>,
    placeholder: bool,
    ctx: &Lowering<'_, '_>,
) -> Nullable<SourceShadow> {
    match read {
        Some(Some(shadow)) => {
            ctx.see(FeatureId::EffectShadow);
            Some(Some(shadow))
        }
        Some(None) if placeholder && ctx.part == CONTENT => Some(None),
        _ => None,
    }
}

/// A visible shadow on a drawable that cannot carry one (OD9104 `{what} shadow`).
pub(crate) fn report_undrawn_shadow(props: &PropertySet, what: &str, paint: &Paint<'_, '_>) {
    if matches!(read_shadow(props), Some(Some(_))) {
        paint.ctx.see(FeatureId::EffectShadow);
        paint.report(
            codes::OD9104,
            DiagnosticSite::new()
                .feature(FeatureId::EffectShadow)
                .data("reason", format!("{what} shadow")),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_lists_as_javascript_does() {
        assert_eq!(split_list(""), [""]);
        assert_eq!(split_list("0 0 300 300"), ["0", "0", "300", "300"]);
        assert_eq!(split_list("0,0, 1  2"), ["0", "0", "1", "2"]);
        assert_eq!(split_list(",1"), ["", "1"]);
        assert_eq!(split_list("1,"), ["1", ""]);
    }

    #[test]
    fn reads_microsoft_marker_names() {
        assert_eq!(ms_name("msArrowEnd 5"), Some(("msArrowEnd", 5)));
        assert_eq!(
            ms_name("msArrowOpenEnd\u{a0} 9"),
            Some(("msArrowOpenEnd", 9))
        );
        assert_eq!(ms_name("msArrowEnd5"), None);
        assert_eq!(ms_name("msArrowEnd 0"), None);
        assert_eq!(ms_name("msArrow End 1"), None);
    }
}
