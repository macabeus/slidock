//! The scale of a shrink that stores none, for text-metrics to measure with: the text is laid out at each scale its
//! source app tries ([`ShrinkSteps`]), largest first, and takes the first at which it fits its box, or the last when
//! none does. `office` is PowerPoint's steps, which LibreOffice takes (`editeng` `constScaleLevels` and
//! `ScaleContentToFitWindow`, libreoffice-25.2.3.2); `percent` is Keynote's whole percents.
//! - Lines break where the printed text breaks them (`white-space: pre-wrap; overflow-wrap: break-word`): after
//!   spaces, tabs and hyphens and around ideographs, inside a word only when it is wider than its line, with the
//!   spaces at a line's end hanging. A character is as wide as its glyph's advance in the face its run draws with,
//!   unshaped, at the run's scaled size, plus the run's letter spacing; a paragraph's first line starts after its
//!   indent and bullet, and a tab reaches the next multiple of the tab size.
//! - The text fits when its glyphs fit the box less its insets: from the top of its first line (or of the glyphs,
//!   where they sit lower in it) to the bottom of its last line's glyphs. The leading above the first line and below
//!   the last takes no room, as Keynote and LibreOffice measure text; paragraph spacing and the lines between count
//!   whole, at the line heights text-metrics gives each scale, a line being as tall as its tallest run.
//! - A face whose file the catalog does not hold (a family bound to the built-in metrics) has no glyph widths: its
//!   text keeps a scale of 1, not fitted (MID5403). A table cell's row grows with its text, which fits at 1. A family
//!   bound to the `emoji` generic is as wide as the platform emoji face draws it
//!   ([`slidock_fonts::catalog::emoji::emoji_advances`]): each cluster one glyph's advance.

use std::borrow::Cow;

use slidock_core_api::visit::TextSite;
use slidock_core_api::{ContainerView, PassError};
use slidock_fonts::catalog::emoji::emoji_advances;
use slidock_fonts::{FaceFile, FamilyKey, FontCatalog};
use slidock_ir::diagnostics::{DiagnosticSink, DiagnosticSite, codes};
use slidock_ir::fonts::{FaceStyle, GenericFamily};
use slidock_ir::ids::NodeId;
use slidock_ir::source::Element;
use slidock_ir::text::{
    Autofit, Bullet, Caps, Inline, LineBasis, LineHeight, LineSpacing, ShrinkSteps, TextBody, Wrap,
};
use slidock_ir::units::UnitRatio;
use slidock_ir::verify::{
    binding, cascaded_para, cascaded_run, complete_body, family_font, framed,
};

use crate::text_metrics::{Scale, core_text_line, measure_at, strut_for, strut_of};

/// A scale in grid steps of 1/100 000: the font's, and the line spacing taken off.
type Step = (i64, i64);

const FULL: Step = (100_000, 0);

/// PowerPoint's steps after the full size, as LibreOffice's `constScaleLevels` lists them.
const OFFICE: [Step; 13] = [
    (100_000, 10_000),
    (92_500, 10_000),
    (92_500, 20_000),
    (85_000, 10_000),
    (85_000, 20_000),
    (77_500, 20_000),
    (70_000, 20_000),
    (62_500, 20_000),
    (55_000, 20_000),
    (47_500, 20_000),
    (40_000, 20_000),
    (32_500, 20_000),
    (25_000, 20_000),
];

/// The scales `steps` tries, largest first.
pub(crate) fn steps_of(steps: ShrinkSteps) -> Vec<Step> {
    match steps {
        ShrinkSteps::Office => std::iter::once(FULL).chain(OFFICE).collect(),
        ShrinkSteps::Percent => (1..=100)
            .rev()
            .map(|percent| (percent * 1_000, 0))
            .collect(),
    }
}

/// How a line may break around a character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Normal,
    /// A space: a break may follow it, and it hangs at a line's end.
    Space,
    /// A tab: a space that reaches the next tab stop.
    Tab,
    /// A break may follow it (a hyphen, a dash, a zero-width space).
    BreakAfter,
    /// A break may come before and after it.
    Ideograph,
    /// A forced line break.
    Newline,
}

fn is_ideograph(ch: char) -> bool {
    matches!(u32::from(ch),
        0x2E80..=0x303F | 0x3040..=0x30FF | 0x3100..=0x31FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFF60 | 0x20000..=0x3FFFF)
}

fn class_of(ch: char, next: Option<char>) -> Class {
    match ch {
        ' ' | '\u{3000}' => Class::Space,
        '\t' => Class::Tab,
        '\n' | '\r' | '\u{b}' | '\u{2028}' | '\u{2029}' => Class::Newline,
        // A hyphen-minus before a number is its sign.
        '-' if next.is_some_and(|next| next.is_ascii_digit()) => Class::Normal,
        '-' | '\u{ad}' | '\u{200b}' | '\u{2010}' | '\u{2013}' | '\u{2014}' => Class::BreakAfter,
        _ if is_ideograph(ch) => Class::Ideograph,
        _ => Class::Normal,
    }
}

/// The width a character the face has no glyph for takes, in ems: a fallback face draws it.
fn fallback_em(ch: char) -> f64 {
    match ch {
        '\u{ad}' | '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{feff}' => 0.0,
        _ if is_ideograph(ch) => 1.0,
        _ => 0.5,
    }
}

/// A character in a paragraph's line: its width at full size (`size`, which a scale multiplies, and `fixed`, which
/// it does not), how a line breaks around it, and the inline it belongs to.
#[derive(Clone, Copy, Debug)]
struct Glyph {
    size: f64,
    fixed: f64,
    class: Class,
    inline: usize,
}

impl Glyph {
    fn width(&self, scale: f64) -> f64 {
        self.size * scale + self.fixed
    }
}

/// A paragraph as its lines break it.
#[derive(Debug)]
struct Paragraph {
    glyphs: Vec<Glyph>,
    /// The width of the bullet label at full size, when it shows.
    bullet: Option<f64>,
    margin_left: f64,
    margin_right: f64,
    indent: f64,
    tab_size: f64,
}

/// The face `family` draws at `weight` and `italic`, and the weight it draws at, when the catalog holds its file.
fn face_file<'f>(
    fonts: &'f FontCatalog,
    view: &ContainerView<'_>,
    family: &str,
    weight: u32,
    italic: bool,
    node: &NodeId,
) -> Result<Option<(&'f FaceFile, u32)>, PassError> {
    let bound = binding(view.fonts, family, node)?;
    let drawn = bound.drawn_weight(weight);
    let style = if italic {
        FaceStyle::Italic
    } else {
        FaceStyle::Normal
    };
    Ok(fonts
        .get(&FamilyKey::of(&bound.family))
        .and_then(|catalog| catalog.css_match(drawn, style))
        .and_then(|face| face.file.as_ref())
        .map(|file| (file, drawn)))
}

/// The advances of `text` in ems, drawn by `family`; `Err` names a family whose face has no file.
fn advances(
    fonts: &FontCatalog,
    view: &ContainerView<'_>,
    family: &str,
    weight: u32,
    italic: bool,
    text: &str,
    node: &NodeId,
) -> Result<Result<Vec<f64>, String>, PassError> {
    if binding(view.fonts, family, node)?.generic == GenericFamily::Emoji {
        return Ok(Ok(emoji_advances(text)));
    }
    let Some((file, drawn)) = face_file(fonts, view, family, weight, italic, node)? else {
        return Ok(Err(family.to_owned()));
    };
    let Some(advances) = file.advances(text, drawn) else {
        return Ok(Err(family.to_owned()));
    };
    Ok(Ok(text
        .chars()
        .zip(advances)
        .map(|(ch, advance)| advance.unwrap_or_else(|| fallback_em(ch)))
        .collect()))
}

/// What a run shows: its text, upper-cased for all caps; small caps draw a lower-case letter as a capital at 70 %, as
/// browsers synthesize them.
fn shown(text: &str, caps: Caps) -> (String, Vec<f64>) {
    let mut out = String::with_capacity(text.len());
    let mut factors = Vec::with_capacity(text.len());
    for ch in text.chars() {
        let (upper, factor) = match caps {
            Caps::None => (None, 1.0),
            Caps::All => (Some(ch), 1.0),
            Caps::Small if ch.is_lowercase() => (Some(ch), 0.7),
            Caps::Small => (None, 1.0),
        };
        match upper {
            Some(ch) => {
                for upper in ch.to_uppercase() {
                    out.push(upper);
                    factors.push(factor);
                }
            }
            None => {
                out.push(ch);
                factors.push(factor);
            }
        }
    }
    (out, factors)
}

/// The paragraphs of `text` as lines break them; `Err` names a family with no glyph widths.
fn paragraphs(
    text: &TextBody,
    view: &ContainerView<'_>,
    fonts: &FontCatalog,
    node: &NodeId,
) -> Result<Result<Vec<Paragraph>, String>, PassError> {
    let mut out = Vec::with_capacity(text.paragraphs.len());
    for paragraph in &text.paragraphs {
        let props = cascaded_para(&paragraph.props, node)?;
        let mut glyphs = Vec::new();
        for (index, inline) in paragraph.inlines.iter().enumerate() {
            let (text, props) = match inline {
                Inline::Run { text, props, .. } => (text, props),
                Inline::Field { cached, props, .. } => (cached, props),
                Inline::Break { .. } | Inline::Tab {} => {
                    let class = match inline {
                        Inline::Tab {} => Class::Tab,
                        _ => Class::Newline,
                    };
                    glyphs.push(Glyph {
                        size: 0.0,
                        fixed: 0.0,
                        class,
                        inline: index,
                    });
                    continue;
                }
            };
            if text.is_empty() {
                continue;
            }
            let run = cascaded_run(Some(props), node)?;
            let family = &family_font(&run.font, Some(node))?.family;
            let (shown, factors) = shown(text, run.caps);
            let ems = match advances(fonts, view, family, run.weight, run.italic, &shown, node)? {
                Ok(ems) => ems,
                Err(family) => return Ok(Err(family)),
            };
            let size = run.size.to_f64();
            let fixed = run.letter_spacing.to_f64();
            let chars: Vec<char> = shown.chars().collect();
            for (at, ((ch, em), factor)) in chars.iter().zip(ems).zip(factors).enumerate() {
                glyphs.push(Glyph {
                    size: em * factor * size,
                    fixed,
                    class: class_of(*ch, chars.get(at + 1).copied()),
                    inline: index,
                });
            }
        }
        let bullet = match &props.bullet {
            Bullet::Label {
                text: label,
                font,
                size,
                ..
            } if !paragraph.shows_no_text() => {
                let strut = strut_of(paragraph, node)?;
                match advances(
                    fonts,
                    view,
                    &font.family,
                    strut.weight,
                    strut.italic,
                    label,
                    node,
                )? {
                    Ok(ems) => Some(ems.iter().sum::<f64>() * size.to_f64()),
                    Err(family) => return Ok(Err(family)),
                }
            }
            _ => None,
        };
        out.push(Paragraph {
            glyphs,
            bullet,
            margin_left: props.margin_left.to_f64(),
            margin_right: props.margin_right.to_f64(),
            indent: props.indent.to_f64(),
            tab_size: props.tab_size.to_f64(),
        });
    }
    Ok(Ok(out))
}

/// Slack for comparing lengths in points.
const EPSILON: f64 = 1e-6;

/// The lines of `paragraph` at `scale` in a box `width` wide (unbounded when it does not wrap): the inlines each
/// line draws.
fn lines(paragraph: &Paragraph, scale: f64, width: Option<f64>) -> Vec<Vec<usize>> {
    let bullet = paragraph
        .bullet
        .map_or(0.0, |label| (label * scale).max(-paragraph.indent));
    let first_start = paragraph.indent + bullet;
    let room = |first: bool| {
        width.map_or(f64::INFINITY, |width| {
            width
                - paragraph.margin_left
                - paragraph.margin_right
                - if first { first_start } else { 0.0 }
        })
    };
    let mut lines: Vec<Vec<usize>> = vec![Vec::new()];
    let mut first = true;
    let mut x = 0.0;
    let mut filled = false;
    let glyphs = &paragraph.glyphs;
    let mut at = 0;
    while let Some(glyph) = glyphs.get(at) {
        if glyph.class == Class::Newline {
            mark(&mut lines, glyph.inline);
            lines.push(Vec::new());
            (first, x, filled) = (false, 0.0, false);
            at += 1;
            continue;
        }
        // The word: up to a break opportunity, then the spaces after it.
        let mut end = at;
        let mut word = 0.0;
        while let Some(next) = glyphs.get(end) {
            if matches!(next.class, Class::Space | Class::Tab | Class::Newline)
                || (next.class == Class::Ideograph && end > at)
            {
                break;
            }
            word += next.width(scale);
            end += 1;
            if matches!(next.class, Class::BreakAfter | Class::Ideograph) {
                break;
            }
        }
        if filled && x + word > room(first) + EPSILON {
            lines.push(Vec::new());
            (first, x, filled) = (false, 0.0, false);
        }
        if word > room(first) + EPSILON {
            // Wider than a whole line: it breaks between its characters.
            for glyph in glyphs.get(at..end).unwrap_or(&[]) {
                let w = glyph.width(scale);
                if filled && x + w > room(first) + EPSILON {
                    lines.push(Vec::new());
                    (first, x) = (false, 0.0);
                }
                x += w;
                filled = true;
                mark(&mut lines, glyph.inline);
            }
        } else {
            for glyph in glyphs.get(at..end).unwrap_or(&[]) {
                mark(&mut lines, glyph.inline);
            }
            x += word;
            filled |= end > at;
        }
        at = end;
        // The spaces hang at the line's end, whatever their width.
        while let Some(space) = glyphs.get(at) {
            let w = match space.class {
                Class::Space => space.width(scale),
                Class::Tab if paragraph.tab_size > 0.0 => {
                    let start = if first { first_start } else { 0.0 };
                    let from = start + x;
                    paragraph.tab_size - from.rem_euclid(paragraph.tab_size)
                }
                Class::Tab => 0.0,
                _ => break,
            };
            x += w;
            filled = true;
            mark(&mut lines, space.inline);
            at += 1;
        }
    }
    lines
}

/// Adds `inline` to the inlines the last line draws.
fn mark(lines: &mut [Vec<usize>], inline: usize) {
    if let Some(line) = lines.last_mut()
        && line.last() != Some(&inline)
    {
        line.push(inline);
    }
}

/// A line height in points, for a run or paragraph of `size` (already scaled).
fn line_pt(line: Option<&LineHeight>, size: f64) -> f64 {
    match line {
        Some(LineHeight::Ratio { value }) => value.to_f64() * size,
        Some(LineHeight::Pt { value }) => value.to_f64(),
        None => 0.0,
    }
}

/// The height of `measured`'s glyphs laid out in `lines` at `scale`: from its first line's top (or its glyphs',
/// where lower) to the bottom of its last line's glyphs.
fn glyph_height(
    measured: &TextBody,
    lines: &[Vec<Vec<usize>>],
    scale: Scale,
    view: &ContainerView<'_>,
    node: &NodeId,
) -> Result<f64, PassError> {
    let mut y = 0.0;
    let mut top: Option<f64> = None;
    let mut bottom = 0.0;
    for (paragraph, lines) in measured.paragraphs.iter().zip(lines) {
        let props = &paragraph.props;
        let strut = strut_for(&strut_of(paragraph, node)?, view, node)?;
        let size = strut.size * scale.font;
        let base = line_pt(props.line_height.as_ref(), size);
        let content = match props.line_spacing {
            Some(LineSpacing::Multiple {
                basis: LineBasis::Coretext,
                ..
            }) => {
                let line = core_text_line(strut.metrics, size);
                line.ascent + line.descent
            }
            _ => {
                let hhea = &strut.metrics.hhea;
                f64::from(hhea.ascender - hhea.descender) / f64::from(strut.metrics.units_per_em)
                    * size
            }
        };
        let skew = props.leading_skew.map_or(0.0, |skew| skew.to_f64() * size);
        y += props.space_before_pt.map_or(0.0, |space| space.to_f64());
        for line in lines {
            let mut height = base;
            for &index in line {
                if let Some(
                    Inline::Run {
                        props: run,
                        line_height: Some(own),
                        ..
                    }
                    | Inline::Field {
                        props: run,
                        line_height: Some(own),
                        ..
                    },
                ) = paragraph.inlines.get(index)
                {
                    let run_size = cascaded_run(Some(run), node)?.size.to_f64() * scale.font;
                    height = height.max(line_pt(Some(own), run_size));
                }
            }
            let above = (height - content + skew) / 2.0;
            top.get_or_insert(y + above.max(0.0));
            bottom = y + above + content;
            y += height;
        }
        y += props.space_after_pt.map_or(0.0, |space| space.to_f64());
    }
    Ok(bottom - top.unwrap_or(0.0))
}

/// `text` with the scale of a shrink that stores none: fitted to its box, or 1 and not fitted (MID5403) when a face it
/// draws has no glyph widths. Any other text is returned as it is.
pub(crate) fn fit_shrink<'t>(
    text: &'t TextBody,
    site: TextSite,
    element: &Element,
    view: &ContainerView<'_>,
    fonts: &FontCatalog,
    diag: &mut dyn DiagnosticSink,
) -> Result<Cow<'t, TextBody>, PassError> {
    let Some(Autofit::Shrink {
        font_scale: None,
        line_spacing_reduction: None,
        steps,
        ..
    }) = &text.body.autofit
    else {
        return Ok(Cow::Borrowed(text));
    };
    let node = element.id();
    let body = complete_body(&text.body, node)?;
    let ((font, reduction), fitted) = match site {
        TextSite::Cell => (FULL, true),
        TextSite::Shape => match paragraphs(text, view, fonts, node)? {
            Ok(paragraphs) => {
                let frame = framed(element)?;
                let insets = &body.insets;
                let width = (body.wrap != Wrap::None)
                    .then(|| frame.w.to_f64() - insets.l.to_f64() - insets.r.to_f64());
                let room = frame.h.to_f64() - insets.t.to_f64() - insets.b.to_f64();
                let candidates = steps_of(steps.unwrap_or(ShrinkSteps::Percent));
                let mut chosen = FULL;
                for &(font, reduction) in &candidates {
                    chosen = (font, reduction);
                    let scale = Scale {
                        font: UnitRatio::from_steps(font).to_f64(),
                        reduction: UnitRatio::from_steps(reduction).to_f64(),
                    };
                    let measured = measure_at(text, site, node, view, scale)?;
                    let lines: Vec<Vec<Vec<usize>>> = paragraphs
                        .iter()
                        .map(|paragraph| lines(paragraph, scale.font, width))
                        .collect();
                    if glyph_height(&measured, &lines, scale, view, node)? <= room + EPSILON {
                        break;
                    }
                }
                (chosen, true)
            }
            Err(family) => {
                diag.report(
                    codes::MID5403,
                    DiagnosticSite::new()
                        .container(view.container.id().clone())
                        .node(node.clone())
                        .data("family", family),
                );
                (FULL, false)
            }
        },
    };
    let mut out = text.clone();
    out.body.autofit = Some(Autofit::Shrink {
        font_scale: Some(UnitRatio::from_steps(font)),
        line_spacing_reduction: Some(UnitRatio::from_steps(reduction)),
        fitted: Some(fitted),
        steps: None,
    });
    Ok(Cow::Owned(out))
}
