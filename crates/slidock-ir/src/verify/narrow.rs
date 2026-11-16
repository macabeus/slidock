//! Narrowing guards: a pass reads a value an earlier pass should have provided, and gets it in its complete form or an
//! [`InvariantError`], which the pass manager reports as IR0909 for that container, naming the pass, the invariant and
//! the rule.

use indexmap::IndexMap;

use crate::color::{Color, RgbColor};
use crate::diagnostics::{DiagnosticCode, codes};
use crate::fonts::{FontBinding, FontEntry, font_key};
use crate::frame::Frame;
use crate::geometry::{Geometry, PathsGeometry};
use crate::ids::NodeId;
use crate::media::{MediaEntry, MediaPlan};
use crate::paint::Fill;
use crate::source::Element;
use crate::text::{
    Align, AutoNumbering, Autofit, BodyProps, Bullet, Caps, Decoration, FamilyFont, FontRef,
    Insets, LineHeight, LineSpacing, Link, PartialBodyProps, PartialParaProps, PartialRunProps,
    ResolvedAutofit, Spacing, TextBody, TextRotation,
};
use crate::units::{Pt, Ratio, Size, UnitRatio};
use crate::verify::invariants::Invariant;

/// A value an earlier pass should have provided is missing.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("{invariant} ({rule}) does not hold at {}: {detail}", node.as_ref().map_or("the presentation", NodeId::as_str))]
pub struct InvariantError {
    pub invariant: Invariant,
    pub rule: DiagnosticCode,
    pub node: Option<NodeId>,
    pub detail: String,
}

fn violation(
    invariant: Invariant,
    rule: DiagnosticCode,
    node: Option<&NodeId>,
    detail: impl Into<String>,
) -> InvariantError {
    InvariantError {
        invariant,
        rule,
        node: node.cloned(),
        detail: detail.into(),
    }
}

fn missing(keys: &[&str]) -> String {
    format!("missing {}", keys.join(", "))
}

// inheritance.resolved

/// The element's frame (IR0108).
pub fn framed(element: &Element) -> Result<&Frame, InvariantError> {
    element.frame().as_ref().ok_or_else(|| {
        violation(
            Invariant::InheritanceResolved,
            codes::IR0108,
            Some(element.id()),
            "no frame",
        )
    })
}

/// The body's complete properties (IR0115).
pub fn complete_body(
    body: &PartialBodyProps,
    node: &NodeId,
) -> Result<BodyProps<Autofit>, InvariantError> {
    let fail = || {
        let keys = body.missing_keys();
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        violation(
            Invariant::InheritanceResolved,
            codes::IR0115,
            Some(node),
            missing(&keys),
        )
    };
    let insets = body.insets.as_ref().ok_or_else(fail)?;
    Ok(BodyProps {
        insets: Insets {
            l: insets.l.ok_or_else(fail)?,
            t: insets.t.ok_or_else(fail)?,
            r: insets.r.ok_or_else(fail)?,
            b: insets.b.ok_or_else(fail)?,
        },
        anchor: body.anchor.ok_or_else(fail)?,
        wrap: body.wrap.ok_or_else(fail)?,
        autofit: body.autofit.clone().ok_or_else(fail)?,
        overflow: body.overflow.ok_or_else(fail)?,
        max_width: body.max_width,
    })
}

// theme.resolved

/// A colour's literal value (IR0101).
pub fn resolved_color<'a>(
    color: &'a Color,
    node: Option<&NodeId>,
) -> Result<&'a RgbColor, InvariantError> {
    match color {
        Color::Rgb(rgb) => Ok(rgb),
        Color::Theme(theme) => theme.resolved.as_ref().ok_or_else(|| {
            let slot = serde_json::to_value(theme.slot)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned));
            violation(
                Invariant::ThemeResolved,
                codes::IR0101,
                node,
                format!("theme colour {}", slot.unwrap_or_default()),
            )
        }),
    }
}

/// A font's family (IR0114).
pub fn family_font<'a>(
    font: &'a FontRef,
    node: Option<&NodeId>,
) -> Result<&'a FamilyFont, InvariantError> {
    match font {
        FontRef::Family(family) => Ok(family),
        FontRef::Theme(theme) => Err(violation(
            Invariant::ThemeResolved,
            codes::IR0114,
            node,
            format!("theme font {}", theme.slot.as_str()),
        )),
    }
}

// text.cascaded

/// Cascaded paragraph properties: the ten cascaded keys, and the line metrics if computed.
#[derive(Clone, Debug, PartialEq)]
pub struct CascadedPara {
    pub align: Align,
    pub margin_left: Pt,
    pub margin_right: Pt,
    pub indent: Pt,
    pub space_before: Spacing,
    pub space_after: Spacing,
    pub line_spacing: LineSpacing,
    pub tab_size: Size,
    pub bullet: Bullet,
    pub rtl: bool,
}

/// A paragraph's cascaded properties (IR0102).
pub fn cascaded_para(
    props: &PartialParaProps,
    node: &NodeId,
) -> Result<CascadedPara, InvariantError> {
    let fail = || {
        violation(
            Invariant::TextCascaded,
            codes::IR0102,
            Some(node),
            missing(&props.missing_cascaded_keys()),
        )
    };
    Ok(CascadedPara {
        align: props.align.ok_or_else(fail)?,
        margin_left: props.margin_left.ok_or_else(fail)?,
        margin_right: props.margin_right.ok_or_else(fail)?,
        indent: props.indent.ok_or_else(fail)?,
        space_before: props.space_before.clone().ok_or_else(fail)?,
        space_after: props.space_after.clone().ok_or_else(fail)?,
        line_spacing: props.line_spacing.clone().ok_or_else(fail)?,
        tab_size: props.tab_size.ok_or_else(fail)?,
        bullet: props.bullet.clone().ok_or_else(fail)?,
        rtl: props.rtl.ok_or_else(fail)?,
    })
}

/// Cascaded run properties: the thirteen run keys, fonts and colours possibly still theme references, and the
/// script scale if the run still has one.
#[derive(Clone, Debug, PartialEq)]
pub struct CascadedRun {
    pub font: FontRef,
    pub size: Size,
    pub weight: u32,
    pub italic: bool,
    pub underline: Decoration,
    pub strike: Decoration,
    pub color: Color,
    pub highlight: Option<Color>,
    pub baseline: Ratio,
    pub caps: Caps,
    pub letter_spacing: Pt,
    pub link: Option<Link>,
    pub synthesize_bold: bool,
    pub script_scale: Option<UnitRatio>,
}

/// A run's cascaded properties (IR0103); absent properties lack every key.
pub fn cascaded_run(
    props: Option<&PartialRunProps>,
    node: &NodeId,
) -> Result<CascadedRun, InvariantError> {
    let empty = PartialRunProps::default();
    let props = props.unwrap_or(&empty);
    let fail = || {
        violation(
            Invariant::TextCascaded,
            codes::IR0103,
            Some(node),
            missing(&props.missing_keys()),
        )
    };
    Ok(CascadedRun {
        font: props.font.clone().ok_or_else(fail)?,
        size: props.size.ok_or_else(fail)?,
        weight: props.weight.ok_or_else(fail)?,
        italic: props.italic.ok_or_else(fail)?,
        underline: props.underline.ok_or_else(fail)?,
        strike: props.strike.ok_or_else(fail)?,
        color: props.color.clone().ok_or_else(fail)?,
        highlight: props.highlight.clone().ok_or_else(fail)?,
        baseline: props.baseline.ok_or_else(fail)?,
        caps: props.caps.ok_or_else(fail)?,
        letter_spacing: props.letter_spacing.ok_or_else(fail)?,
        link: props.link.clone().ok_or_else(fail)?,
        synthesize_bold: props.synthesize_bold.ok_or_else(fail)?,
        script_scale: props.script_scale,
    })
}

// lists.labelled

/// A bullet number-lists labelled: none, or a label with its text, font, size and colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LabelledBullet<'a> {
    None,
    Label {
        text: &'a str,
        font: &'a FamilyFont,
        size: Size,
        color: &'a Color,
        auto: Option<&'a AutoNumbering>,
    },
}

/// A labelled bullet (IR0111).
pub fn labelled_bullet<'a>(
    bullet: Option<&'a Bullet>,
    node: &NodeId,
) -> Result<LabelledBullet<'a>, InvariantError> {
    match bullet {
        Some(Bullet::None {}) => Ok(LabelledBullet::None),
        Some(Bullet::Label {
            text,
            font,
            size,
            color,
            auto,
        }) => Ok(LabelledBullet::Label {
            text,
            font,
            size: *size,
            color,
            auto: auto.as_ref(),
        }),
        other => Err(violation(
            Invariant::ListsLabelled,
            codes::IR0111,
            Some(node),
            format!("bullet {}", other.map_or("missing", Bullet::kind)),
        )),
    }
}

// fonts.bound

/// The binding of a family (IR0104).
pub fn binding<'a>(
    fonts: &'a IndexMap<String, FontEntry>,
    family: &str,
    node: &NodeId,
) -> Result<&'a FontBinding, InvariantError> {
    fonts
        .get(&font_key(family))
        .and_then(|entry| entry.binding.as_ref())
        .ok_or_else(|| {
            violation(
                Invariant::FontsBound,
                codes::IR0104,
                Some(node),
                format!("font {family}"),
            )
        })
}

// text.metrics

/// The line metrics text-metrics computes.
#[derive(Clone, Debug, PartialEq)]
pub struct LineMetrics {
    pub line_height: LineHeight,
    pub space_before_pt: Pt,
    pub space_after_pt: Pt,
}

/// A paragraph's line metrics (IR0110).
pub fn line_metrics(
    props: &PartialParaProps,
    node: &NodeId,
) -> Result<LineMetrics, InvariantError> {
    let fail = || {
        violation(
            Invariant::TextMetrics,
            codes::IR0110,
            Some(node),
            missing(&props.missing_line_metric_keys()),
        )
    };
    Ok(LineMetrics {
        line_height: props.line_height.clone().ok_or_else(fail)?,
        space_before_pt: props.space_before_pt.ok_or_else(fail)?,
        space_after_pt: props.space_after_pt.ok_or_else(fail)?,
    })
}

/// A measured autofit: a shrink with its scale, reduction and whether it was fitted (IR0107).
pub fn fitted_autofit(autofit: &Autofit, node: &NodeId) -> Result<ResolvedAutofit, InvariantError> {
    Ok(match autofit {
        Autofit::None {} => ResolvedAutofit::None {},
        Autofit::Resize {
            width,
            height,
            anchor_x,
        } => ResolvedAutofit::Resize {
            width: *width,
            height: *height,
            anchor_x: *anchor_x,
        },
        Autofit::Shrink {
            font_scale,
            line_spacing_reduction,
            fitted,
            ..
        } => match (font_scale, line_spacing_reduction, fitted) {
            (Some(font_scale), Some(line_spacing_reduction), Some(fitted)) => {
                ResolvedAutofit::Shrink {
                    font_scale: *font_scale,
                    line_spacing_reduction: *line_spacing_reduction,
                    fitted: *fitted,
                }
            }
            _ => {
                let keys: Vec<&str> = [
                    ("fontScale", font_scale.is_some()),
                    ("lineSpacingReduction", line_spacing_reduction.is_some()),
                    ("fitted", fitted.is_some()),
                ]
                .into_iter()
                .filter(|(_, present)| !present)
                .map(|(key, _)| key)
                .collect();
                return Err(violation(
                    Invariant::TextMetrics,
                    codes::IR0107,
                    Some(node),
                    missing(&keys),
                ));
            }
        },
    })
}

// geometry.lowered

/// The lowered geometry (IR0105).
pub fn lowered_geometry<'a>(
    geometry: Option<&'a Geometry>,
    node: &NodeId,
) -> Result<&'a PathsGeometry, InvariantError> {
    match geometry {
        Some(Geometry::Paths(paths)) => Ok(paths),
        other => Err(violation(
            Invariant::GeometryLowered,
            codes::IR0105,
            Some(node),
            format!("geometry {}", other.map_or("missing", Geometry::kind)),
        )),
    }
}

/// A fill in its frame-local form; colours may still be theme references (IR0113).
pub fn lowered_fill<'a>(fill: &'a Fill, node: Option<&NodeId>) -> Result<&'a Fill, InvariantError> {
    if fill.is_lowered() {
        Ok(fill)
    } else {
        Err(violation(
            Invariant::GeometryLowered,
            codes::IR0113,
            node,
            format!("source-form {} gradient", fill.kind()),
        ))
    }
}

/// The text's rotation (IR0113).
pub fn text_rotation(text: &TextBody, node: &NodeId) -> Result<TextRotation, InvariantError> {
    text.rotation.ok_or_else(|| {
        violation(
            Invariant::GeometryLowered,
            codes::IR0113,
            Some(node),
            "no text rotation",
        )
    })
}

// media.planned

/// A media entry's plan (IR0106).
pub fn media_plan(entry: &MediaEntry) -> Result<&MediaPlan, InvariantError> {
    entry.plan.as_ref().ok_or_else(|| {
        violation(
            Invariant::MediaPlanned,
            codes::IR0106,
            None,
            format!("media {}", entry.sha256),
        )
    })
}
