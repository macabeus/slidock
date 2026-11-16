//! Text: a text box per body (the shape or the table cell itself where it holds nothing else), a `<p>` per paragraph
//! with a unitless line height, each run as text or a `<span>` where it differs from what it inherits, bullets as hung
//! labels, and consecutive labelled paragraphs as the `<li>`s of a list. The text box preserves whitespace (`pre-wrap`,
//! or `pre` unwrapped), so every space inside a paragraph comes from the text. A stored shrink scales run, label and
//! strut sizes here; line heights arrive computed by text-metrics.
//!
//! Each paragraph block carries `data-p`, its index in the body, which paragraph-range builds address.

use indexmap::IndexMap;
use slidock_canon::{js_number, to_js_string, utf16};
use slidock_format::css_string;
use slidock_format::markers::{BULLET, PARAGRAPH, TEXT, attr};
use slidock_ir::diagnostics::codes;
use slidock_ir::fonts::font_key;
use slidock_ir::frame::Frame;
use slidock_ir::ids::NodeId;
use slidock_ir::text::{
    Align, Anchor, AutoNumScheme, Caps, Decoration, Inline, LineHeight, LineSpacing, Link,
    Overflow, ResolvedAutofit, ResolvedBodyProps, ResolvedBullet, ResolvedInline,
    ResolvedParagraph, ResolvedRunProps, ResolvedTextBody, Wrap,
};
use slidock_ir::units::Ratio;

use crate::color::css_color;
use crate::context::PrintCtx;
use crate::css::{MaybeDecl, decl, decl_if, number, pt, ratio, sides, style};
use crate::error::Result;
use crate::frame::{Autosize, text_turn, text_turn_decl};
use crate::paint_motion::Hook;
use crate::shadow::{ShadowForm, shadow_decl};
use crate::tree::{Attrs, Element, Node, lines, text};

/// The longest link URL a diagnostic quotes, in UTF-16 code units.
const QUOTED_URL: usize = 120;

/// `^(?:https?|mailto):`, case-insensitive.
fn is_safe_url(url: &str) -> bool {
    ["http:", "https:", "mailto:"].iter().any(|scheme| {
        url.get(..scheme.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(scheme))
    })
}

/// A link as one attribute value: `url:<url>`, `slide:<id>` or `action:<name>`. A URL outside the http, https and
/// mailto schemes is dropped with HTML6001.
pub fn link_value(link: &Link, ctx: &mut PrintCtx<'_>, node: &NodeId) -> Result<Option<String>> {
    Ok(match link {
        Link::Url { url } => {
            if is_safe_url(url) {
                Some(format!("url:{url}"))
            } else {
                let site = ctx
                    .site()
                    .node(node.clone())
                    .data("url", utf16::clip(url, QUOTED_URL));
                ctx.diag(codes::HTML6001, site);
                None
            }
        }
        Link::Slide { slide } => Some(format!("slide:{slide}")),
        Link::Action { action } => Some(format!("action:{}", action.as_str())),
    })
}

/// A link around inline content: `<a href>` for URLs, `<a data-link>` for slide and action links.
fn linked(
    link: Option<&Link>,
    child: Element,
    ctx: &mut PrintCtx<'_>,
    node: &NodeId,
) -> Result<Element> {
    let Some(link) = link else {
        return Ok(child);
    };
    let Some(value) = link_value(link, ctx, node)? else {
        return Ok(child);
    };
    let target = match link {
        Link::Url { url } => Attrs::new().val("href", url.clone()),
        _ => Attrs::new().val("data-link", value),
    };
    Ok(Element::new(
        "a",
        target.val("style", "color:inherit;text-decoration:inherit".to_owned()),
        vec![child.into()],
    ))
}

/// The font stack of a family: as requested, then what binds it when that differs, then its generic. A family the deck
/// ships faces of prints the families its rules define instead of the first two, so no font installed
/// on the viewer's computer draws it. A family bound to the `emoji` generic prints the platforms' emoji families
/// before it, whatever it is called: the viewer's emoji font draws it.
fn font_stack(family: &str, ctx: &PrintCtx<'_>) -> Result<String> {
    let key = font_key(family);
    let Some(entry) = ctx.presentation.fonts.get(&key) else {
        return Ok(css_string(family));
    };
    let binding = &entry.binding;
    let platform = binding.generic.platform_families();
    if !platform.is_empty() {
        let mut stack = String::new();
        for name in platform {
            stack.push_str(&css_string(name));
            stack.push(',');
        }
        stack.push_str(binding.generic.as_str());
        return Ok(stack);
    }
    if let Some(shipped) = ctx.stacks.get(&key).filter(|stack| !stack.is_empty()) {
        let mut stack = String::new();
        for name in shipped {
            stack.push_str(&css_string(name));
            stack.push(',');
        }
        stack.push_str(binding.generic.as_str());
        return Ok(stack);
    }
    let mut stack = css_string(family);
    if font_key(&binding.family) != key {
        stack.push(',');
        stack.push_str(&css_string(&binding.family));
    }
    stack.push(',');
    stack.push_str(binding.generic.as_str());
    Ok(stack)
}

/// The weight to print: the binding's drawn weight.
fn font_weight(family: &str, weight: u32, ctx: &PrintCtx<'_>) -> u32 {
    match ctx.presentation.fonts.get(&font_key(family)) {
        Some(entry) => entry.binding.drawn_weight(weight),
        None => weight,
    }
}

/// What a face sets of the text it draws.
#[derive(Clone, Copy)]
struct Face<'a> {
    family: &'a str,
    size: f64,
    weight: u32,
    italic: bool,
}

impl<'a> Face<'a> {
    fn of(props: &'a ResolvedRunProps) -> Face<'a> {
        Face {
            family: &props.font.family,
            size: props.size.to_f64(),
            weight: props.weight,
            italic: props.italic,
        }
    }
}

fn decoration_decls(props: &ResolvedRunProps, inherited: &Inherited) -> [MaybeDecl; 2] {
    if let Some(underline) = &inherited.underline {
        return [decl("text-decoration-line", underline.clone()), None];
    }
    let mut line = Vec::new();
    if props.underline != Decoration::None {
        line.push("underline");
    }
    if props.strike != Decoration::None {
        line.push("line-through");
    }
    let double = props.underline == Decoration::Double || props.strike == Decoration::Double;
    [
        decl_if(!line.is_empty(), "text-decoration-line", || line.join(" ")),
        decl_if(double, "text-decoration-style", || "double".to_owned()),
    ]
}

/// How a span sizes its share of the line: its own line height, and how much more of its leading sits above its
/// text than below (a ratio of its size). CSS centres an inline box on its text, so the box is raised by half that
/// skew and the text lowered back: the box then reaches the leading the basis puts above the text, and below it
/// the rest. The box is raised by at most half its line height, so it still holds the paragraph's strut, which
/// would otherwise make the line taller; the text is lowered by the whole half skew, so it sits where the basis
/// puts it either way.
#[derive(Clone, Default)]
struct SpanLine {
    line_height: Option<LineHeight>,
    leading_skew: Option<Ratio>,
}

struct SpanLineDecls {
    line_height: MaybeDecl,
    vertical_align: MaybeDecl,
    lift: Vec<MaybeDecl>,
}

/// Half a line height in em of a span drawn at `size` points.
fn half_line_em(line_height: &LineHeight, size: f64) -> Option<f64> {
    match line_height {
        LineHeight::Ratio { value } => Some(value.to_f64() / 2.0),
        LineHeight::Pt { value } if size > 0.0 => Some(value.to_f64() / size / 2.0),
        LineHeight::Pt { .. } => None,
    }
}

/// A span's line declarations, for a span drawn at `size` points.
fn span_line_decls(line: &SpanLine, baseline: f64, size: f64) -> Result<SpanLineDecls> {
    let lift = line.leading_skew.map(|skew| skew.to_f64() / 2.0);
    let half = line
        .line_height
        .as_ref()
        .and_then(|line_height| half_line_em(line_height, size));
    let raise = lift.map(|lift| match half {
        Some(half) => lift.max(-half).min(half),
        None => lift,
    });
    // In em, as the IR states a baseline shift: CSS resolves a vertical-align percentage against the line height.
    let vertical_align = if raise.is_some() || baseline != 0.0 {
        decl(
            "vertical-align",
            format!("{}em", ratio(baseline + raise.unwrap_or(0.0))?),
        )
    } else {
        None
    };
    let line_height = match &line.line_height {
        Some(line_height) => decl("line-height", line_height_value(line_height)?),
        None => None,
    };
    let lift = match lift {
        None => Vec::new(),
        Some(lift) => vec![
            decl("position", "relative"),
            decl("top", format!("{}em", ratio(lift)?)),
        ],
    };
    Ok(SpanLineDecls {
        line_height,
        vertical_align,
        lift,
    })
}

/// What a span inherits: its paragraph's strut face, and its text box's colour and synthesis. A span prints a value
/// only where it differs.
struct Inherited {
    family: Option<String>,
    size: Option<String>,
    weight: u32,
    italic: bool,
    color: Option<String>,
    synthesize_bold: bool,
    /// The underline of a text whose underline a step switches: `var(--sd-underline,…)`, which every run prints.
    underline: Option<String>,
}

/// Face declarations that differ from what the span inherits.
fn face_decls(
    family: &str,
    size: f64,
    scale: f64,
    inherited: &Inherited,
    ctx: &PrintCtx<'_>,
) -> Result<[MaybeDecl; 2]> {
    let stack = font_stack(family, ctx)?;
    let printed = pt(size * scale)?;
    Ok([
        (inherited.family.as_deref() != Some(stack.as_str())).then_some(("font-family", stack)),
        (inherited.size.as_deref() != Some(printed.as_str())).then_some(("font-size", printed)),
    ])
}

fn color_decl(color: String, inherited: &Inherited) -> MaybeDecl {
    (inherited.color.as_deref() != Some(color.as_str())).then_some(("color", color))
}

/// The synthesis of what a paragraph draws besides its runs and fields (its label, its tabs and the spaces hung out
/// of flow): the browser's own, printed where the text box leaves it out for its runs.
fn synthesis_decl(inherited: &Inherited) -> MaybeDecl {
    decl_if(!inherited.synthesize_bold, "font-synthesis-weight", || {
        "auto".to_owned()
    })
}

/// A run's declarations: only what differs from CSS's initial value or from what it inherits.
fn run_style(
    props: &ResolvedRunProps,
    scale: f64,
    inherited: &Inherited,
    ctx: &PrintCtx<'_>,
    line: &SpanLine,
) -> Result<String> {
    let decls = span_line_decls(line, props.baseline.to_f64(), props.size.to_f64() * scale)?;
    let [family, size] = face_decls(
        &props.font.family,
        props.size.to_f64(),
        scale,
        inherited,
        ctx,
    )?;
    let weight = font_weight(&props.font.family, props.weight, ctx);
    let mut out = vec![
        family,
        size,
        decls.line_height,
        decl_if(weight != inherited.weight, "font-weight", || {
            weight.to_string()
        }),
        decl_if(props.italic != inherited.italic, "font-style", || {
            if props.italic { "italic" } else { "normal" }.to_owned()
        }),
        color_decl(css_color(&props.color)?, inherited),
    ];
    out.extend(decoration_decls(props, inherited));
    out.push(match &props.highlight {
        Some(highlight) => decl("background-color", css_color(highlight)?),
        None => None,
    });
    out.push(if props.letter_spacing.steps() != 0 {
        decl("letter-spacing", pt(props.letter_spacing.to_f64())?)
    } else {
        None
    });
    out.push(decls.vertical_align);
    out.push(decl_if(props.caps == Caps::All, "text-transform", || {
        "uppercase".to_owned()
    }));
    out.push(decl_if(
        props.caps == Caps::Small,
        "font-variant-caps",
        || "small-caps".to_owned(),
    ));
    out.push(decl_if(
        props.synthesize_bold != inherited.synthesize_bold,
        "font-synthesis-weight",
        || {
            if props.synthesize_bold {
                "auto"
            } else {
                "none"
            }
            .to_owned()
        },
    ));
    out.extend(decls.lift);
    out.push(shadow_decl(
        props.shadow.as_ref(),
        &ctx.text_axes,
        ShadowForm::Text,
    )?);
    Ok(style(out))
}

fn line_height_value(line_height: &LineHeight) -> Result<String> {
    match line_height {
        LineHeight::Ratio { value } => ratio(value.to_f64()),
        LineHeight::Pt { value } => pt(value.to_f64()),
    }
}

/// The IR's line spacing, printed beside the derived line height so a later lift reads it back.
fn line_spacing_value(spacing: &LineSpacing) -> Result<String> {
    Ok(match spacing {
        LineSpacing::Multiple { value, basis } => {
            format!("multiple:{}:{}", number(value.to_f64())?, basis.as_str())
        }
        LineSpacing::Between { min, max } => format!(
            "between:{}:{}",
            number(min.to_f64())?,
            number(max.to_f64())?
        ),
        LineSpacing::Exact { value } => format!("exact:{}", number(value.to_f64())?),
        LineSpacing::AtLeast { value } => format!("atLeast:{}", number(value.to_f64())?),
        LineSpacing::AtMost { value } => format!("atMost:{}", number(value.to_f64())?),
    })
}

/// The run that sizes the paragraph's own line box: of the smallest runs that draw text, one in the face most of
/// their text uses (the first such), else the paragraph mark. Each line is then as tall as its own largest run, as
/// presentation apps lay lines out; a paragraph sized by its largest run would give that height to every line. The
/// line box takes that run's whole face and own line height: a browser rounds each face's ascent and descent on its
/// own, so a run in another face can grow its line by a pixel, and the majority face keeps that to the fewest lines.
struct Floor<'a> {
    props: &'a ResolvedRunProps,
    line: Option<&'a LineHeight>,
    leading_skew: Option<Ratio>,
}

/// The UTF-16 length of what a run or field draws.
fn drawn_length(inline: &ResolvedInline) -> Option<usize> {
    match inline {
        Inline::Run { text, .. } => Some(utf16::len(text)),
        Inline::Field { cached, .. } => Some(utf16::len(cached)),
        _ => None,
    }
}

fn line_floor<'a>(paragraph: &'a ResolvedParagraph, ctx: &PrintCtx<'_>) -> Option<Floor<'a>> {
    let mut drawn: Vec<(Floor<'a>, usize)> = Vec::new();
    for inline in &paragraph.inlines {
        let (props, line_height, leading_skew) = match inline {
            Inline::Run {
                props,
                line_height,
                leading_skew,
                ..
            }
            | Inline::Field {
                props,
                line_height,
                leading_skew,
                ..
            } => (props, line_height, leading_skew),
            _ => continue,
        };
        let length = drawn_length(inline).unwrap_or(0);
        if matches!(inline, Inline::Run { .. }) && length == 0 {
            continue;
        }
        drawn.push((
            Floor {
                props,
                line: line_height.as_ref(),
                leading_skew: *leading_skew,
            },
            length,
        ));
    }
    let Some(size) = drawn.iter().map(|(run, _)| run.props.size).min() else {
        return paragraph.end_props.as_ref().map(|props| Floor {
            props,
            line: None,
            leading_skew: paragraph.props.leading_skew,
        });
    };
    let face = |props: &ResolvedRunProps| {
        format!(
            "{}/{}/{}",
            font_key(&props.font.family),
            font_weight(&props.font.family, props.weight, ctx),
            if props.italic { "italic" } else { "normal" }
        )
    };
    let smallest: Vec<(Floor<'a>, usize)> = drawn
        .into_iter()
        .filter(|(run, _)| run.props.size == size)
        .collect();
    let mut lengths: IndexMap<String, usize> = IndexMap::new();
    for (run, length) in &smallest {
        *lengths.entry(face(run.props)).or_insert(0) += length;
    }
    let most = lengths.values().copied().max().unwrap_or(0);
    smallest
        .into_iter()
        .find(|(run, _)| lengths.get(&face(run.props)) == Some(&most))
        .map(|(run, _)| run)
}

/// What an inline box sets of its line: its face, size, baseline shift, line height and skew.
fn line_key(
    face: Face<'_>,
    line: &LineHeight,
    leading_skew: Option<Ratio>,
    baseline: f64,
    scale: f64,
    ctx: &PrintCtx<'_>,
) -> Result<String> {
    Ok([
        font_stack(face.family, ctx)?,
        pt(face.size * scale)?,
        font_weight(face.family, face.weight, ctx).to_string(),
        face.italic.to_string(),
        to_js_string(baseline),
        line_height_value(line)?,
        leading_skew.map_or_else(String::new, |skew| to_js_string(skew.to_f64())),
    ]
    .join("|"))
}

/// The props and line of a run or field.
fn run_parts(
    inline: &ResolvedInline,
) -> Option<(&ResolvedRunProps, Option<&LineHeight>, Option<Ratio>)> {
    match inline {
        Inline::Run {
            props,
            line_height,
            leading_skew,
            ..
        }
        | Inline::Field {
            props,
            line_height,
            leading_skew,
            ..
        } => Some((props, line_height.as_ref(), *leading_skew)),
        _ => None,
    }
}

/// The inlines an empty paragraph prints. Its runs draw no text, but an empty span still sets its line like any
/// inline box: a run is left out only where it sets the line as the paragraph mark does.
fn empty_inlines<'a>(
    paragraph: &'a ResolvedParagraph,
    scale: f64,
    ctx: &PrintCtx<'_>,
) -> Result<Vec<&'a ResolvedInline>> {
    let props = &paragraph.props;
    let Some(end) = &paragraph.end_props else {
        return Ok(paragraph.inlines.iter().collect());
    };
    let mark = line_key(
        Face::of(end),
        &props.line_height,
        props.leading_skew,
        0.0,
        scale,
        ctx,
    )?;
    let mut out = Vec::new();
    for inline in &paragraph.inlines {
        let keep = match inline {
            Inline::Run {
                props: run,
                line_height,
                leading_skew,
                ..
            } => {
                line_key(
                    Face::of(run),
                    line_height.as_ref().unwrap_or(&props.line_height),
                    *leading_skew,
                    run.baseline.to_f64(),
                    scale,
                    ctx,
                )? != mark
            }
            _ => true,
        };
        if keep {
            out.push(inline);
        }
    }
    Ok(out)
}

/// Whether everything a paragraph prints in flow takes its floor run's face, size and line, unshifted: its runs and
/// fields, its tabs and the line its mark sizes in the paragraph's own line, and its label. A browser then sizes each
/// line by the paragraph's strut alone, whatever element carries the line.
fn draws_one_line(
    paragraph: &ResolvedParagraph,
    inlines: &[&ResolvedInline],
    floor: &Floor<'_>,
    scale: f64,
    ctx: &PrintCtx<'_>,
) -> Result<bool> {
    let props = &paragraph.props;
    let key = |face: Face<'_>, line: &LineHeight, skew: Option<Ratio>, baseline: f64| {
        line_key(face, line, skew, baseline, scale, ctx)
    };
    let floor_line = floor.line.unwrap_or(&props.line_height);
    let own = key(Face::of(floor.props), floor_line, floor.leading_skew, 0.0)?;
    let strut_line = |face: Face<'_>| key(face, &props.line_height, props.leading_skew, 0.0);
    let mut keys = Vec::with_capacity(inlines.len() + 2);
    for inline in inlines {
        keys.push(match run_parts(inline) {
            Some((run, line_height, skew)) => key(
                Face::of(run),
                line_height.unwrap_or(&props.line_height),
                skew,
                run.baseline.to_f64(),
            )?,
            None if matches!(inline, Inline::Tab {}) => strut_line(Face::of(floor.props))?,
            None => own.clone(),
        });
    }
    let shows_no_text = paragraph.shows_no_text();
    if shows_no_text || matches!(inlines.last(), Some(Inline::Break { .. })) {
        let size = paragraph
            .end_props
            .as_ref()
            .map_or(floor.props.size, |end| end.size);
        keys.push(strut_line(Face {
            size: size.to_f64(),
            ..Face::of(floor.props)
        })?);
    }
    if let ResolvedBullet::Label { font, size, .. } = &props.bullet
        && !shows_no_text
    {
        let face = Face {
            family: &font.family,
            size: size.to_f64(),
            ..Face::of(floor.props)
        };
        keys.push(key(face, floor_line, floor.leading_skew, 0.0)?);
    }
    Ok(keys.iter().all(|each| *each == own) && !pieces_meet(inlines))
}

/// Whether two pieces of text (runs, fields or tabs) meet within a line. A vertical-align on each span shapes it
/// apart from its neighbours, where text in one line box would kern across their edge.
fn pieces_meet(inlines: &[&ResolvedInline]) -> bool {
    let mut open = false;
    for inline in inlines {
        match inline {
            Inline::Break { .. } => open = false,
            Inline::Run { text, .. } if text.is_empty() => {}
            _ => {
                if open {
                    return true;
                }
                open = true;
            }
        }
    }
    false
}

/// The line declarations alone of a span drawn at `size` points.
fn line_decls(line: &SpanLine, size: f64) -> Result<Vec<MaybeDecl>> {
    let decls = span_line_decls(line, 0.0, size)?;
    let mut out = vec![decls.line_height, decls.vertical_align];
    out.extend(decls.lift);
    Ok(out)
}

/// The empty line a paragraph mark or a break draws, sized by its properties (`mark`): a span only where they differ
/// from the strut's.
fn mark_line(
    mark: Option<&ResolvedRunProps>,
    scale: f64,
    inherited: &Inherited,
    line: &SpanLine,
    strut_size: f64,
) -> Result<Element> {
    let drawn = mark.map_or(strut_size, |end| end.size.to_f64() * scale);
    let size = match mark {
        Some(end) => Some(pt(end.size.to_f64() * scale)?),
        None => None,
    };
    let mut decls = vec![
        size.filter(|size| inherited.size.as_ref() != Some(size))
            .map(|size| ("font-size", size)),
    ];
    decls.extend(line_decls(line, drawn)?);
    let decls = style(decls);
    let br = Element::empty("br");
    Ok(if decls.is_empty() {
        br
    } else {
        Element::new("span", Attrs::new().val("style", decls), vec![br.into()])
    })
}

/// The spaces that end a line before a break or the paragraph's end, by inline index. Presentation apps leave them
/// out when they align a line, but CSS hangs a line's final spaces only where they overflow, so a centred line
/// ending in a space would sit half a space left. Each is printed out of flow after its run ([`hanging_span`]).
fn hanging_spaces(inlines: &[&ResolvedInline]) -> IndexMap<usize, String> {
    let mut hanging = IndexMap::new();
    // Backwards, a line at a time; a line of nothing but spaces keeps them, as they are all it holds.
    let mut line: Vec<(usize, String)> = Vec::new();
    let mut trailing = true;
    let mut drawn = false;
    let mut close = |line: &mut Vec<(usize, String)>, trailing: &mut bool, drawn: &mut bool| {
        if *drawn {
            for (index, spaces) in line.drain(..) {
                hanging.insert(index, spaces);
            }
        }
        line.clear();
        *trailing = true;
        *drawn = false;
    };
    for (index, inline) in inlines.iter().enumerate().rev() {
        if matches!(inline, Inline::Break { .. }) {
            close(&mut line, &mut trailing, &mut drawn);
            continue;
        }
        if let (Inline::Run { text, .. }, true) = (inline, trailing) {
            let kept = text.trim_end_matches(' ').len();
            let spaces = text.get(kept..).unwrap_or("");
            if !spaces.is_empty() {
                line.push((index, spaces.to_owned()));
            }
            if kept == 0 {
                continue;
            }
        }
        trailing = false;
        drawn = true;
    }
    close(&mut line, &mut trailing, &mut drawn);
    hanging
}

/// Spaces that take no room in their line: out of flow where they would be, keeping a highlight or a decoration.
fn hanging_span(
    spaces: &str,
    props: &ResolvedRunProps,
    scale: f64,
    inherited: &Inherited,
    ctx: &PrintCtx<'_>,
) -> Result<Element> {
    let [family, size] = face_decls(
        &props.font.family,
        props.size.to_f64(),
        scale,
        inherited,
        ctx,
    )?;
    let mut decls = vec![
        decl("position", "absolute"),
        family,
        size,
        color_decl(css_color(&props.color)?, inherited),
    ];
    decls.extend(decoration_decls(props, inherited));
    decls.push(match &props.highlight {
        Some(highlight) => decl("background-color", css_color(highlight)?),
        None => None,
    });
    decls.push(synthesis_decl(inherited));
    Ok(Element::new(
        "span",
        Attrs::new().val("style", style(decls)),
        vec![text(spaces)],
    ))
}

/// A length in points, left out where it is 0 (the initial value of every property printed with it).
fn pt_decl(property: &'static str, value: f64) -> Result<MaybeDecl> {
    let printed = pt(value)?;
    Ok((printed != "0pt").then_some((property, printed)))
}

/// A paragraph's space before and after it; the text's height ends where its last line's text does.
fn paragraph_margins(paragraph: &ResolvedParagraph, last: bool) -> (f64, f64) {
    let props = &paragraph.props;
    let tail = if last {
        props.line_tail_pt.map_or(0.0, |tail| tail.to_f64())
    } else {
        0.0
    };
    (
        props.space_before_pt.to_f64(),
        props.space_after_pt.to_f64() - tail,
    )
}

fn holds_tab(paragraph: &ResolvedParagraph) -> bool {
    paragraph.inlines.iter().any(|inline| match inline {
        Inline::Tab {} => true,
        Inline::Run { text, .. } => text.contains('\t'),
        Inline::Field { cached, .. } => cached.contains('\t'),
        Inline::Break { .. } => false,
    })
}

fn align_name(align: Align) -> &'static str {
    match align {
        Align::Left => "left",
        Align::Center => "center",
        Align::Right => "right",
        Align::Justify => "justify",
    }
}

/// A paragraph to print: its place in the body and what its box sets for all its text.
struct ParagraphCtx<'b> {
    index: usize,
    tag: &'static str,
    scale: f64,
    last: bool,
    shared: &'b BoxText,
    node: &'b NodeId,
}

fn print_paragraph(
    paragraph: &ResolvedParagraph,
    p: &ParagraphCtx<'_>,
    ctx: &mut PrintCtx<'_>,
) -> Result<Element> {
    let props = &paragraph.props;
    let scale = p.scale;
    let floor = line_floor(paragraph, ctx);
    let strut = floor.as_ref().map(|floor| floor.props);
    if let Some(strut) = strut {
        ctx.drew(&strut.font.family, strut.weight, strut.italic);
    }
    let shows_no_text = paragraph.shows_no_text();
    let inlines: Vec<&ResolvedInline> = if shows_no_text {
        empty_inlines(paragraph, scale, ctx)?
    } else {
        paragraph.inlines.iter().collect()
    };
    // The paragraph's line box takes its floor run's own line. Where that run's leading is skewed and every line
    // draws in its face and line alone, the paragraph is lifted by half the skew as a whole. Elsewhere every line
    // takes its height from spans that carry the skew instead: a centred strut rounds its ascent and descent apart
    // from theirs, and would make each line a pixel taller. A run whose own line differs, or that carries a skew,
    // prints its line.
    let skewed = floor
        .as_ref()
        .is_some_and(|floor| floor.leading_skew.is_some());
    let lifted = match &floor {
        Some(floor) if skewed => draws_one_line(paragraph, &inlines, floor, scale, ctx)?,
        _ => false,
    };
    let skewed_spans = skewed && !lifted;
    let paragraph_line = if skewed_spans {
        LineHeight::Ratio { value: Ratio::ZERO }
    } else {
        floor
            .as_ref()
            .and_then(|floor| floor.line)
            .unwrap_or(&props.line_height)
            .clone()
    };
    let paragraph_line_value = line_height_value(&paragraph_line)?;
    let run_line = |own: Option<&LineHeight>, leading_skew: Option<Ratio>| -> Result<SpanLine> {
        let line = own.unwrap_or(&props.line_height);
        if leading_skew.is_some() && !lifted {
            return Ok(SpanLine {
                line_height: Some(line.clone()),
                leading_skew,
            });
        }
        Ok(if line_height_value(line)? == paragraph_line_value {
            SpanLine::default()
        } else {
            SpanLine {
                line_height: Some(line.clone()),
                leading_skew: None,
            }
        })
    };
    let strut_line = || run_line(None, props.leading_skew);
    let strut_size = strut.map_or(0.0, |strut| strut.size.to_f64() * scale);
    let inherited = Inherited {
        family: strut.map(|s| font_stack(&s.font.family, ctx)).transpose()?,
        size: strut.map(|s| pt(s.size.to_f64() * scale)).transpose()?,
        weight: strut.map_or(400, |s| font_weight(&s.font.family, s.weight, ctx)),
        italic: strut.is_some_and(|s| s.italic),
        color: p.shared.color.clone(),
        synthesize_bold: p.shared.synthesize_bold,
        underline: ctx
            .hook(p.node.as_str(), Hook::Underline)
            .map(|value| Hook::Underline.var(value)),
    };
    let bold = ctx
        .hook(p.node.as_str(), Hook::Bold)
        .map(|value| Hook::Bold.var(value));
    let italic = ctx
        .hook(p.node.as_str(), Hook::Italic)
        .map(|value| Hook::Italic.var(value));
    let inline_style = |props: &ResolvedRunProps,
                        line_height: Option<&LineHeight>,
                        leading_skew: Option<Ratio>,
                        ctx: &PrintCtx<'_>|
     -> Result<String> {
        run_style(
            props,
            scale,
            &inherited,
            ctx,
            &run_line(line_height, leading_skew)?,
        )
    };
    let indent = pt_decl("text-indent", props.indent.to_f64())?;
    let (margin_top, margin_bottom) = paragraph_margins(paragraph, p.last);
    let tab_size = if holds_tab(paragraph) {
        Some(pt(props.tab_size.to_f64())?)
    } else {
        None
    };
    // A browser may align an item from its list's start side (`text-align:match-parent`) rather than its own, and
    // the list runs left to right: a right-to-left item prints its alignment whatever it is.
    let start_side = if props.rtl { Align::Right } else { Align::Left };
    let aligned = props.align != start_side || (props.rtl && p.tag == "li");
    let mut children: Vec<Node> = Vec::new();

    // Presentation apps show a bullet only on a paragraph with text.
    if let ResolvedBullet::Label {
        text: label,
        font,
        size,
        color,
        ..
    } = &props.bullet
        && !shows_no_text
    {
        ctx.drew(
            &font.family,
            strut.map_or(400, |strut| strut.weight),
            strut.is_some_and(|strut| strut.italic),
        );
        let [family, label_size] = face_decls(&font.family, size.to_f64(), scale, &inherited, ctx)?;
        let decls = vec![
            decl("display", "inline-block"),
            pt_decl("min-width", f64::max(0.0, -props.indent.to_f64()))?,
            indent.as_ref().map(|_| ("text-indent", "0pt".to_owned())),
            family,
            label_size,
            color_decl(css_color(color)?, &inherited),
            synthesis_decl(&inherited),
        ];
        children.push(
            Element::new(
                "span",
                Attrs::new()
                    .flag(BULLET, true)
                    .val("aria-hidden", "true".to_owned())
                    .val("style", style(decls)),
                vec![text(label.clone())],
            )
            .into(),
        );
    }

    let hanging = hanging_spaces(&inlines);
    let split = inlines
        .iter()
        .any(|inline| matches!(inline, Inline::Break { .. }));
    // The index in `children` of the last run printed as bare text.
    let mut bare: Option<usize> = None;
    // The line under way draws text: where it draws none, a break ends it as an empty paragraph's line, which the
    // spans' lines would otherwise leave no height.
    let mut drawn = false;
    for (index, inline) in inlines.iter().enumerate() {
        match inline {
            Inline::Run {
                text: run_text,
                props: run,
                line_height,
                leading_skew,
            } => {
                let spaces = hanging.get(&index).map_or("", String::as_str);
                let shown = run_text
                    .get(..run_text.len() - spaces.len())
                    .unwrap_or(run_text.as_str());
                drawn |= !shown.is_empty();
                let decls = inline_style(run, line_height.as_ref(), *leading_skew, ctx)?;
                // A run prints when it holds text or its span sets something: even empty, its box takes part in
                // the line.
                if !run_text.is_empty() || !decls.is_empty() || run.link.is_some() {
                    ctx.drew(&run.font.family, run.weight, run.italic);
                }
                let span = |decls: String| {
                    Element::new(
                        "span",
                        Attrs::new().val("style", (!decls.is_empty()).then_some(decls)),
                        vec![text(shown)],
                    )
                };
                // A run in its paragraph's style is its text alone, or nothing when it shows none. Text shapes apart
                // at a run's edge, so a run right after such a text keeps its span. Text edits in place as the one
                // text of its element, which a paragraph a break splits is not: each of its runs keeps its span, to
                // edit on its own.
                if !decls.is_empty() || run.link.is_some() {
                    let span = linked(run.link.as_ref(), span(decls), ctx, p.node)?;
                    children.push(span.into());
                } else if !shown.is_empty() {
                    let after_text = bare.is_some_and(|at| at + 1 == children.len());
                    if after_text || split {
                        bare = None;
                        children.push(span(decls).into());
                    } else {
                        bare = Some(children.len());
                        children.push(text(shown));
                    }
                }
                if !spaces.is_empty() {
                    children.push(hanging_span(spaces, run, scale, &inherited, ctx)?.into());
                }
            }
            Inline::Field {
                field,
                cached,
                props: run,
                line_height,
                leading_skew,
            } => {
                drawn = true;
                ctx.drew(&run.font.family, run.weight, run.italic);
                let decls = inline_style(run, line_height.as_ref(), *leading_skew, ctx)?;
                let span = Element::new(
                    "span",
                    Attrs::new()
                        .val(attr::FIELD, field.as_str().to_owned())
                        .val("style", (!decls.is_empty()).then_some(decls)),
                    vec![text(cached.clone())],
                );
                children.push(linked(run.link.as_ref(), span, ctx, p.node)?.into());
            }
            // Where the lines take their height from spans, an empty line is sized as the paragraph mark sizes an
            // empty paragraph's.
            Inline::Break { props: mark } => {
                children.push(if skewed_spans && !drawn {
                    mark_line(mark.as_ref(), scale, &inherited, &strut_line()?, strut_size)?.into()
                } else {
                    Element::empty("br").into()
                });
                drawn = false;
            }
            Inline::Tab {} => {
                drawn = true;
                let mut decls = if skewed_spans {
                    line_decls(&strut_line()?, strut_size)?
                } else {
                    Vec::new()
                };
                decls.push(synthesis_decl(&inherited));
                let decls = style(decls);
                children.push(if decls.is_empty() {
                    text("\t")
                } else {
                    Element::new("span", Attrs::new().val("style", decls), vec![text("\t")]).into()
                });
            }
        }
    }
    // An empty paragraph keeps its line, and so does the line a final break opens: the paragraph mark sizes them.
    if shows_no_text {
        children.push(
            mark_line(
                paragraph.end_props.as_ref(),
                scale,
                &inherited,
                &strut_line()?,
                strut_size,
            )?
            .into(),
        );
    } else if matches!(inlines.last(), Some(Inline::Break { .. })) {
        let line = if skewed_spans {
            strut_line()?
        } else {
            SpanLine::default()
        };
        children.push(
            mark_line(
                paragraph.end_props.as_ref(),
                scale,
                &inherited,
                &line,
                strut_size,
            )?
            .into(),
        );
    }

    let mut decls = vec![
        decl_if(aligned, "text-align", || align_name(props.align).to_owned()),
        pt_decl("padding-left", props.margin_left.to_f64())?,
        pt_decl("padding-right", props.margin_right.to_f64())?,
        indent,
        pt_decl("margin-top", margin_top)?,
        pt_decl("margin-bottom", margin_bottom)?,
        decl("line-height", paragraph_line_value),
        inherited.size.clone().map(|size| ("font-size", size)),
        inherited
            .family
            .clone()
            .map(|family| ("font-family", family)),
        match bold {
            Some(bold) => decl("font-weight", bold),
            None => decl_if(inherited.weight != 400, "font-weight", || {
                inherited.weight.to_string()
            }),
        },
        match italic {
            Some(italic) => decl("font-style", italic),
            None => decl_if(inherited.italic, "font-style", || "italic".to_owned()),
        },
        tab_size
            .filter(|size| p.shared.tab_size.as_ref() != Some(size))
            .map(|size| ("tab-size", size)),
        decl_if(props.rtl, "direction", || "rtl".to_owned()),
    ];
    if let (true, Some(floor)) = (lifted, &floor) {
        let line = SpanLine {
            line_height: None,
            leading_skew: floor.leading_skew,
        };
        decls.extend(span_line_decls(&line, 0.0, strut_size)?.lift);
    }
    Ok(Element::new(
        p.tag,
        Attrs::new()
            .val(PARAGRAPH, p.index.to_string())
            .val(
                "data-line-spacing",
                line_spacing_value(&props.line_spacing)?,
            )
            .val("style", style(decls)),
        children,
    ))
}

/// What a text box sets for all its text: the colour most of its text draws in, a synthesis no run of it takes, and
/// the tab size most of its paragraphs with a tab use.
struct BoxText {
    color: Option<String>,
    synthesize_bold: bool,
    tab_size: Option<String>,
}

/// The key with the largest count, the first of equals.
fn most(counts: &IndexMap<String, usize>) -> Option<String> {
    let mut best: Option<(&String, usize)> = None;
    for (key, count) in counts {
        if best.is_none_or(|(_, top)| *count > top) {
            best = Some((key, *count));
        }
    }
    best.map(|(key, _)| key.clone())
}

fn box_text(body: &ResolvedTextBody) -> Result<BoxText> {
    let mut colors: IndexMap<String, usize> = IndexMap::new();
    let mut tab_sizes: IndexMap<String, usize> = IndexMap::new();
    // Synthesised unless every drawn run leaves it out.
    let mut synthesize_bold: Option<bool> = None;
    for paragraph in &body.paragraphs {
        if holds_tab(paragraph) {
            *tab_sizes
                .entry(pt(paragraph.props.tab_size.to_f64())?)
                .or_insert(0) += 1;
        }
        for inline in &paragraph.inlines {
            let (Some((props, _, _)), Some(length)) = (run_parts(inline), drawn_length(inline))
            else {
                continue;
            };
            if length == 0 {
                continue;
            }
            *colors.entry(css_color(&props.color)?).or_insert(0) += length;
            synthesize_bold = Some(synthesize_bold.unwrap_or(false) || props.synthesize_bold);
        }
    }
    Ok(BoxText {
        color: most(&colors),
        synthesize_bold: synthesize_bold.unwrap_or(true),
        tab_size: most(&tab_sizes),
    })
}

/// A paragraph that shows a label, an item of a list.
fn labelled(paragraph: &ResolvedParagraph) -> bool {
    matches!(paragraph.props.bullet, ResolvedBullet::Label { .. }) && !paragraph.shows_no_text()
}

/// A paragraph that shows text with no label, which ends a list.
fn plain(paragraph: &ResolvedParagraph) -> bool {
    !matches!(paragraph.props.bullet, ResolvedBullet::Label { .. }) && !paragraph.shows_no_text()
}

/// A body's paragraphs in order, as lists and single paragraphs: a list is each longest run of labelled paragraphs,
/// with the empty paragraphs between them. Each entry is (list, from, to).
fn list_runs(paragraphs: &[ResolvedParagraph]) -> Vec<(bool, usize, usize)> {
    let mut runs = Vec::new();
    let mut from = 0;
    while let Some(first) = paragraphs.get(from) {
        let list = labelled(first);
        let mut to = from + 1;
        let mut next = to;
        while list && let Some(paragraph) = paragraphs.get(next) {
            if plain(paragraph) {
                break;
            }
            if labelled(paragraph) {
                to = next + 1;
            }
            next += 1;
        }
        runs.push((list, from, to));
        from = to;
    }
    runs
}

fn roman_digit(digit: char) -> f64 {
    match digit {
        'i' => 1.0,
        'v' => 5.0,
        'x' => 10.0,
        'l' => 50.0,
        'c' => 100.0,
        'd' => 500.0,
        'm' => 1000.0,
        _ => 0.0,
    }
}

/// The number a numbered label shows, read in its scheme: `3.` is 3, `(BB)` 28, `iv)` 4, and an arabic one as
/// `Number` reads it, so `0x1f.` is 31.
fn label_number(label: &str, scheme: AutoNumScheme) -> f64 {
    let digits: String = label
        .chars()
        .filter(|c| !matches!(c, '(' | ')' | '.'))
        .collect::<String>()
        .to_lowercase();
    use AutoNumScheme as S;
    let arabic = matches!(
        scheme,
        S::ArabicPeriod | S::ArabicParenR | S::ArabicParenBoth | S::ArabicPlain
    );
    let alpha = matches!(
        scheme,
        S::AlphaLcPeriod
            | S::AlphaUcPeriod
            | S::AlphaLcParenR
            | S::AlphaUcParenR
            | S::AlphaLcParenBoth
            | S::AlphaUcParenBoth
    );
    if arabic {
        return js_number(&digits);
    }
    if alpha {
        let Some(first) = digits.encode_utf16().next() else {
            return f64::NAN;
        };
        return f64::from(first) - 96.0 + 26.0 * (utf16::len(&digits) as f64 - 1.0);
    }
    let values: Vec<f64> = digits.chars().map(roman_digit).collect();
    let mut sum = 0.0;
    for (i, value) in values.iter().enumerate() {
        let next = values.get(i + 1).copied().unwrap_or(0.0);
        sum += if *value < next { -value } else { *value };
    }
    sum
}

/// A list's tag, from its first item at its shallowest level: `<ol>` where that item's label is a number, with the
/// number as `start` where it is not 1, and `<ul>` where it is a character. Deeper items keep their own labels.
fn list_head(items: &[ResolvedParagraph]) -> (&'static str, Option<String>) {
    let shown: Vec<&ResolvedParagraph> = items.iter().filter(|item| labelled(item)).collect();
    let Some(level) = shown.iter().map(|item| item.level).min() else {
        return ("ul", None);
    };
    let bullet = shown
        .iter()
        .find(|item| item.level == level)
        .map(|item| &item.props.bullet);
    let Some(ResolvedBullet::Label {
        text,
        auto: Some(auto),
        ..
    }) = bullet
    else {
        return ("ul", None);
    };
    let start = label_number(text, auto.scheme);
    ("ol", (start != 1.0).then(|| to_js_string(start)))
}

/// A list adds no space and no marker, as its items keep their labels as text. It lays them out as the text box
/// lays out paragraphs, in a flex column: the newlines between them take no line, and their margins do not
/// collapse.
const LIST_STYLE: &str = "margin:0;padding:0;list-style:none;display:flex;flex-direction:column";

/// The text's vertical alignment, left out at the top, where a flex column starts; `safe` keeps text taller than
/// its box from spreading above it.
fn justify(body: &ResolvedBodyProps) -> MaybeDecl {
    let value = match body.anchor {
        Anchor::Top => return None,
        Anchor::Middle => "center",
        Anchor::Bottom => "flex-end",
    };
    decl(
        "justify-content",
        if body.overflow == Overflow::Spread {
            value.to_owned()
        } else {
            format!("safe {value}")
        },
    )
}

/// A text body as the box that holds it: its markers, its insets, the flex column that lays out its paragraphs and
/// lists (so the newlines between them take no line), what its text inherits, and the turn to its page orientation.
pub struct TextBox {
    pub attributes: Attrs,
    pub padding: MaybeDecl,
    pub column: Vec<MaybeDecl>,
    pub inherited: Vec<MaybeDecl>,
    pub turn: MaybeDecl,
    pub blocks: Vec<Element>,
    /// Whether two adjacent blocks each have a margin where they meet, which block flow would collapse into one.
    pub collapses: bool,
}

/// A text body's blocks (paragraphs and lists), and whether two adjacent blocks each have a margin where they meet.
fn text_blocks(
    body: &ResolvedTextBody,
    scale: f64,
    shared: &BoxText,
    ctx: &mut PrintCtx<'_>,
    node: &NodeId,
) -> Result<(Vec<Element>, bool)> {
    let paragraphs = &body.paragraphs;
    let count = paragraphs.len();
    let runs = list_runs(paragraphs);
    let mut blocks = Vec::with_capacity(runs.len());
    let mut margins = Vec::with_capacity(runs.len());
    for &(list, from, to) in &runs {
        let print = |index: usize, tag: &'static str, ctx: &mut PrintCtx<'_>| -> Result<Element> {
            let Some(paragraph) = paragraphs.get(index) else {
                return Ok(Element::empty(tag));
            };
            let p = ParagraphCtx {
                index,
                tag,
                scale,
                last: index + 1 == count,
                shared,
                node,
            };
            print_paragraph(paragraph, &p, ctx)
        };
        if !list {
            blocks.push(print(from, "p", ctx)?);
            if let Some(paragraph) = paragraphs.get(from) {
                margins.push(paragraph_margins(paragraph, from + 1 == count));
            }
            continue;
        }
        let (tag, start) = list_head(paragraphs.get(from..to).unwrap_or(&[]));
        let items = (from..to)
            .map(|index| print(index, "li", ctx))
            .collect::<Result<Vec<_>>>()?;
        blocks.push(Element::new(
            tag,
            Attrs::new()
                .val("start", start)
                .val("style", LIST_STYLE.to_owned()),
            lines(items),
        ));
        // A list's own margins are 0, and its items' stay inside its flex column.
        margins.push((0.0, 0.0));
    }
    let printed = |value: f64| -> Result<bool> { Ok(pt(value)? != "0pt") };
    let mut collapses = false;
    for pair in margins.windows(2) {
        if let [(_, bottom), (top, _)] = pair
            && printed(*bottom)?
            && printed(*top)?
        {
            collapses = true;
        }
    }
    Ok((blocks, collapses))
}

fn text_box(
    body: &ResolvedTextBody,
    frame: &Frame,
    padding: MaybeDecl,
    alignment: MaybeDecl,
    ctx: &mut PrintCtx<'_>,
    node: &NodeId,
) -> Result<TextBox> {
    let (scale, autofit_kind, font_scale, fitted) = match &body.body.autofit {
        ResolvedAutofit::Shrink {
            font_scale, fitted, ..
        } => (
            font_scale.to_f64(),
            Some("shrink"),
            Some(*font_scale),
            Some(*fitted),
        ),
        ResolvedAutofit::Resize { .. } => (1.0, Some("resize"), None, None),
        ResolvedAutofit::None {} => (1.0, None, None, None),
    };
    let shared = box_text(body)?;
    // The runs' shadows draw in the text box's axes: the element's turn, then the text's own.
    let (turn_x, turn_y) = text_turn(frame, body);
    let text_axes = ctx.axes.then_frame(frame).then(0.0, turn_x < 0, turn_y < 0);
    let outer_axes = std::mem::replace(&mut ctx.text_axes, text_axes);
    let printed = text_blocks(body, scale, &shared, ctx, node);
    ctx.text_axes = outer_axes;
    let (blocks, collapses) = printed?;
    let attributes = Attrs::new()
        .flag(TEXT, true)
        .val("data-autofit", autofit_kind.map(str::to_owned))
        .val(
            "data-font-scale",
            font_scale.map(|scale| ratio(scale.to_f64())).transpose()?,
        )
        .val("data-fitted", fitted.map(|fitted| fitted.to_string()));
    Ok(TextBox {
        attributes,
        padding,
        column: vec![
            decl("display", "flex"),
            decl("flex-direction", "column"),
            alignment,
        ],
        inherited: vec![
            decl(
                "white-space",
                if body.body.wrap == Wrap::None {
                    "pre"
                } else {
                    "pre-wrap"
                },
            ),
            decl("overflow-wrap", "break-word"),
            shared.color.clone().map(|color| {
                (
                    "color",
                    match ctx.hook(node.as_str(), Hook::Text) {
                        Some(value) => Hook::Text.var(value),
                        None => color,
                    },
                )
            }),
            decl_if(!shared.synthesize_bold, "font-synthesis-weight", || {
                "none".to_owned()
            }),
            shared.tab_size.clone().map(|size| ("tab-size", size)),
        ],
        turn: text_turn_decl(frame, body),
        blocks,
        collapses,
    })
}

/// A text box as an element of its own, placed by `placement` in the element that holds it.
pub fn text_element(text_box: TextBox, placement: Vec<MaybeDecl>) -> Element {
    let mut decls = placement;
    decls.push(text_box.padding);
    decls.extend(text_box.column);
    decls.extend(text_box.inherited);
    decls.push(text_box.turn);
    Element::new(
        "div",
        text_box.attributes.val("style", style(decls)),
        lines(text_box.blocks),
    )
}

/// A shape's text box, and how the shape's own box grows when the text resizes it.
pub struct ShapeText {
    pub text_box: TextBox,
    pub autosize: Option<Autosize>,
}

pub fn print_text(
    body: &ResolvedTextBody,
    frame: &Frame,
    ctx: &mut PrintCtx<'_>,
    node: &NodeId,
) -> Result<ShapeText> {
    let props = &body.body;
    let autosize = match &props.autofit {
        ResolvedAutofit::Resize {
            width,
            height,
            anchor_x,
        } if *width || *height => Some(Autosize {
            width: *width,
            height: *height,
            anchor_x: *anchor_x,
            anchor_y: props.anchor,
            max_width: props.max_width,
        }),
        _ => None,
    };
    let insets = &props.insets;
    let padding = sides(
        insets.t.to_f64(),
        insets.r.to_f64(),
        insets.b.to_f64(),
        insets.l.to_f64(),
    )?;
    // Autosized text lays out from the top of its shape, so it takes no vertical alignment.
    let alignment = if autosize.is_none() {
        justify(props)
    } else {
        None
    };
    let padding = (padding != "0pt").then_some(("padding", padding));
    let text_box = text_box(body, frame, padding, alignment, ctx, node)?;
    Ok(ShapeText { text_box, autosize })
}

/// Where a shape's own text box sits: over the whole shape, or in flow, where an autosized shape grows with it.
pub fn text_placement(autosize: Option<&Autosize>) -> Vec<MaybeDecl> {
    match autosize {
        Some(_) => vec![decl("position", "relative")],
        None => vec![decl("position", "absolute"), decl("inset", "0")],
    }
}

/// A table cell's text, in flow: the cell holds its insets as padding and its anchor as vertical alignment, and its
/// row grows to fit it.
pub fn print_cell_text(
    body: &ResolvedTextBody,
    frame: &Frame,
    ctx: &mut PrintCtx<'_>,
    node: &NodeId,
) -> Result<TextBox> {
    text_box(body, frame, None, None, ctx, node)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_label_numbers_in_their_scheme() {
        assert_eq!(label_number("3.", AutoNumScheme::ArabicPeriod), 3.0);
        assert_eq!(label_number("12)", AutoNumScheme::ArabicParenR), 12.0);
        assert_eq!(label_number("(BB)", AutoNumScheme::AlphaUcParenBoth), 28.0);
        assert_eq!(label_number("a.", AutoNumScheme::AlphaLcPeriod), 1.0);
        assert_eq!(label_number("iv)", AutoNumScheme::RomanLcParenR), 4.0);
        assert_eq!(label_number("XIV.", AutoNumScheme::RomanUcPeriod), 14.0);
        assert!(label_number("x1", AutoNumScheme::ArabicPlain).is_nan());
        assert_eq!(label_number("0X1F.", AutoNumScheme::ArabicPeriod), 31.0);
        assert_eq!(label_number("(0b101)", AutoNumScheme::ArabicParenBoth), 5.0);
        assert_eq!(label_number("", AutoNumScheme::ArabicPlain), 0.0);
    }

    #[test]
    fn accepts_only_http_https_and_mailto_links() {
        assert!(is_safe_url("https://example.com"));
        assert!(is_safe_url("HTTP://x"));
        assert!(is_safe_url("mailto:a@b"));
        assert!(!is_safe_url("javascript:alert(1)"));
        assert!(!is_safe_url("file:///etc/passwd"));
        assert!(!is_safe_url("http"));
    }

    #[test]
    fn picks_the_first_of_equal_counts() {
        let mut counts = IndexMap::new();
        counts.insert("a".to_owned(), 2);
        counts.insert("b".to_owned(), 3);
        counts.insert("c".to_owned(), 3);
        assert_eq!(most(&counts).as_deref(), Some("b"));
        assert_eq!(most(&IndexMap::new()), None);
    }
}
