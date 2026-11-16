//! Paint as SVG and CSS. Resolved gradients are frame-local, functions of the element's box fractions; they print in
//! user space scaled by the box ([`crate::gradient`]), which is exactly those units over the frame (an
//! `objectBoundingBox` gradient would use each path's own bounds instead).

use slidock_format::css_string;
use slidock_ir::color::ResolvedColor;
use slidock_ir::paint::{Cap, Dash, DashPreset, Join, ResolvedFill, Stroke, StrokePaint};
use slidock_ir::table::Border;

use crate::color::css_color;
use crate::context::PrintCtx;
use crate::css::{Decl, fixed};
use crate::error::Result;
use crate::gradient::{PaintBox, svg_gradient};
use crate::tree::{Attrs, Element, lines};

/// An SVG paint value and the definitions it references.
pub struct SvgPaint {
    pub value: String,
    pub defs: Vec<Element>,
}

impl SvgPaint {
    fn none() -> SvgPaint {
        SvgPaint {
            value: "none".to_owned(),
            defs: Vec::new(),
        }
    }
}

/// A fill as SVG paint for the paint box `area`; `id` names its definitions.
pub fn svg_fill(
    fill: Option<&ResolvedFill>,
    area: &PaintBox,
    id: &str,
    ctx: &mut PrintCtx<'_>,
) -> Result<SvgPaint> {
    let Some(fill) = fill else {
        return Ok(SvgPaint::none());
    };
    if let Some(defs) = svg_gradient(fill, area, id)? {
        return Ok(SvgPaint {
            value: format!("url(#{id})"),
            defs,
        });
    }
    match fill {
        ResolvedFill::Solid { color } => Ok(SvgPaint {
            value: css_color(color)?,
            defs: Vec::new(),
        }),
        ResolvedFill::Image { media, .. } => {
            let Some(href) = ctx.media_ref(media)? else {
                return Ok(SvgPaint::none());
            };
            let (fw, fh) = (fixed(area.w)?, fixed(area.h)?);
            let image = Element::new(
                "image",
                Attrs::new()
                    .val("href", href)
                    .val("width", fw.clone())
                    .val("height", fh.clone())
                    .val("preserveAspectRatio", "none".to_owned()),
                Vec::new(),
            );
            let pattern = Element::new(
                "pattern",
                Attrs::new()
                    .val("id", id.to_owned())
                    .val("patternUnits", "userSpaceOnUse".to_owned())
                    .val("width", fw)
                    .val("height", fh),
                lines(vec![image]),
            );
            Ok(SvgPaint {
                value: format!("url(#{id})"),
                defs: vec![pattern],
            })
        }
        _ => Ok(SvgPaint::none()),
    }
}

/// ECMA-376's preset dashes, as dash and gap lengths in multiples of the stroke width.
fn preset_pattern(preset: DashPreset) -> &'static [f64] {
    match preset {
        DashPreset::Solid => &[],
        DashPreset::Dot => &[1.0, 3.0],
        DashPreset::Dash => &[4.0, 3.0],
        DashPreset::LgDash => &[8.0, 3.0],
        DashPreset::DashDot => &[4.0, 3.0, 1.0, 3.0],
        DashPreset::LgDashDot => &[8.0, 3.0, 1.0, 3.0],
        DashPreset::LgDashDotDot => &[8.0, 3.0, 1.0, 3.0, 1.0, 3.0],
        DashPreset::SysDot => &[1.0, 1.0],
        DashPreset::SysDash => &[3.0, 1.0],
        DashPreset::SysDashDot => &[3.0, 1.0, 1.0, 1.0],
        DashPreset::SysDashDotDot => &[3.0, 1.0, 1.0, 1.0, 1.0, 1.0],
    }
}

/// A dash as dash and gap lengths in multiples of the stroke width; empty for a solid line.
pub fn dash_pattern(dash: &Dash) -> Vec<f64> {
    match dash {
        Dash::Preset(preset) => preset_pattern(*preset).to_vec(),
        Dash::Custom { custom } => custom.iter().map(|length| length.to_f64()).collect(),
    }
}

/// A stroke or a table cell edge: what both draw with.
pub trait Line {
    fn width(&self) -> f64;
    fn paint(&self) -> &StrokePaint<ResolvedColor>;
    fn dash(&self) -> &Dash;
    fn cap(&self) -> Cap;
    fn join(&self) -> Join;
}

impl Line for Stroke<ResolvedColor> {
    fn width(&self) -> f64 {
        self.width.to_f64()
    }
    fn paint(&self) -> &StrokePaint<ResolvedColor> {
        &self.fill
    }
    fn dash(&self) -> &Dash {
        &self.dash
    }
    fn cap(&self) -> Cap {
        self.cap
    }
    fn join(&self) -> Join {
        self.join
    }
}

impl Line for Border<ResolvedColor> {
    fn width(&self) -> f64 {
        self.width.to_f64()
    }
    fn paint(&self) -> &StrokePaint<ResolvedColor> {
        &self.fill
    }
    fn dash(&self) -> &Dash {
        &self.dash
    }
    fn cap(&self) -> Cap {
        self.cap
    }
    fn join(&self) -> Join {
        self.join
    }
}

/// Whether a line paints anything: a solid line with a width and a colour that is not fully transparent.
pub fn strokes<L: Line>(line: Option<&L>) -> Option<&L> {
    line.filter(|line| match line.paint() {
        StrokePaint::Solid { color } => line.width() > 0.0 && color.rgb().a.to_f64() > 0.0,
        StrokePaint::None => false,
    })
}

/// The stroke attributes of a stroked path, its lengths in user units or in what `length` writes.
pub fn stroke_attributes<L: Line>(line: &L, length: fn(f64) -> Result<String>) -> Result<Attrs> {
    let pattern = dash_pattern(line.dash());
    let dasharray = if pattern.is_empty() {
        None
    } else {
        let parts = pattern
            .iter()
            .map(|n| length(n * line.width()))
            .collect::<Result<Vec<_>>>()?;
        Some(parts.join(" "))
    };
    let cap = match line.cap() {
        Cap::Flat => None,
        Cap::Round => Some("round"),
        Cap::Square => Some("square"),
    };
    let join = match line.join() {
        Join::Miter => None,
        Join::Round => Some("round"),
        Join::Bevel => Some("bevel"),
    };
    Ok(Attrs::new()
        .val("stroke-width", length(line.width())?)
        .val("stroke-dasharray", dasharray)
        .val("stroke-linecap", cap.map(str::to_owned))
        .val("stroke-linejoin", join.map(str::to_owned)))
}

/// The colour a solid line paints, which also fills its line ends.
pub fn stroke_color<L: Line>(line: &L) -> Result<String> {
    match line.paint() {
        StrokePaint::Solid { color } => css_color(color),
        StrokePaint::None => Ok("none".to_owned()),
    }
}

/// A container background as CSS, for solid and image fills; gradients print as SVG ([`svg_fill`]).
pub fn background_decls(fill: &ResolvedFill, ctx: &mut PrintCtx<'_>) -> Result<Vec<Decl>> {
    match fill {
        ResolvedFill::Solid { color } => Ok(vec![("background-color", css_color(color)?)]),
        ResolvedFill::Image { media, .. } => Ok(match ctx.media_ref(media)? {
            None => Vec::new(),
            Some(href) => vec![
                ("background-image", format!("url({})", css_string(&href))),
                ("background-size", "100% 100%".to_owned()),
            ],
        }),
        _ => Ok(Vec::new()),
    }
}
