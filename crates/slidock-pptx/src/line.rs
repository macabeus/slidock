//! Outlines (`a:ln`, §20.1.2.2.24): width, paint, dash, cap, join and line ends. A shape's own `a:ln` overrides, field
//! by field, the line its style reference names in the theme; a gradient line is shown as its first stop.

use std::rc::Rc;

use slidock_ir::Nullable;
use slidock_ir::color::Color;
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::features::FeatureId;
use slidock_ir::paint::{
    Cap, Dash, DashPreset, EndSize, Join, LineEnd, LineEndPreset, LineEndShape, SourceStroke,
    StrokePaint,
};

use crate::fill::{PaintCx, fill_color, fill_element};
use crate::report::Report;
use crate::units::{emu_to_size, ratio};
use crate::xml::{El, Element, Ns};

/// PowerPoint draws a line that omits `w` at three quarters of a point.
const DEFAULT_WIDTH: i64 = 9525;

/// A line reference: the theme's line and the colour its `phClr` stands for.
#[derive(Clone, Debug, Default)]
pub(crate) struct StyleLine {
    pub element: Option<Rc<Element>>,
    pub color: Option<Color>,
}

fn paint(
    ln: Option<&Element>,
    report: &mut dyn Report,
    ph_color: Option<&Color>,
) -> Option<StrokePaint<Color>> {
    let element = fill_element(ln)?;
    let solid = |color: Option<Color>| {
        color.map_or(StrokePaint::None, |color| StrokePaint::Solid { color })
    };
    Some(match &*element.local {
        "noFill" => StrokePaint::None,
        "solidFill" => solid(fill_color(element, report, ph_color)),
        "gradFill" => {
            report.report(
                codes::PX2105,
                DiagnosticSite::new().feature(FeatureId::StrokeGradient),
            );
            let first = element.child(Ns::A, "gsLst").child(Ns::A, "gs");
            solid(fill_color(first, report, ph_color))
        }
        "pattFill" => {
            report.report(
                codes::PX2103,
                DiagnosticSite::new().feature(FeatureId::FillPattern),
            );
            solid(fill_color(element.child(Ns::A, "fgClr"), report, ph_color))
        }
        _ => StrokePaint::None,
    })
}

pub(crate) fn dash_preset(name: &str) -> Option<DashPreset> {
    Some(match name {
        "solid" => DashPreset::Solid,
        "dot" => DashPreset::Dot,
        "dash" => DashPreset::Dash,
        "lgDash" => DashPreset::LgDash,
        "dashDot" => DashPreset::DashDot,
        "lgDashDot" => DashPreset::LgDashDot,
        "lgDashDotDot" => DashPreset::LgDashDotDot,
        "sysDot" => DashPreset::SysDot,
        "sysDash" => DashPreset::SysDash,
        "sysDashDot" => DashPreset::SysDashDot,
        "sysDashDotDot" => DashPreset::SysDashDotDot,
        _ => return None,
    })
}

fn dash(ln: Option<&Element>) -> Option<Dash> {
    if let Some(preset) = ln.child(Ns::A, "prstDash").attr("val") {
        return Some(Dash::Preset(
            dash_preset(preset).unwrap_or(DashPreset::Solid),
        ));
    }
    let custom = ln.child(Ns::A, "custDash")?;
    // Dash and space lengths are thousandths of a percent of the line width.
    let multiples: Vec<_> = custom
        .children(Ns::A, "ds")
        .flat_map(|ds| {
            [
                ds.int_attr("d").unwrap_or(0),
                ds.int_attr("sp").unwrap_or(0),
            ]
        })
        .take(32)
        .map(|length| ratio(length.max(0) as f64 / 100_000.0))
        .collect();
    Some(if multiples.is_empty() {
        Dash::Preset(DashPreset::Solid)
    } else {
        Dash::Custom { custom: multiples }
    })
}

fn join(ln: Option<&Element>) -> Option<Join> {
    ln.kids()
        .into_iter()
        .find_map(|element| match &*element.local {
            "round" => Some(Join::Round),
            "bevel" => Some(Join::Bevel),
            "miter" => Some(Join::Miter),
            _ => None,
        })
}

fn cap(name: &str) -> Option<Cap> {
    match name {
        "flat" => Some(Cap::Flat),
        "rnd" => Some(Cap::Round),
        "sq" => Some(Cap::Square),
        _ => None,
    }
}

fn line_end(element: Option<&Element>) -> Option<LineEnd> {
    let shape = match element.attr("type")? {
        "triangle" => LineEndShape::Triangle,
        "stealth" => LineEndShape::Stealth,
        "diamond" => LineEndShape::Diamond,
        "oval" => LineEndShape::Oval,
        "arrow" => LineEndShape::Arrow,
        _ => return None,
    };
    let size = |name: &str| match element.attr(name).unwrap_or("med") {
        "sm" => EndSize::Sm,
        "lg" => EndSize::Lg,
        _ => EndSize::Med,
    };
    Some(LineEnd::Preset(LineEndPreset {
        shape,
        w: size("w"),
        len: size("len"),
    }))
}

/// The stroke of an outline and the line its style names: `None` when both are absent (a placeholder then inherits
/// its parent's), `Some(None)` when the style references no line or the line paints nothing. With `record`, the
/// features of the source's own line are recorded on the container.
pub(crate) fn read_line(
    ln: Option<&Element>,
    style: Option<&StyleLine>,
    paint_cx: &mut PaintCx<'_, '_>,
    record: bool,
) -> Nullable<SourceStroke> {
    let base = style.and_then(|style| style.element.as_deref());
    if ln.is_none() && style.is_none() {
        return None;
    }
    // A style reference with `idx="0"` removes the line.
    if ln.is_none() && base.is_none() {
        return Some(None);
    }
    let fill = match paint(ln, paint_cx, None) {
        Some(fill) => Some(fill),
        None => paint(base, paint_cx, style.and_then(|style| style.color.as_ref())),
    };
    let fill = match fill {
        None | Some(StrokePaint::None) => return Some(None),
        Some(fill) => fill,
    };
    let width = ln
        .int_attr("w")
        .or_else(|| base.int_attr("w"))
        .unwrap_or(DEFAULT_WIDTH);
    let end = |name: &str| line_end(ln.child(Ns::A, name).or_else(|| base.child(Ns::A, name)));
    let stroke = SourceStroke {
        width: emu_to_size(width),
        fill,
        dash: dash(ln)
            .or_else(|| dash(base))
            .unwrap_or(Dash::Preset(DashPreset::Solid)),
        cap: cap(ln.attr("cap").or_else(|| base.attr("cap")).unwrap_or("")).unwrap_or(Cap::Flat),
        join: join(ln).or_else(|| join(base)).unwrap_or(Join::Round),
        head: end("headEnd"),
        tail: end("tailEnd"),
    };
    if record && fill_element(ln).is_some() {
        paint_cx.ctx.seen(FeatureId::Stroke);
        if !stroke.dash.is_solid() && dash(ln).is_some() {
            paint_cx.ctx.seen(FeatureId::StrokeDash);
        }
        if line_end(ln.child(Ns::A, "headEnd")).is_some()
            || line_end(ln.child(Ns::A, "tailEnd")).is_some()
        {
            paint_cx.ctx.seen(FeatureId::StrokeLineEnd);
        }
    }
    Some(Some(stroke))
}
