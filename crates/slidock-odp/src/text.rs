//! ODF text: a shape's paragraphs (`text:p`, `text:h`) and lists, their spans, spaces, tabs, line breaks, links and
//! fields. Properties resolve through the style chains, in the order LibreOffice applies them: the shape's style (its
//! paragraph and text properties; deeper levels of an outline from the master page's `-outline<n>` styles), then the
//! shape's `draw:text-style-name`, then the paragraph's style, then each enclosing span's. The shape's part becomes the
//! body's list style, complete per level; paragraphs and runs carry what their own styles declare, and list paragraphs
//! their bullet and indents. White space collapses as ODF says: runs of spaces, tabs and newlines in the text are one
//! space, ignored at a paragraph's start; `text:s` writes the spaces it counts.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use slidock_canon::{js_number, utf16};
use slidock_core_api::lowering::LowerError;
use slidock_ir::color::Color;
use slidock_ir::diagnostics::{DiagnosticCode, DiagnosticSite, codes};
use slidock_ir::features::FeatureId;
use slidock_ir::ids::NodeId;
use slidock_ir::shadow::SourceShadow;
use slidock_ir::text::{
    Align, Anchor, AnchorX, Autofit, Bullet, Caps, Decoration, FamilyFont, FieldKind, FontRef,
    Inline, LEVELS, LevelStyle, LevelStyles, LineSpacing, Link, MAX_LEVEL, MAX_STRING,
    PartialBodyProps, PartialInsets, PartialParaProps, PartialRunProps, ShrinkSteps, SourceInline,
    SourceParagraph, Spacing, TextBody, Wrap,
};

use crate::context::{Lowering, PageDeclarations};
use crate::defaults::LINE_BASIS;
use crate::links::read_href;
use crate::lists::list_level;
use crate::names::{QName, qn};
use crate::odf::{Element, Ns, XmlNode};
use crate::styles::{CommonStyles, Family, PropertySet, Props, get, unquote};
use crate::units::{
    hex_color, is_space, length_pt, percent, pt, ratio_of, rgb, size, size_pt, to_pt, unit_ratio,
};
use slidock_math::{Grid, quantise};

/// How a reader reports: a code with its site, on the element being read.
pub(crate) type Report<'r> = &'r dyn Fn(DiagnosticCode, DiagnosticSite);

/// A report that is dropped.
pub(crate) fn silent(_: DiagnosticCode, _: DiagnosticSite) {}

/// How a text body on a drawable reads: reports land on it, fields read the page's declarations. Speaker notes read
/// with no node, no declarations, and record no feature.
#[derive(Clone)]
pub(crate) struct TextContext<'a, 'd> {
    pub ctx: Lowering<'a, 'd>,
    node: Option<NodeId>,
    track: bool,
    pub declarations: Option<&'d PageDeclarations>,
}

impl<'a, 'd> TextContext<'a, 'd> {
    /// The text of a drawable.
    pub(crate) fn on(ctx: Lowering<'a, 'd>, node: &NodeId) -> Self {
        TextContext {
            declarations: ctx.declarations,
            ctx,
            node: Some(node.clone()),
            track: true,
        }
    }

    /// The text of speaker notes.
    pub(crate) fn notes(ctx: Lowering<'a, 'd>) -> Self {
        TextContext {
            ctx,
            node: None,
            track: false,
            declarations: None,
        }
    }

    pub(crate) fn report(&self, code: DiagnosticCode, site: DiagnosticSite) {
        if self.track
            && let Some(feature) = site.feature
        {
            self.ctx.see(feature);
        }
        let site = match &self.node {
            Some(node) => site.node(node.clone()),
            None => site,
        };
        self.ctx.report(code, site);
    }

    pub(crate) fn seen(&self, feature: FeatureId) {
        if self.track {
            self.ctx.see(feature);
        }
    }
}

fn align_of(value: Option<&str>) -> Option<Align> {
    match value? {
        "start" | "left" => Some(Align::Left),
        "end" | "right" => Some(Align::Right),
        "center" => Some(Align::Center),
        "justify" => Some(Align::Justify),
        _ => None,
    }
}

/// A paragraph-properties set as IR paragraph properties: every value it states.
pub(crate) fn para_props(set: &PropertySet, report: Report<'_>) -> PartialParaProps {
    let mut out = PartialParaProps {
        align: align_of(get(set, qn!("fo:text-align"))),
        ..PartialParaProps::default()
    };
    // A side the set leaves out takes the `fo:margin` shorthand.
    let length = |name: QName| {
        let written = get(set, name).or_else(|| {
            name.as_str()
                .starts_with("fo:margin-")
                .then(|| get(set, qn!("fo:margin")))
                .flatten()
        });
        length_pt(written)
    };
    out.margin_left = length(qn!("fo:margin-left")).map(pt);
    out.margin_right = length(qn!("fo:margin-right")).map(pt);
    out.indent = length(qn!("fo:text-indent")).map(pt);
    out.space_before = length(qn!("fo:margin-top")).map(|value| Spacing::Pt { value: pt(value) });
    out.space_after = length(qn!("fo:margin-bottom")).map(|value| Spacing::Pt { value: pt(value) });
    out.line_spacing = line_spacing(set);
    if let Some(leading) = get(set, qn!("style:line-spacing"))
        && leading != "0cm"
    {
        report(
            codes::OD9201,
            DiagnosticSite::new()
                .feature(FeatureId::TextLineSpacing)
                .data("leading", utf16::clip(leading, 32).to_owned()),
        );
    }
    if let Some(tab) = size_pt(get(set, qn!("style:tab-stop-distance")))
        && tab > 0.0
    {
        out.tab_size = Some(size(tab));
    }
    match get(set, qn!("style:writing-mode")) {
        Some(mode) if mode.starts_with("tb") => report(
            codes::OD9207,
            DiagnosticSite::new()
                .feature(FeatureId::TextVertical)
                .data("mode", mode),
        ),
        Some("rl-tb" | "rl") => out.rtl = Some(true),
        Some("lr-tb" | "lr") => out.rtl = Some(false),
        _ => {}
    }
    out
}

/// `fo:line-height` (a percentage of single spacing, `normal`, or a length) and its at-least form.
fn line_spacing(set: &PropertySet) -> Option<LineSpacing> {
    let height = get(set, qn!("fo:line-height"));
    if height == Some("normal") {
        return Some(LineSpacing::Multiple {
            value: ratio_of(1.0),
            basis: LINE_BASIS,
        });
    }
    if let Some(pct) = percent(height) {
        return Some(LineSpacing::Multiple {
            value: ratio_of(pct.max(0.0)),
            basis: LINE_BASIS,
        });
    }
    if let Some(exact) = size_pt(height) {
        return Some(LineSpacing::Exact { value: size(exact) });
    }
    size_pt(get(set, qn!("style:line-height-at-least")))
        .map(|value| LineSpacing::AtLeast { value: size(value) })
}

fn line_style(style: Option<&str>, kind: Option<&str>) -> Option<Decoration> {
    if style.is_none() && kind.is_none() {
        return None;
    }
    if style == Some("none") || kind == Some("none") {
        return Some(Decoration::None);
    }
    Some(if kind == Some("double") {
        Decoration::Double
    } else {
        Decoration::Single
    })
}

/// `style:text-position`: `super`, `sub`, or a raise as a percentage of the size, then the size the run draws at as
/// a percentage of its own.
fn text_position(value: Option<&str>) -> (Option<f64>, Option<f64>) {
    let Some(value) = value else {
        return (None, None);
    };
    let mut parts = slidock_canon::js_trim(value)
        .split(is_space)
        .filter(|part| !part.is_empty());
    let raise = parts.next().unwrap_or("");
    let size = parts.next();
    let baseline = match raise {
        "super" => Some(0.33),
        "sub" => Some(-0.33),
        _ => percent(Some(raise)).map(|pct| pct.clamp(-1.0, 1.0)),
    };
    let scale = percent(size).filter(|&scale| scale > 0.0).map(|scale| {
        slidock_math::quantise(scale.min(1.0), slidock_math::Grid::HundredThousandths)
    });
    (baseline, scale)
}

/// A text-properties set as IR run properties: every value it states. Effects outside the IR's model are reported.
pub(crate) fn run_props(
    set: &PropertySet,
    common: &CommonStyles<'_>,
    report: Report<'_>,
) -> PartialRunProps {
    let mut out = PartialRunProps::default();
    let font_name = get(set, qn!("style:font-name")).filter(|s| !s.is_empty());
    let font_family = get(set, qn!("fo:font-family")).filter(|s| !s.is_empty());
    let family = match (font_name, font_family) {
        (Some(name), _) => Some(common.font_family(name)),
        (None, Some(family)) => Some(unquote(family)),
        (None, None) => None,
    };
    if let Some(family) = family.filter(|f| !f.is_empty()) {
        out.font = Some(FontRef::Family(FamilyFont::new(
            utf16::clip(&family, 256).to_owned(),
        )));
    }
    if let Some(value) = size_pt(get(set, qn!("fo:font-size"))) {
        out.size = Some(size(value.min(4000.0)));
    }
    if let Some(weight) = get(set, qn!("fo:font-weight")) {
        let numeric = match weight {
            "normal" => 400.0,
            "bold" => 700.0,
            _ => js_number(weight),
        };
        if numeric.fract() == 0.0 && (100.0..=900.0).contains(&numeric) {
            out.weight = Some(numeric as u32);
        }
    }
    if let Some(style) = get(set, qn!("fo:font-style")) {
        out.italic = Some(style == "italic" || style == "oblique");
    }
    let underline_style = get(set, qn!("style:text-underline-style"));
    let underline = line_style(underline_style, get(set, qn!("style:text-underline-type")));
    out.underline = underline;
    if underline != Some(Decoration::None)
        && let Some(style) = underline_style
        && !style.is_empty()
        && style != "solid"
    {
        report(
            codes::OD9206,
            DiagnosticSite::new().data("style", utf16::clip(style, 32).to_owned()),
        );
    }
    out.strike = line_style(
        get(set, qn!("style:text-line-through-style")),
        get(set, qn!("style:text-line-through-type")),
    );
    if get(set, qn!("style:use-window-font-color")) == Some("true") {
        out.color = Some(Color::Rgb(rgb(0.0, 0.0, 0.0, 1.0)));
    } else if let Some(color) = hex_color(get(set, qn!("fo:color"))) {
        out.color = Some(Color::Rgb(color));
    }
    if let Some(highlight) = get(set, qn!("fo:background-color")) {
        out.highlight = Some(if highlight == "transparent" {
            None
        } else {
            hex_color(Some(highlight)).map(Color::Rgb)
        });
    }
    let (baseline, scale) = text_position(get(set, qn!("style:text-position")));
    out.baseline = baseline.map(ratio_of);
    out.script_scale = scale.map(unit_ratio);
    let transform = get(set, qn!("fo:text-transform"));
    let variant = get(set, qn!("fo:font-variant"));
    if variant == Some("small-caps") {
        out.caps = Some(Caps::Small);
    } else if transform == Some("uppercase") {
        out.caps = Some(Caps::All);
    } else if transform.is_some() || variant.is_some() {
        out.caps = Some(Caps::None);
    }
    if let Some(transform @ ("lowercase" | "capitalize")) = transform {
        report(
            codes::OD9205,
            DiagnosticSite::new()
                .feature(FeatureId::TextCaps)
                .data("transform", transform),
        );
    }
    if let Some(spacing) = get(set, qn!("fo:letter-spacing")) {
        out.letter_spacing = Some(pt(if spacing == "normal" {
            0.0
        } else {
            length_pt(Some(spacing)).unwrap_or(0.0)
        }));
    }
    let outline = get(set, qn!("style:text-outline")) == Some("true");
    let relief = get(set, qn!("style:font-relief")).is_some_and(|s| !s.is_empty() && s != "none");
    let effects: Vec<&str> = [(outline, "outline"), (relief, "relief")]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
    if !effects.is_empty() {
        report(
            codes::OD9204,
            DiagnosticSite::new().data("effects", effects.join(", ")),
        );
    }
    out
}

/// LibreOffice's character shadow offset, right and down, as a share of the text size: its 1/24 of the font height,
/// measured at 0.0451 of the size over 12–96 pt Liberation Sans.
const TEXT_SHADOW_OFFSET: f64 = 0.0451;

/// The character shadow of fully resolved text properties, as LibreOffice draws it: any `fo:text-shadow` but `none`
/// turns it on, and the value's own offset, blur and colour are ignored (`TextDecoratedPortionPrimitive2D`). Unblurred,
/// page-fixed, black, or light grey on text whose colour is black or of luminance under 8/255.
pub(crate) fn text_shadow(full: &PropertySet) -> Option<SourceShadow> {
    if get(full, qn!("fo:text-shadow")).is_none_or(|value| value == "none") {
        return None;
    }
    let text_size = size_pt(get(full, qn!("fo:font-size"))).unwrap_or(18.0);
    let color = if get(full, qn!("style:use-window-font-color")) == Some("true") {
        None
    } else {
        hex_color(get(full, qn!("fo:color")))
    };
    let luminance = color.map_or(0.0, |c| {
        0.30 * c.r.to_f64() + 0.59 * c.g.to_f64() + 0.11 * c.b.to_f64()
    });
    let grey = 192.0 / 255.0;
    let shadow_color = if luminance < 8.0 / 255.0 {
        rgb(grey, grey, grey, 1.0)
    } else {
        rgb(0.0, 0.0, 0.0, 1.0)
    };
    let offset = pt(quantise(TEXT_SHADOW_OFFSET * text_size, Grid::Thousandths));
    Some(SourceShadow {
        dx: offset,
        dy: offset,
        sigma: size(0.0),
        color: Color::Rgb(shadow_color),
        rotate_with_shape: false,
        overflow_casts: false,
    })
}

/// A run's shadow over the level it cascades from: stated only where it differs from the level's.
fn run_shadow(full: &PropertySet, level: Option<&SourceShadow>) -> Option<Option<SourceShadow>> {
    let own = text_shadow(full);
    (own.as_ref() != level).then_some(own)
}

/// Whether a run's properties state nothing.
fn is_empty_run(props: &PartialRunProps) -> bool {
    *props == PartialRunProps::default()
}

/// The features a paragraph's inlines hold: what they say and the properties their runs declare.
fn inline_features(inlines: &[SourceInline], text: &TextContext<'_, '_>) {
    for inline in inlines {
        match inline {
            Inline::Run {
                text: run, props, ..
            } => {
                if !run.is_empty() {
                    text.seen(FeatureId::TextRun);
                }
                run_features(props, text);
            }
            Inline::Field { props, .. } => {
                text.seen(FeatureId::TextField);
                run_features(props, text);
            }
            Inline::Tab {} => text.seen(FeatureId::TextTab),
            Inline::Break { props } => {
                text.seen(FeatureId::TextBreak);
                if let Some(props) = props {
                    run_features(props, text);
                }
            }
        }
    }
}

/// The features a run's declared properties hold in the source.
fn run_features(props: &PartialRunProps, text: &TextContext<'_, '_>) {
    if props.weight.is_some_and(|weight| weight != 400) {
        text.seen(FeatureId::TextWeight);
    }
    if props.italic == Some(true) {
        text.seen(FeatureId::TextItalic);
    }
    if props.underline.is_some_and(|u| u != Decoration::None) {
        text.seen(FeatureId::TextUnderline);
    }
    if props.strike.is_some_and(|s| s != Decoration::None) {
        text.seen(FeatureId::TextStrike);
    }
    if matches!(props.highlight, Some(Some(_))) {
        text.seen(FeatureId::TextHighlight);
    }
    if props.baseline.is_some_and(|b| b.steps() != 0) {
        text.seen(FeatureId::TextBaseline);
    }
    if props.caps.is_some_and(|c| c != Caps::None) {
        text.seen(FeatureId::TextCaps);
    }
    if props.letter_spacing.is_some_and(|l| l.steps() != 0) {
        text.seen(FeatureId::TextLetterSpacing);
    }
    if matches!(props.link, Some(Some(_))) {
        text.seen(FeatureId::TextLink);
    }
    if matches!(props.shadow, Some(Some(_))) {
        text.seen(FeatureId::TextShadow);
    }
}

fn pick(set: &PropertySet, keys: &BTreeSet<String>) -> PropertySet {
    set.iter()
        .filter(|(key, _)| keys.contains(*key))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// The text area of graphic properties: its padding as insets, and its vertical alignment as anchor.
pub(crate) fn text_area(graphic: &PropertySet) -> (PartialInsets, Anchor) {
    let padding = length_pt(get(graphic, qn!("fo:padding"))).unwrap_or(0.0);
    let inset = |side: QName| Some(pt(length_pt(get(graphic, side)).unwrap_or(padding)));
    let anchor = match get(graphic, qn!("draw:textarea-vertical-align")) {
        Some("middle") => Anchor::Middle,
        Some("bottom") => Anchor::Bottom,
        _ => Anchor::Top,
    };
    (
        PartialInsets {
            l: inset(qn!("fo:padding-left")),
            t: inset(qn!("fo:padding-top")),
            r: inset(qn!("fo:padding-right")),
            b: inset(qn!("fo:padding-bottom")),
        },
        anchor,
    )
}

/// The body properties of a shape's graphic properties: padding, anchor, wrap and autofit. Only custom shapes follow
/// `fo:wrap-option`; text frames and other shapes wrap at their width unless they grow to their text. Overflow is the
/// presentation default's.
pub(crate) fn body_props(
    graphic: &PropertySet,
    custom_shape: bool,
    seen: &dyn Fn(FeatureId),
) -> PartialBodyProps {
    let grow_width = get(graphic, qn!("draw:auto-grow-width")) == Some("true");
    let grow_height = get(graphic, qn!("draw:auto-grow-height")) == Some("true");
    let no_wrap = if custom_shape {
        get(graphic, qn!("fo:wrap-option")) == Some("no-wrap")
    } else {
        grow_width
    };
    let fit = get(graphic, qn!("draw:fit-to-size"));
    let autofit = if get(graphic, qn!("style:shrink-to-fit")) == Some("true")
        || matches!(fit, Some("shrink-to-fit" | "true" | "all"))
    {
        seen(FeatureId::TextAutofitShrink);
        // ODF stores no scale; LibreOffice fits the text by PowerPoint's steps.
        Autofit::Shrink {
            font_scale: None,
            line_spacing_reduction: None,
            fitted: None,
            steps: Some(ShrinkSteps::Office),
        }
    } else if grow_width || grow_height {
        seen(FeatureId::TextAutofitResize);
        let anchor_x = match get(graphic, qn!("draw:textarea-horizontal-align")) {
            Some("center") => AnchorX::Center,
            Some("right") => AnchorX::Right,
            _ => AnchorX::Left,
        };
        Autofit::Resize {
            width: grow_width,
            height: grow_height,
            anchor_x,
        }
    } else {
        Autofit::None {}
    };
    let (insets, anchor) = text_area(graphic);
    PartialBodyProps {
        insets: Some(insets),
        anchor: Some(anchor),
        wrap: Some(if no_wrap { Wrap::None } else { Wrap::Square }),
        autofit: Some(autofit),
        overflow: None,
        max_width: None,
    }
}

/// Where a paragraph sits in lists: its depth, list style, start value and whether it shows its item's label.
#[derive(Clone, Copy)]
struct ListPlace<'a> {
    in_list: bool,
    depth: i64,
    list: Option<&'a Element>,
    start_at: Option<i64>,
    labelled: bool,
}

/// A text body being read: its paragraphs, the levels they use, the character count against the limit.
struct Reading<'a, 'd, 't> {
    text: &'t TextContext<'a, 'd>,
    level_props: &'t dyn Fn(u32) -> Rc<Props<'a>>,
    /// The shape's list style: the last fallback, after the list's own and the paragraph style's.
    shape_list: Option<&'a Element>,
    paragraphs: Vec<SourceParagraph>,
    levels: BTreeSet<u32>,
    chars: usize,
    /// Tells apart the run properties of each reading of an element's inlines, which a run continues only within.
    next_props: Cell<u64>,
}

/// Inline content being collected for one paragraph.
struct InlineBuilder {
    inlines: Vec<SourceInline>,
    /// At a paragraph's start, and after a space, white space in the text is ignored.
    skip_space: bool,
    /// The run being collected: its text, its length in UTF-16 code units, and its properties with their reading.
    pending: Option<(String, usize, u64, PartialRunProps)>,
}

impl InlineBuilder {
    fn new() -> Self {
        InlineBuilder {
            inlines: Vec::new(),
            skip_space: true,
            pending: None,
        }
    }

    fn count(
        &mut self,
        reading: &mut Reading<'_, '_, '_>,
        length: usize,
    ) -> Result<(), LowerError> {
        reading.chars = reading.chars.saturating_add(length);
        let limit = reading.text.ctx.limits.text_per_body;
        if reading.chars > limit {
            return Err(LowerError::Limit(format!(
                "a text body holds more than {limit} characters"
            )));
        }
        Ok(())
    }

    /// Text as written, its white space collapsed.
    fn text(
        &mut self,
        reading: &mut Reading<'_, '_, '_>,
        raw: &str,
        props: &(u64, PartialRunProps),
    ) -> Result<(), LowerError> {
        let mut out = String::new();
        for c in raw.chars() {
            if matches!(c, ' ' | '\t' | '\n' | '\r') {
                if self.skip_space {
                    continue;
                }
                out.push(' ');
                self.skip_space = true;
            } else {
                out.push(c);
                self.skip_space = false;
            }
        }
        self.literal(reading, &out, props)
    }

    /// Text kept verbatim (`text:s`).
    fn literal(
        &mut self,
        reading: &mut Reading<'_, '_, '_>,
        text: &str,
        props: &(u64, PartialRunProps),
    ) -> Result<(), LowerError> {
        if text.is_empty() {
            return Ok(());
        }
        let length = utf16::len(text);
        self.count(reading, length)?;
        match &mut self.pending {
            Some((pending, units, id, _)) if *id == props.0 => {
                pending.push_str(text);
                *units += length;
            }
            _ => {
                self.flush();
                self.pending = Some((text.to_owned(), length, props.0, props.1.clone()));
            }
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|(_, units, _, _)| *units > MAX_STRING)
        {
            return Err(LowerError::Limit(format!(
                "a run holds more than {MAX_STRING} characters"
            )));
        }
        if !text.chars().next_back().is_some_and(is_space) {
            self.skip_space = false;
        }
        Ok(())
    }

    fn push(&mut self, inline: SourceInline) {
        self.flush();
        self.inlines.push(inline);
        self.skip_space = false;
    }

    fn flush(&mut self) {
        if let Some((text, _, _, props)) = self.pending.take() {
            self.inlines.push(Inline::Run {
                text,
                props,
                line_height: None,
                leading_skew: None,
            });
        }
    }

    fn finish(mut self) -> Vec<SourceInline> {
        self.flush();
        self.inlines
    }
}

fn field_kind(local: &str) -> Option<FieldKind> {
    match local {
        "page-number" => Some(FieldKind::SlideNumber),
        "page-count" => Some(FieldKind::SlideCount),
        "date" => Some(FieldKind::Date),
        "time" => Some(FieldKind::Time),
        _ => None,
    }
}

/// Text elements that stand for a value (title, author, file name …) and hold its last text.
const VALUE_FIELDS: [&str; 15] = [
    "title",
    "subject",
    "description",
    "file-name",
    "author-name",
    "author-initials",
    "sender-firstname",
    "sender-lastname",
    "creator",
    "creation-date",
    "modification-date",
    "chapter",
    "sequence",
    "variable-get",
    "user-defined",
];

fn field(kind: FieldKind, cached: String, props: PartialRunProps) -> SourceInline {
    Inline::Field {
        field: kind,
        cached,
        props,
        line_height: None,
        leading_skew: None,
    }
}

/// One level of span content being read: its nodes, its resolved properties, the property names its style chains
/// declare, its link, and the run properties of this reading of it once a node needed them.
struct SpanLevel<'a> {
    nodes: std::slice::Iter<'a, XmlNode>,
    full: Props<'a>,
    declared: BTreeSet<String>,
    link: Option<Link>,
    /// The shadow of the level the paragraph's runs cascade from.
    level_shadow: Option<SourceShadow>,
    current: Option<(u64, PartialRunProps)>,
}

impl<'a> SpanLevel<'a> {
    /// The run properties of this level's text, read once (and reported once) per level.
    fn run_properties(&mut self, reading: &Reading<'a, '_, '_>) -> (u64, PartialRunProps) {
        if let Some(current) = &self.current {
            return current.clone();
        }
        let text = reading.text;
        let report = |code, site| text.report(code, site);
        let mut own = run_props(
            &pick(&self.full.text, &self.declared),
            text.ctx.styles.common,
            &report,
        );
        if let Some(link) = &self.link {
            own.link = Some(Some(link.clone()));
        }
        own.shadow = run_shadow(&self.full.text, self.level_shadow.as_ref());
        let id = reading.next_props.get();
        reading.next_props.set(id + 1);
        self.current.insert((id, own)).clone()
    }
}

/// Span content: text, spans, links, spaces, tabs, breaks and fields, with their resolved properties. Spans are
/// walked with a stack of their own, so nesting costs no call depth.
fn read_inlines<'a>(
    parent: &'a Element,
    full: &Props<'a>,
    declared: &BTreeSet<String>,
    level_shadow: Option<&SourceShadow>,
    builder: &mut InlineBuilder,
    reading: &mut Reading<'a, '_, '_>,
) -> Result<(), LowerError> {
    let text = reading.text;
    let styles = text.ctx.styles;
    let mut levels = vec![SpanLevel {
        nodes: parent.children.iter(),
        full: full.clone(),
        declared: declared.clone(),
        link: None,
        level_shadow: level_shadow.cloned(),
        current: None,
    }];
    while let Some(level) = levels.last_mut() {
        let Some(node) = level.nodes.next() else {
            levels.pop();
            continue;
        };
        let node = match node {
            XmlNode::Text(raw) => {
                if !raw.is_empty() {
                    let props = level.run_properties(reading);
                    builder.text(reading, raw, &props)?;
                }
                continue;
            }
            XmlNode::Element(element) => element,
        };
        if node.is(Ns::Text, "span") || node.is(Ns::Text, "a") {
            let level_shadow = level.level_shadow.clone();
            let style = node.attr(qn!("text:style-name"));
            let chain = styles.over(Family::Text, style, &level.full);
            let mut keys = level.declared.clone();
            keys.extend(styles.declared(Family::Text, style).text);
            let href = if node.is(Ns::Text, "a") {
                read_href(node.attr(qn!("xlink:href")), text.ctx.pages)
            } else {
                None
            };
            let link = href.or_else(|| level.link.clone());
            levels.push(SpanLevel {
                nodes: node.children.iter(),
                full: chain,
                declared: keys,
                link,
                level_shadow,
                current: None,
            });
        } else if node.is(Ns::Text, "s") {
            let count = node.int_attr(qn!("text:c")).unwrap_or(1).clamp(1, 1024);
            let props = level.run_properties(reading);
            let spaces = " ".repeat(usize::try_from(count).unwrap_or(1));
            builder.literal(reading, &spaces, &props)?;
        } else if node.is(Ns::Text, "tab") {
            builder.push(Inline::Tab {});
        } else if node.is(Ns::Text, "line-break") {
            let (_, own) = level.run_properties(reading);
            builder.push(Inline::Break {
                props: (!is_empty_run(&own)).then_some(own),
            });
        } else if let Some(kind) = node
            .in_namespace(Ns::Text)
            .then(|| field_kind(&node.local))
            .flatten()
        {
            let cached = utf16::clip(&node.text_of(), 256).to_owned();
            let (_, props) = level.run_properties(reading);
            builder.push(field(kind, cached, props));
        } else if node.is(Ns::Presentation, "date-time") {
            let declaration = text.declarations.and_then(|d| d.date_time.as_ref());
            match declaration {
                Some(declaration) if declaration.fixed => {
                    let props = level.run_properties(reading);
                    builder.text(reading, &declaration.text, &props)?;
                }
                _ => {
                    let cached = declaration.map_or_else(String::new, |d| d.text.clone());
                    let (_, props) = level.run_properties(reading);
                    builder.push(field(FieldKind::Date, cached, props));
                }
            }
        } else if node.is(Ns::Presentation, "footer") || node.is(Ns::Presentation, "header") {
            let value = text.declarations.and_then(|d| {
                if node.local == "footer" {
                    d.footer.as_ref()
                } else {
                    d.header.as_ref()
                }
            });
            if let Some(value) = value.filter(|v| !v.is_empty()) {
                let props = level.run_properties(reading);
                builder.text(reading, value, &props)?;
            }
        } else if node.in_namespace(Ns::Text) && VALUE_FIELDS.contains(&node.local.as_str()) {
            text.report(
                codes::OD9210,
                DiagnosticSite::new().data("field", node.name.as_str()),
            );
            let props = level.run_properties(reading);
            builder.text(reading, &node.text_of(), &props)?;
        }
    }
    Ok(())
}

/// One paragraph: its level, its own properties, list bullet and indents, inlines and paragraph mark.
fn read_paragraph<'a>(
    p: &'a Element,
    place: ListPlace<'a>,
    reading: &mut Reading<'a, '_, '_>,
) -> Result<(), LowerError> {
    let text = reading.text;
    let styles = text.ctx.styles;
    let report = |code, site| text.report(code, site);
    let level = u32::try_from(place.depth.clamp(0, i64::from(MAX_LEVEL))).unwrap_or(0);
    reading.levels.insert(level);
    let base = (reading.level_props)(level);
    let style = p.attr(qn!("text:style-name"));
    let full = styles.over(Family::Paragraph, style, &base);
    let own = styles.over(Family::Paragraph, style, &Props::default());
    let declared_keys = styles.declared(Family::Paragraph, style);
    let mut props = para_props(&pick(&full.paragraph, &declared_keys.paragraph), &report);

    let list_style = place.list.or(own.list_style).or(reading.shape_list);
    if place.in_list {
        let effective = para_props(&full.paragraph, &silent);
        let item = list_level(
            list_style,
            level,
            place.start_at,
            styles.common,
            &|code, site| text.report(code, site),
        );
        let bullet = if place.labelled {
            item.bullet
        } else {
            Bullet::None {}
        };
        let margin = effective.margin_left.map_or(0.0, |m| m.to_f64());
        props.margin_left = Some(pt(to_pt(margin + item.text)));
        props.indent = Some(pt(if matches!(bullet, Bullet::None {}) {
            0.0
        } else {
            to_pt(item.label - item.text)
        }));
        match bullet {
            Bullet::Char { .. } => text.seen(FeatureId::TextBulletChar),
            Bullet::Auto { .. } => text.seen(FeatureId::TextBulletAuto),
            _ => {}
        }
        props.bullet = Some(bullet);
    }
    if props.rtl == Some(true) {
        text.seen(FeatureId::TextRtl);
    }
    if let Some(line) = &props.line_spacing
        && !matches!(line, LineSpacing::Multiple { value, .. } if value.steps() == ratio_of(1.0).steps())
    {
        text.seen(FeatureId::TextLineSpacing);
    }

    let stops = own
        .tab_stops
        .into_iter()
        .flat_map(Element::kids)
        .filter(|stop| length_pt(stop.attr(qn!("style:position"))).unwrap_or(0.0) > 0.0)
        .count();
    if stops > 0 {
        text.report(codes::OD9202, DiagnosticSite::new());
    }
    let declared = &declared_keys.text;
    let level_shadow = text_shadow(&base.text);
    let mut builder = InlineBuilder::new();
    read_inlines(
        p,
        &full,
        declared,
        level_shadow.as_ref(),
        &mut builder,
        reading,
    )?;
    let inlines = builder.finish();
    inline_features(&inlines, text);
    let mut end_props = run_props(&pick(&full.text, declared), text.ctx.styles.common, &silent);
    end_props.shadow = run_shadow(&full.text, level_shadow.as_ref());
    reading.paragraphs.push(SourceParagraph {
        level,
        props,
        inlines,
        end_props: (!is_empty_run(&end_props)).then_some(end_props),
    });
    Ok(())
}

/// A list and its items, each item's paragraphs and nested lists one level deeper.
fn read_list<'a>(
    list: &'a Element,
    outer: ListPlace<'a>,
    reading: &mut Reading<'a, '_, '_>,
) -> Result<(), LowerError> {
    let style = reading
        .text
        .ctx
        .styles
        .list_style(list.attr(qn!("text:style-name")))
        .or(outer.list);
    let depth = outer.depth + 1;
    let mut starts: BTreeMap<i64, i64> = BTreeMap::new();
    for item in list.kids() {
        let header = item.is(Ns::Text, "list-header");
        if !header && !item.is(Ns::Text, "list-item") {
            continue;
        }
        if let Some(start) = item.int_attr(qn!("text:start-value")) {
            starts.insert(depth, start);
        }
        let mut first = !header;
        for node in item.kids() {
            if node.is(Ns::Text, "p") || node.is(Ns::Text, "h") {
                let place = ListPlace {
                    in_list: true,
                    depth,
                    list: style,
                    start_at: starts.get(&depth).copied(),
                    labelled: first,
                };
                read_paragraph(node, place, reading)?;
                first = false;
            } else if node.is(Ns::Text, "list") {
                let place = ListPlace {
                    in_list: true,
                    depth,
                    list: style,
                    start_at: None,
                    labelled: true,
                };
                read_list(node, place, reading)?;
                first = false;
            }
        }
    }
    Ok(())
}

/// Paragraphs and lists in document order.
fn read_blocks<'a>(
    holder: &'a Element,
    reading: &mut Reading<'a, '_, '_>,
) -> Result<(), LowerError> {
    for node in holder.kids() {
        if node.is(Ns::Text, "p") || node.is(Ns::Text, "h") {
            let place = ListPlace {
                in_list: false,
                depth: 0,
                list: None,
                start_at: None,
                labelled: false,
            };
            read_paragraph(node, place, reading)?;
        } else if node.is(Ns::Text, "list") {
            let place = ListPlace {
                in_list: true,
                depth: -1,
                list: None,
                start_at: None,
                labelled: true,
            };
            read_list(node, place, reading)?;
        }
    }
    Ok(())
}

/// A level's complete style: the shape's (or the master's outline level's) paragraph and text properties.
pub(crate) fn level_style_of(props: &Props<'_>, text: &TextContext<'_, '_>) -> LevelStyle {
    let report = |code, site| text.report(code, site);
    let mut run = run_props(&props.text, text.ctx.styles.common, &report);
    run.shadow = text_shadow(&props.text).map(Some);
    LevelStyle {
        para: Some(para_props(&props.paragraph, &report)),
        run: Some(run),
    }
}

/// Sets one level of a list style.
pub(crate) fn set_level(styles: &mut LevelStyles, level: u32, style: LevelStyle) {
    let level = usize::try_from(level).map_or(LEVELS - 1, |level| level.min(LEVELS - 1));
    if let Some(slot) = styles.slot_mut(level) {
        *slot = Some(Box::new(style));
    }
}

/// What reading a shape's text needs from the shape.
pub(crate) struct ShapeText<'a, 's> {
    /// The element holding the paragraphs: the shape itself, or a frame's `draw:text-box`.
    pub holder: &'a Element,
    /// The drawable, whose `draw:text-style-name` and presentation class apply.
    pub shape: &'a Element,
    /// The drawable's resolved style.
    pub style: &'s Props<'a>,
    /// A custom shape, whose text wraps as its wrap option says.
    pub custom_shape: bool,
}

/// A shape's text body; `None` when it holds no paragraph.
pub(crate) fn read_text_body<'a>(
    shape: &ShapeText<'a, '_>,
    text: &TextContext<'a, '_>,
) -> Result<Option<TextBody>, LowerError> {
    let Some((list_style, paragraphs)) = read_text(shape, text)? else {
        return Ok(None);
    };
    let seen = |feature| text.seen(feature);
    Ok(Some(TextBody {
        body: body_props(&shape.style.graphic, shape.custom_shape, &seen),
        list_style: Some(list_style),
        paragraphs,
        flips_with_frame: false,
        rotation: None,
    }))
}

/// A shape's paragraphs, and the list style of the levels they use; `None` when it holds no paragraph.
pub(crate) fn read_text<'a>(
    shape: &ShapeText<'a, '_>,
    text: &TextContext<'a, '_>,
) -> Result<Option<(LevelStyles, Vec<SourceParagraph>)>, LowerError> {
    let has_blocks = shape
        .holder
        .kids()
        .any(|node| node.is(Ns::Text, "p") || node.is(Ns::Text, "h") || node.is(Ns::Text, "list"));
    if !has_blocks {
        return Ok(None);
    }
    let styles = text.ctx.styles;
    let master_name = text.ctx.master_name.filter(|name| !name.is_empty());
    let outline = shape.shape.attr(qn!("presentation:class")) == Some("outline");
    let text_style = shape.shape.attr(qn!("draw:text-style-name"));
    let cache: RefCell<BTreeMap<u32, Rc<Props<'a>>>> = RefCell::new(BTreeMap::new());
    let level_props = |level: u32| -> Rc<Props<'a>> {
        if let Some(props) = cache.borrow().get(&level) {
            return Rc::clone(props);
        }
        // Deeper outline levels take the master page's outline style for that level, under the shape's own.
        let deeper = master_name.map(|name| format!("{name}-outline{}", level + 1));
        let base = match deeper {
            Some(deeper) if outline && level > 0 && styles.has(Family::Presentation, &deeper) => {
                styles.own(
                    Family::Presentation,
                    shape.shape.attr(qn!("presentation:style-name")),
                    &styles.resolve(Family::Presentation, Some(&deeper)),
                )
            }
            _ => shape.style.clone(),
        };
        let props = Rc::new(styles.over(Family::Paragraph, text_style, &base));
        cache.borrow_mut().insert(level, Rc::clone(&props));
        props
    };
    let mut reading = Reading {
        text,
        level_props: &level_props,
        shape_list: shape.style.list_style,
        paragraphs: Vec::new(),
        levels: BTreeSet::new(),
        chars: 0,
        next_props: Cell::new(0),
    };
    read_blocks(shape.holder, &mut reading)?;
    let mut list_style = LevelStyles::default();
    for level in &reading.levels {
        set_level(
            &mut list_style,
            *level,
            level_style_of(&level_props(*level), text),
        );
    }
    Ok(Some((list_style, reading.paragraphs)))
}

#[cfg(test)]
mod tests {
    use slidock_ir::text::FontRef;

    use super::*;
    use crate::defaults::{default_body, default_text, root_props};

    #[test]
    fn states_the_same_values_in_its_root_style_layer_and_the_presentation_defaults() {
        let root = root_props();
        let common = CommonStyles::default();
        let run = run_props(&root.text, &common, &silent);
        let para = para_props(&root.paragraph, &silent);
        let defaults = default_text();
        for (_, level) in defaults.levels() {
            let full = &level.run;
            assert_eq!(run.font.as_ref(), Some(&full.font));
            assert_eq!(run.size, Some(full.size));
            assert_eq!(run.weight, Some(full.weight));
            assert_eq!(run.italic, Some(full.italic));
            assert_eq!(run.color.as_ref(), Some(&full.color));
            let full = &level.para;
            assert_eq!(para.align, Some(full.align));
            assert_eq!(para.margin_left, Some(full.margin_left));
            assert_eq!(para.margin_right, Some(full.margin_right));
            assert_eq!(para.indent, Some(full.indent));
            assert_eq!(para.space_before.as_ref(), Some(&full.space_before));
            assert_eq!(para.space_after.as_ref(), Some(&full.space_after));
            assert_eq!(para.line_spacing.as_ref(), Some(&full.line_spacing));
            assert_eq!(para.tab_size, Some(full.tab_size));
        }
        let body = body_props(&root.graphic, false, &|_| {});
        let expected = default_body();
        assert_eq!(body.anchor, Some(expected.anchor));
        assert_eq!(body.wrap, Some(expected.wrap));
        assert_eq!(body.autofit, Some(expected.autofit));
        let insets = body.insets.unwrap();
        assert_eq!(
            [insets.l, insets.t, insets.r, insets.b],
            [
                Some(expected.insets.l),
                Some(expected.insets.t),
                Some(expected.insets.r),
                Some(expected.insets.b)
            ]
        );
        // The default family is a family, never the text's.
        assert!(matches!(run.font, Some(FontRef::Family(_))));
    }

    #[test]
    fn reads_text_positions() {
        assert_eq!(text_position(Some("super 58%")), (Some(0.33), Some(0.58)));
        assert_eq!(text_position(Some("-20% 150%")), (Some(-0.2), Some(1.0)));
        assert_eq!(text_position(Some("200%")), (Some(1.0), None));
        assert_eq!(text_position(Some("")), (None, None));
    }
}
