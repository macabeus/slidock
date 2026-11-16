//! text-metrics: the line height and paragraph spacing a backend prints, from the cascaded properties, the font
//! bindings and a shrink's scale: the stored one, or the one fitted first for a shrink that stores none
//! ([`crate::shrink_fit`]). A paragraph's lines follow its strut, the largest run (the first on a tie; the paragraph
//! mark when it has none), so the line box matches the text.
//! - A multiple of a metric basis is a unitless ratio: value × (ascender − descender + gap) / unitsPerEm of that
//!   table; `size-1.2` is value × 1.2; `coretext` rounds the hhea ascent, descent and gap to whole points at the
//!   strut's drawn size and scales all but the gap, as CoreText lays out lines. A stored line-spacing reduction
//!   comes off the multiple first.
//! - Exact spacing is points times the stored font scale; at-least, at-most and between clamp against the strut
//!   font's natural height (hhea, what browsers call `normal`) at its scaled size.
//! - Spacing in points stays as written; a percentage is of the strut's basis line height at its scaled size.
//! - Metrics are those of the face each run draws with (a bold file is often taller), and a run whose own multiple
//!   line differs from the paragraph's carries it, so each line is as tall as its tallest run.
//! - Where a basis sets the text off-centre in its line, the paragraph and each run say how much more of their
//!   leading sits above the text than below it (`leadingSkew`): `coretext` puts CoreText's synthesized leading
//!   above; `size-1.2` puts the baseline 0.8 of the way down the single line whatever the face, and a multiple's
//!   extra space below the text, as Google Slides draws PowerPoint and ODF text. `coretext` keeps the single
//!   line's leading above the text whatever the multiple: the space a multiple adds, or takes off, is below it
//!   (Keynote draws a 0.8 multiple's baseline as far down its line as a single line's).
//! - A text's height ends short of its last line box (`lineTailPt`): by the space a multiple adds below the text,
//!   and in a `coretext` table cell by the font's line gap. A `coretext` multiple under 1 reaches past it instead:
//!   the text's height ends at its last single line.

use slidock_canon::js_round;
use slidock_core_api::visit::TextSite;
use slidock_core_api::{ContainerView, OwnedContainer, PassCtx, PassError, PassInfo, SlidePass};
use slidock_ir::fonts::FontMetrics;
use slidock_ir::ids::NodeId;
use slidock_ir::text::ResolvedAutofit;
use slidock_ir::text::{
    Inline, LineBasis, LineHeight, LineSpacing, SourceParagraph, Spacing, TextBody, strut_index,
};
use slidock_ir::units::{Pt, Ratio, Size};
use slidock_ir::verify::{
    CascadedRun, Invariant, binding, cascaded_para, cascaded_run, complete_body, family_font,
    fitted_autofit,
};

use crate::container::{try_map_text_bodies, unit_error, with_mapped_elements};
use crate::shrink_fit::fit_shrink;

pub struct TextMetrics;

static INFO: PassInfo = PassInfo::new(
    "text-metrics",
    1,
    &[Invariant::FontsBound],
    &[Invariant::TextMetrics],
);

impl SlidePass for TextMetrics {
    fn info(&self) -> &PassInfo {
        &INFO
    }

    fn run(
        &self,
        view: &ContainerView<'_>,
        ctx: &mut PassCtx<'_>,
    ) -> Result<OwnedContainer, PassError> {
        let fonts = ctx.fonts;
        let diag = &mut *ctx.diag;
        with_mapped_elements(view.container, |element| {
            try_map_text_bodies(element, |text, site| {
                let text = fit_shrink(text, site, element, view, fonts, diag)?;
                measure_body(&text, site, element.id(), view)
            })
        })
    }
}

/// The line height a size-free basis gives one unit of font size; `coretext` has none, as its line depends on the
/// size ([`basis_line`]).
pub fn basis_ratio(basis: LineBasis, metrics: &FontMetrics) -> Option<f64> {
    let upm = f64::from(metrics.units_per_em);
    let (hhea, typo, win) = (&metrics.hhea, &metrics.typo, &metrics.win);
    Some(match basis {
        LineBasis::Size1_2 => 1.2,
        LineBasis::Hhea => f64::from(hhea.ascender - hhea.descender + hhea.line_gap) / upm,
        LineBasis::Typo => f64::from(typo.ascender - typo.descender + typo.line_gap) / upm,
        LineBasis::Win => f64::from(win.ascent + win.descent) / upm,
        LineBasis::Coretext => return None,
    })
}

fn hhea_ratio(metrics: &FontMetrics) -> f64 {
    let hhea = &metrics.hhea;
    f64::from(hhea.ascender - hhea.descender + hhea.line_gap) / f64::from(metrics.units_per_em)
}

/// CoreText's line parts at a size, in whole points; the synthesized leading sits above the text.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CoreTextLine {
    pub(crate) ascent: f64,
    pub(crate) descent: f64,
    gap: f64,
    leading: f64,
}

/// Whether CoreText gives a font a fifth of its height as extra leading: it does when the font has no line gap,
/// its glyphs reach past the hhea ascent or descent, and À rises above the ascent (Helvetica, Times, Palatino).
fn synthesizes_leading(metrics: &FontMetrics) -> bool {
    let hhea = &metrics.hhea;
    match (&metrics.bounds, metrics.grave_top) {
        (Some(bounds), Some(grave_top)) if hhea.line_gap == 0 => {
            (bounds.y_max > hhea.ascender || bounds.y_min < hhea.descender)
                && grave_top > hhea.ascender
        }
        _ => false,
    }
}

pub(crate) fn core_text_line(metrics: &FontMetrics, size: f64) -> CoreTextLine {
    let upm = f64::from(metrics.units_per_em);
    let points = |units: i32| js_round(f64::from(units) / upm * size);
    let ascent = points(metrics.hhea.ascender);
    let descent = points(-metrics.hhea.descender);
    let leading = if synthesizes_leading(metrics) {
        js_round(0.2 * (ascent + descent))
    } else {
        0.0
    };
    CoreTextLine {
        ascent,
        descent,
        gap: points(metrics.hhea.line_gap),
        leading,
    }
}

/// The single line a basis gives at a size, in points.
pub fn basis_line(basis: LineBasis, metrics: &FontMetrics, size: f64) -> f64 {
    match basis_ratio(basis, metrics) {
        Some(ratio) => size * ratio,
        None => {
            let line = core_text_line(metrics, size);
            line.leading + line.ascent + line.descent + line.gap
        }
    }
}

/// A shrink's scales.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Scale {
    pub(crate) font: f64,
    pub(crate) reduction: f64,
}

/// A run's size and the metrics of the face it draws with.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Strut<'a> {
    pub(crate) size: f64,
    pub(crate) metrics: &'a FontMetrics,
}

pub(crate) fn strut_for<'a>(
    run: &CascadedRun,
    view: &ContainerView<'a>,
    node: &NodeId,
) -> Result<Strut<'a>, PassError> {
    let family = &family_font(&run.font, Some(node))?.family;
    let bound = binding(view.fonts, family, node)?;
    Ok(Strut {
        size: run.size.to_f64(),
        metrics: bound.face_metrics(run.weight, run.italic),
    })
}

/// The cascaded run of the paragraph's strut.
pub(crate) fn strut_of(
    paragraph: &SourceParagraph,
    node: &NodeId,
) -> Result<CascadedRun, PassError> {
    let strut = strut_index(&paragraph.inlines).and_then(|i| paragraph.inlines.get(i));
    let props = match strut {
        Some(Inline::Run { props, .. } | Inline::Field { props, .. }) => Some(props),
        _ => paragraph.end_props.as_ref(),
    };
    Ok(cascaded_run(props, node)?)
}

fn ratio(x: f64) -> Result<Ratio, PassError> {
    Ratio::quantise(x).map_err(|e| unit_error("A line metric", e))
}

fn pt(x: f64) -> Result<Pt, PassError> {
    Pt::quantise(x).map_err(|e| unit_error("A line metric", e))
}

fn pt_height(x: f64) -> Result<LineHeight, PassError> {
    Ok(LineHeight::Pt {
        value: Size::quantise(x).map_err(|e| unit_error("A line height", e))?,
    })
}

fn line_height_of(
    spacing: &LineSpacing,
    strut: Strut<'_>,
    scale: Scale,
) -> Result<LineHeight, PassError> {
    let natural = strut.size * scale.font * hhea_ratio(strut.metrics);
    match spacing {
        LineSpacing::Multiple { value, basis } => {
            let multiple = (value.to_f64() - scale.reduction).max(0.0);
            let value = match basis_ratio(*basis, strut.metrics) {
                Some(basis) => multiple * basis,
                None => {
                    let size = strut.size * scale.font;
                    let line = core_text_line(strut.metrics, size);
                    if size > 0.0 {
                        (multiple * (line.leading + line.ascent + line.descent) + line.gap) / size
                    } else {
                        multiple
                    }
                }
            };
            Ok(LineHeight::Ratio {
                value: ratio(value)?,
            })
        }
        LineSpacing::Exact { value } => pt_height(value.to_f64() * scale.font),
        LineSpacing::AtLeast { value } => pt_height(value.to_f64().max(natural)),
        LineSpacing::AtMost { value } => pt_height(value.to_f64().min(natural)),
        LineSpacing::Between { min, max } => pt_height(max.to_f64().min(min.to_f64().max(natural))),
    }
}

/// The single line's share above the baseline in the `size-1.2` basis.
const SIZE_1_2_ASCENT: f64 = 0.8;

/// The leading above the text less the leading below it, as a ratio of the size, when the basis makes them
/// differ.
fn leading_skew_of(
    spacing: &LineSpacing,
    strut: Strut<'_>,
    scale: Scale,
) -> Result<Option<Ratio>, PassError> {
    let LineSpacing::Multiple { value, basis } = spacing else {
        return Ok(None);
    };
    let multiple = (value.to_f64() - scale.reduction).max(0.0);
    let skew = match basis {
        LineBasis::Coretext => {
            let size = strut.size * scale.font;
            let line = core_text_line(strut.metrics, size);
            // The single line's leading stays above the text, and the space a multiple adds or takes off is
            // below it.
            let skew =
                line.leading - (multiple - 1.0) * (line.leading + line.ascent + line.descent);
            if size > 0.0 { skew / size } else { 0.0 }
        }
        LineBasis::Size1_2 => {
            let upm = f64::from(strut.metrics.units_per_em);
            let hhea = &strut.metrics.hhea;
            let single = 1.2;
            let line = multiple * single;
            // The baseline sits at 0.8 of the single line (of the whole line when that is shorter); the rest is
            // below.
            let baseline = SIZE_1_2_ASCENT * line.min(single);
            let above = baseline - f64::from(hhea.ascender) / upm;
            let below = line - baseline + f64::from(hhea.descender) / upm;
            above - below
        }
        _ => return Ok(None),
    };
    let skew = ratio(skew)?;
    Ok((skew.steps() != 0).then_some(skew))
}

/// How far the line box reaches below the end of the text: the space a multiple adds below the text, and in a
/// table cell the font's line gap, which Keynote leaves out of a row's height (but keeps in a shape's). A
/// `coretext` multiple under 1 makes it negative: the text's height still ends at its last single line.
fn line_tail_of(
    spacing: &LineSpacing,
    strut: Strut<'_>,
    scale: Scale,
    site: TextSite,
) -> Result<Option<Pt>, PassError> {
    let LineSpacing::Multiple { value, basis } = spacing else {
        return Ok(None);
    };
    let multiple = (value.to_f64() - scale.reduction).max(0.0);
    let size = strut.size * scale.font;
    let tail = match basis {
        LineBasis::Size1_2 => (multiple - 1.0).max(0.0) * size * 1.2,
        LineBasis::Coretext => {
            let line = core_text_line(strut.metrics, size);
            let gap = match site {
                TextSite::Cell => {
                    f64::from(strut.metrics.hhea.line_gap) / f64::from(strut.metrics.units_per_em)
                        * size
                }
                TextSite::Shape => 0.0,
            };
            (multiple - 1.0) * (line.leading + line.ascent + line.descent) + gap
        }
        _ => return Ok(None),
    };
    let tail = pt(tail)?;
    Ok((tail.steps() != 0).then_some(tail))
}

fn spacing_pt(spacing: &Spacing, line: f64) -> Result<Pt, PassError> {
    pt(match spacing {
        Spacing::Pt { value } => value.to_f64(),
        Spacing::Pct { value } => value.to_f64() * line,
    })
}

fn measure_body(
    text: &TextBody,
    site: TextSite,
    node: &NodeId,
    view: &ContainerView<'_>,
) -> Result<TextBody, PassError> {
    let autofit = fitted_autofit(&complete_body(&text.body, node)?.autofit, node)?;
    let scale = match autofit {
        ResolvedAutofit::Shrink {
            font_scale,
            line_spacing_reduction,
            ..
        } => Scale {
            font: font_scale.to_f64(),
            reduction: line_spacing_reduction.to_f64(),
        },
        _ => Scale {
            font: 1.0,
            reduction: 0.0,
        },
    };
    measure_at(text, site, node, view, scale)
}

/// `text` with the line metrics of its paragraphs and runs at `scale`.
pub(crate) fn measure_at(
    text: &TextBody,
    site: TextSite,
    node: &NodeId,
    view: &ContainerView<'_>,
    scale: Scale,
) -> Result<TextBody, PassError> {
    let mut out = text.clone();
    for paragraph in &mut out.paragraphs {
        let props = cascaded_para(&paragraph.props, node)?;
        let strut = strut_for(&strut_of(paragraph, node)?, view, node)?;
        let basis = match props.line_spacing {
            LineSpacing::Multiple { basis, .. } => basis,
            _ => LineBasis::Hhea,
        };
        let line = basis_line(basis, strut.metrics, strut.size * scale.font);
        let line_height = line_height_of(&props.line_spacing, strut, scale)?;
        let leading_skew = leading_skew_of(&props.line_spacing, strut, scale)?;
        let line_tail_pt = line_tail_of(&props.line_spacing, strut, scale, site)?;
        // Each run's own line, where its face or size makes it differ: a line is as tall as its tallest run.
        for inline in &mut paragraph.inlines {
            let empty_run = matches!(inline, Inline::Run { text, .. } if text.is_empty());
            let (Inline::Run {
                props: run,
                line_height: own_height,
                leading_skew: own_skew,
                ..
            }
            | Inline::Field {
                props: run,
                line_height: own_height,
                leading_skew: own_skew,
                ..
            }) = inline
            else {
                continue;
            };
            *own_height = None;
            *own_skew = None;
            let LineHeight::Ratio {
                value: paragraph_ratio,
            } = line_height
            else {
                continue;
            };
            if empty_run {
                continue;
            }
            let face = strut_for(&cascaded_run(Some(run), node)?, view, node)?;
            if let LineHeight::Ratio { value } = line_height_of(&props.line_spacing, face, scale)?
                && value != paragraph_ratio
            {
                *own_height = Some(LineHeight::Ratio { value });
            }
            *own_skew = leading_skew_of(&props.line_spacing, face, scale)?;
        }
        let para = &mut paragraph.props;
        para.line_height = Some(line_height);
        para.leading_skew = leading_skew;
        para.line_tail_pt = line_tail_pt;
        para.space_before_pt = Some(spacing_pt(&props.space_before, line)?);
        para.space_after_pt = Some(spacing_pt(&props.space_after, line)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use slidock_ir::fonts::{GlyphBounds, VMetrics, WinMetrics};

    /// Liberation Sans's metrics.
    fn sans() -> FontMetrics {
        FontMetrics {
            units_per_em: 2048,
            hhea: VMetrics {
                ascender: 1854,
                descender: -434,
                line_gap: 67,
            },
            typo: VMetrics {
                ascender: 1491,
                descender: -431,
                line_gap: 307,
            },
            win: WinMetrics {
                ascent: 1854,
                descent: 434,
            },
            bounds: None,
            grave_top: None,
        }
    }

    #[test]
    fn ratios_per_basis() {
        let m = sans();
        assert_eq!(basis_ratio(LineBasis::Size1_2, &m), Some(1.2));
        assert_eq!(basis_ratio(LineBasis::Hhea, &m), Some(2355.0 / 2048.0));
        assert_eq!(basis_ratio(LineBasis::Win, &m), Some(2288.0 / 2048.0));
        assert_eq!(basis_ratio(LineBasis::Coretext, &m), None);
    }

    #[test]
    fn rounds_the_core_text_line_to_whole_points() {
        // 18 pt: ascent 16.29 → 16, descent 3.81 → 4, gap 0.59 → 1.
        assert_eq!(basis_line(LineBasis::Coretext, &sans(), 18.0), 21.0);
        let synthesized = FontMetrics {
            hhea: VMetrics {
                line_gap: 0,
                ..sans().hhea
            },
            bounds: Some(GlyphBounds {
                y_min: -900,
                y_max: 2000,
            }),
            grave_top: Some(1900),
            ..sans()
        };
        // A fifth of the 20 pt line as leading above it.
        assert_eq!(basis_line(LineBasis::Coretext, &synthesized, 18.0), 24.0);
        let low_grave = FontMetrics {
            grave_top: Some(1800),
            ..synthesized
        };
        assert_eq!(basis_line(LineBasis::Coretext, &low_grave, 18.0), 20.0);
    }

    /// Party LET's metrics: no line gap, glyphs past the ascent, a tall À, so CoreText synthesizes leading.
    fn party() -> FontMetrics {
        FontMetrics {
            units_per_em: 2048,
            hhea: VMetrics {
                ascender: 1365,
                descender: -1024,
                line_gap: 0,
            },
            typo: VMetrics {
                ascender: 1365,
                descender: -1024,
                line_gap: 0,
            },
            win: WinMetrics {
                ascent: 1789,
                descent: 1140,
            },
            bounds: Some(GlyphBounds {
                y_min: -1140,
                y_max: 1789,
            }),
            grave_top: Some(1389),
        }
    }

    fn core_text(value: f64) -> LineSpacing {
        LineSpacing::Multiple {
            value: Ratio::quantise(value).unwrap(),
            basis: LineBasis::Coretext,
        }
    }

    const PLAIN: Scale = Scale {
        font: 1.0,
        reduction: 0.0,
    };

    /// The skew in points and the tail of a shape's text, at a size.
    fn skew_and_tail(metrics: &FontMetrics, value: f64, size: f64) -> (f64, f64) {
        let strut = Strut { size, metrics };
        let spacing = core_text(value);
        let skew = leading_skew_of(&spacing, strut, PLAIN).unwrap();
        let tail = line_tail_of(&spacing, strut, PLAIN, TextSite::Shape).unwrap();
        (
            skew.map_or(0.0, |s| s.to_f64() * size),
            tail.map_or(0.0, Pt::to_f64),
        )
    }

    #[test]
    fn keeps_the_core_text_leading_above_the_text_and_takes_a_multiple_under_1_off_below_it() {
        // 300 pt Party LET: ascent 199.95 → 200, descent 150, leading 0.2 × 350 = 70; a 420 pt single line, and
        // 0.8 × 420 = 336 pt lines. Keynote draws the baseline 70 + 200 pt down the line, as at 1, and ends the
        // text 420 pt below its last line's top: one line, centred on 540 pt, has its baseline at 600 pt.
        let m = party();
        assert_eq!(
            line_height_of(
                &core_text(0.8),
                Strut {
                    size: 300.0,
                    metrics: &m
                },
                PLAIN
            )
            .unwrap(),
            LineHeight::Ratio {
                value: Ratio::quantise(1.12).unwrap()
            }
        );
        let (skew, tail) = skew_and_tail(&m, 0.8, 300.0);
        // Above the text 70 pt, below it 336 − 70 − 350 = −84 pt.
        assert!((skew - (70.0 + 84.0)).abs() < 0.01, "{skew}");
        assert_eq!(tail, -84.0);
        let (line, half) = (336.0, (336.0 - 350.0 + skew) / 2.0);
        let height = line - tail;
        assert!((540.0 - height / 2.0 + half + 200.0 - 600.0).abs() < 0.01);
        // At 1 and over it the same rule: the single line's leading above, the extra below.
        let (single_skew, single_tail) = skew_and_tail(&m, 1.0, 300.0);
        assert!((single_skew - 70.0).abs() < 0.01, "{single_skew}");
        assert_eq!(single_tail, 0.0);
        let (wide_skew, wide_tail) = skew_and_tail(&m, 1.5, 100.0);
        assert!((wide_skew - (23.0 - 70.0)).abs() < 0.01, "{wide_skew}");
        assert_eq!(wide_tail, 70.0);
    }

    #[test]
    fn takes_a_core_text_multiple_under_1_off_below_the_text_without_synthesized_leading() {
        // 65 pt with a 750/150/23 of 1000 face: ascent 48.75 → 49, descent 9.75 → 10, no leading; 0.9 × 59 pt.
        let m = FontMetrics {
            units_per_em: 1000,
            hhea: VMetrics {
                ascender: 750,
                descender: -150,
                line_gap: 23,
            },
            bounds: None,
            grave_top: None,
            ..party()
        };
        let (skew, tail) = skew_and_tail(&m, 0.9, 65.0);
        assert!((skew - 5.9).abs() < 0.01, "{skew}");
        assert!((tail + 5.9).abs() < 0.001, "{tail}");
        // A `size-1.2` multiple under 1 keeps no tail.
        let size_1_2 = LineSpacing::Multiple {
            value: Ratio::quantise(0.9).unwrap(),
            basis: LineBasis::Size1_2,
        };
        let strut = Strut {
            size: 65.0,
            metrics: &m,
        };
        assert_eq!(
            line_tail_of(&size_1_2, strut, PLAIN, TextSite::Shape).unwrap(),
            None
        );
    }
}
