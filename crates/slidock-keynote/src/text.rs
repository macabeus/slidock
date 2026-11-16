//! Text storages. A storage is one string with attribute tables keyed by character index (UTF-16 code units): a
//! paragraph style per paragraph, character-style overrides (an entry without a style returns to the paragraph's),
//! list styles and levels, smart fields (links) and attachments at U+FFFC. Each table is read by one forward cursor,
//! so a storage costs one pass whatever its tables hold. Keynote's style chains resolve here into explicit IR values;
//! a run holds what its chain sets, and the presentation defaults (Keynote's own) fill the rest.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use slidock_canon::{js_trim, utf16};
use slidock_core_api::lowering::LowerError;
use slidock_ir::color::Color;
use slidock_ir::diagnostics::{DiagnosticCode, DiagnosticSite, codes};
use slidock_ir::features::FeatureId;
use slidock_ir::ids::NodeId;
use slidock_ir::text::{
    Align, Anchor, AnchorX, Autofit, BodyProps, Bullet, Caps, Decoration, FamilyFont, FieldKind,
    FontRef, Inline, Insets, LineBasis, LineSpacing, Link, Overflow, ParaProps, PartialBodyProps,
    PartialInsets, PartialParaProps, PartialRunProps, RunProps, ShrinkSteps, SourceParagraph,
    Spacing, TextBody, Wrap,
};
use slidock_ir::units::{Pt, Ratio, Size};

use crate::context::Lowering;
use crate::fonts::{DEFAULT_POSTSCRIPT_NAME, font_face};
use crate::geometry::{HEIGHT_VALID, SlideMap, WIDTH_VALID};
use crate::graph::referenced;
use crate::num::{clamp, js_max, js_min, pt, ratio, size, unit_ratio, wide};
use crate::paint::{ShadowRead, black, color, shadow};
use crate::proto::{kn, tsp, tswp};
use crate::styles::{
    CharacterStyle, ListStyle, ParagraphStyle, TextShapeStyle, inherited, style_chain,
};

/// What a relative line spacing means in Keynote: CoreText's line, the font's hhea ascent, descent and line gap each
/// rounded to whole points, with the spacing scaling ascent plus descent. Keynote's exports match CoreText's own
/// layout of the same font to the point (Monaco 47 pt: 63 pt lines, not the 62.67 pt of unrounded hhea).
pub const KEYNOTE_LINE_BASIS: LineBasis = LineBasis::Coretext;

pub const DEFAULT_SIZE: f64 = 24.0;
/// A text shape's inset on each side when its style sets none; a `padding_null` style means none at all.
pub const DEFAULT_PADDING: f64 = 4.0;
const DEFAULT_TAB: f64 = 36.0;

fn single_spacing() -> LineSpacing {
    LineSpacing::Multiple {
        value: ratio(1.0),
        basis: KEYNOTE_LINE_BASIS,
    }
}

/// Keynote's paragraph defaults, the presentation's `defaults.text` at every level.
pub fn keynote_para() -> ParaProps {
    ParaProps {
        align: Align::Left,
        margin_left: Pt::ZERO,
        margin_right: Pt::ZERO,
        indent: Pt::ZERO,
        space_before: Spacing::Pt { value: Pt::ZERO },
        space_after: Spacing::Pt { value: Pt::ZERO },
        line_spacing: single_spacing(),
        tab_size: size(DEFAULT_TAB),
        bullet: Bullet::None {},
        rtl: false,
        line_height: None,
        space_before_pt: None,
        space_after_pt: None,
        leading_skew: None,
        line_tail_pt: None,
    }
}

/// Keynote's run defaults, the presentation's `defaults.text` at every level.
pub fn keynote_run() -> RunProps {
    RunProps {
        font: FontRef::Family(FamilyFont::new(font_face(DEFAULT_POSTSCRIPT_NAME).family)),
        size: size(DEFAULT_SIZE),
        weight: 400,
        italic: false,
        underline: Decoration::None,
        strike: Decoration::None,
        color: Color::Rgb(black()),
        highlight: None,
        baseline: Ratio::ZERO,
        caps: Caps::None,
        letter_spacing: Pt::ZERO,
        link: None,
        // Keynote draws a bold run in a family without a bold face with the regular face.
        synthesize_bold: false,
        // Keynote's styles are resolved here, sizes of raised and lowered runs included.
        script_scale: unit_ratio(1.0),
        shadow: None,
    }
}

fn default_insets() -> Insets {
    let side = pt(DEFAULT_PADDING);
    Insets {
        l: side,
        t: side,
        r: side,
        b: side,
    }
}

/// Keynote's text-box defaults, the presentation's `defaults.body`.
pub fn keynote_body() -> BodyProps<Autofit> {
    BodyProps {
        insets: default_insets(),
        anchor: Anchor::Top,
        wrap: Wrap::Square,
        autofit: Autofit::None {},
        overflow: Overflow::Below,
        max_width: None,
    }
}

const NEWLINE: u16 = 0x0a;
const CARRIAGE_RETURN: u16 = 0x0d;
const PARAGRAPH_SEPARATOR: u16 = 0x2029;
const LINE_SEPARATOR: u16 = 0x2028;
const TAB: u16 = 0x09;
const OBJECT_REPLACEMENT: u16 = 0xfffc;

/// Text set from a script can hold a carriage return where Keynote's own editing puts a newline.
fn is_paragraph_separator(unit: u16) -> bool {
    matches!(unit, NEWLINE | CARRIAGE_RETURN | PARAGRAPH_SEPARATOR)
}

/// Keynote's slide number field; Pages and Keynote both write the generic number attachment for it too.
const SLIDE_NUMBER_ATTACHMENTS: [&str; 2] = [
    "KN.SlideNumberAttachmentArchive",
    "TSWP.NumberAttachmentArchive",
];

const ALIGNMENTS: [Align; 4] = [Align::Left, Align::Right, Align::Center, Align::Justify];
const NATURAL: i32 = 4;
/// Vertical alignments: top, middle, bottom, and justified, which sets text from the top.
pub const VERTICAL: [Anchor; 4] = [Anchor::Top, Anchor::Middle, Anchor::Bottom, Anchor::Top];

/// Superscript and subscript: Keynote's raised or lowered text at a reduced size.
const SCRIPT_SCALE: f64 = 0.7;
const SCRIPT_RAISE: f64 = 0.33;
const SCRIPT_LOWER: f64 = -0.2;

/// A vertical alignment by value, top when the value names none.
pub fn vertical(value: Option<i32>) -> Anchor {
    usize::try_from(value.unwrap_or(0))
        .ok()
        .and_then(|i| VERTICAL.get(i))
        .copied()
        .unwrap_or(Anchor::Top)
}

/// A text coordinate in points, within the IR's range.
fn text_pt(x: f64) -> Pt {
    pt(clamp(x, -100_000.0, 100_000.0))
}

fn text_size(x: f64) -> Size {
    size(clamp(x, -100_000.0, 100_000.0))
}

/// An attribute table's entry: where it starts, and the object it names.
trait Entry {
    fn start(&self) -> u32;
    fn object(&self) -> Option<&tsp::Reference>;
}

impl Entry for tswp::object_attribute_table::ObjectAttribute {
    fn start(&self) -> u32 {
        self.character_index.unwrap_or(0)
    }

    fn object(&self) -> Option<&tsp::Reference> {
        self.object.as_ref()
    }
}

impl Entry for tswp::para_data_attribute_table::ParaDataAttribute {
    fn start(&self) -> u32 {
        self.character_index.unwrap_or(0)
    }

    fn object(&self) -> Option<&tsp::Reference> {
        None
    }
}

/// The entry in force at a character index, read forward: the last entry starting at or before it. Indexes passed
/// to `at` only grow or repeat, so a table is walked once. Entries are named by their position.
struct Cursor<'e, E: Clone> {
    entries: Cow<'e, [E]>,
    next: usize,
    current: Option<usize>,
    /// The last entry passed that holds a style; a paragraph entry without one keeps the style before it.
    styled: Option<usize>,
}

impl<'e, E: Entry + Clone> Cursor<'e, E> {
    fn new(entries: Cow<'e, [E]>) -> Self {
        Cursor {
            entries,
            next: 0,
            current: None,
            styled: None,
        }
    }

    fn at(&mut self, index: usize) -> Option<usize> {
        while let Some(entry) = self.entries.get(self.next) {
            if entry.start() as usize > index {
                break;
            }
            self.current = Some(self.next);
            if entry.object().is_some() {
                self.styled = Some(self.next);
            }
            self.next += 1;
        }
        self.current
    }

    fn entry(&self, at: Option<usize>) -> Option<&E> {
        at.and_then(|at| self.entries.get(at))
    }

    /// The object of the entry in force at `index`.
    fn object(&mut self, index: usize) -> Option<tsp::Reference> {
        let at = self.at(index);
        self.entry(at).and_then(|entry| entry.object().cloned())
    }

    /// The style in force at `index`, skipping entries without one.
    fn style(&mut self, index: usize) -> Option<tsp::Reference> {
        self.at(index);
        self.entry(self.styled)
            .and_then(|entry| entry.object().cloned())
    }

    /// The entry starting exactly at `index`.
    fn exactly(&mut self, index: usize) -> Option<&E> {
        let at = self.at(index);
        self.entry(at)
            .filter(|entry| entry.start() as usize == index)
    }
}

/// A table's entries in index order; an out-of-order table (Keynote writes them sorted) is sorted and flagged.
fn sorted<'e, E: Entry + Clone>(entries: &'e [E], unsorted: &mut bool) -> Cow<'e, [E]> {
    let in_order = entries
        .windows(2)
        .all(|pair| matches!(pair, [a, b] if b.start() >= a.start()));
    if in_order {
        return Cow::Borrowed(entries);
    }
    *unsorted = true;
    let mut out = entries.to_vec();
    out.sort_by_key(Entry::start);
    Cow::Owned(out)
}

fn object_entries(
    table: Option<&tswp::ObjectAttributeTable>,
) -> &[tswp::object_attribute_table::ObjectAttribute] {
    table
        .map(|table| table.entries.as_slice())
        .unwrap_or_default()
}

/// A diagnostic a run style reports once an inline draws with it.
#[derive(Clone, Debug)]
struct Report {
    code: DiagnosticCode,
    feature: Option<FeatureId>,
    style: Option<&'static str>,
    reason: Option<String>,
}

/// A run style: the properties its chains set, and what using it in a run records (features, reports).
#[derive(Clone, Debug)]
struct RunStyle {
    props: PartialRunProps,
    /// The run's drawn size, which relative values scale with.
    size: f64,
    postscript_name: String,
    /// Whether a style names the font; only a named font becomes a presentation font request.
    named: bool,
    /// Whether the named face is the face drawn: bold or italic on a lighter or upright face draws another one.
    exact: bool,
    features: Vec<FeatureId>,
    reports: Vec<Report>,
}

/// The run properties a run's chains set: its character style's, then its paragraph style's.
fn run_style(
    ctx: &Lowering<'_>,
    char_ref: Option<&tsp::Reference>,
    para_ref: Option<&tsp::Reference>,
) -> RunStyle {
    let mut chain = style_chain::<CharacterStyle>(ctx.graph, char_ref);
    chain.extend(style_chain::<CharacterStyle>(ctx.graph, para_ref));
    let chain = chain.as_slice();
    let mut props = PartialRunProps::default();
    let mut features = Vec::new();
    let mut reports = Vec::new();
    let approximated = |reports: &mut Vec<Report>, style: &'static str, feature| {
        reports.push(Report {
            code: codes::KN1207,
            feature,
            style: Some(style),
            reason: None,
        });
    };
    let font_name = inherited(chain, |p| (p.font_name.as_deref(), p.font_name_null));
    let face = font_face(font_name.unwrap_or(DEFAULT_POSTSCRIPT_NAME));
    let bold = inherited(chain, |p| (p.bold.as_ref(), None)).copied();
    let italic = inherited(chain, |p| (p.italic.as_ref(), None)).copied();
    let superscript = inherited(chain, |p| (p.superscript.as_ref(), None))
        .copied()
        .unwrap_or(0);
    let script_scale = if superscript == 1 || superscript == 2 {
        SCRIPT_SCALE
    } else {
        1.0
    };
    let declared_size = inherited(chain, |p| (p.font_size.as_ref(), None)).map(|x| f64::from(*x));
    let size = js_min(
        4000.0,
        js_max(0.0, declared_size.unwrap_or(DEFAULT_SIZE) * script_scale),
    );
    if font_name.is_some() {
        props.font = Some(FontRef::Family(FamilyFont::new(face.family.clone())));
    }
    if font_name.is_some() || bold.is_some() {
        let weight = if bold == Some(true) {
            face.weight.max(700)
        } else {
            face.weight
        };
        props.weight = Some(weight);
        if weight != 400 {
            features.push(FeatureId::TextWeight);
        }
    }
    if italic.is_some() || face.italic {
        let value = italic == Some(true) || face.italic;
        props.italic = Some(value);
        if value {
            features.push(FeatureId::TextItalic);
        }
    }
    if declared_size.is_some() || script_scale != 1.0 {
        props.size = Some(text_size(size));
    }
    if let Some(font_color) = color(inherited(chain, |p| {
        (p.font_color.as_ref(), p.font_color_null)
    })) {
        props.color = Some(Color::Rgb(font_color));
    }
    if let Some(&underline) = inherited(chain, |p| (p.underline.as_ref(), None)) {
        props.underline = Some(match underline {
            0 => Decoration::None,
            2 => Decoration::Double,
            _ => Decoration::Single,
        });
        if underline != 0 {
            features.push(FeatureId::TextUnderline);
        }
        if underline == 3 {
            approximated(
                &mut reports,
                "wavy underline",
                Some(FeatureId::TextUnderline),
            );
        }
    }
    if let Some(&strike) = inherited(chain, |p| (p.strikethru.as_ref(), None)) {
        props.strike = Some(match strike {
            0 => Decoration::None,
            1 => Decoration::Single,
            _ => Decoration::Double,
        });
        if strike != 0 {
            features.push(FeatureId::TextStrike);
        }
        if strike == 3 {
            approximated(
                &mut reports,
                "triple strikethrough",
                Some(FeatureId::TextStrike),
            );
        }
    }
    let shift =
        inherited(chain, |p| (p.baseline_shift.as_ref(), None)).map_or(0.0, |x| f64::from(*x));
    let raise = match superscript {
        1 => SCRIPT_RAISE,
        2 => SCRIPT_LOWER,
        _ => 0.0,
    };
    if raise != 0.0 || shift != 0.0 {
        let relative = if size > 0.0 { shift / size } else { 0.0 };
        props.baseline = Some(ratio(clamp(raise + relative, -1.0, 1.0)));
        features.push(FeatureId::TextBaseline);
    }
    if let Some(&caps) = inherited(chain, |p| (p.capitalization.as_ref(), None)) {
        props.caps = Some(match caps {
            1 => Caps::All,
            2 => Caps::Small,
            _ => Caps::None,
        });
        if caps == 1 || caps == 2 {
            features.push(FeatureId::TextCaps);
        }
        if caps == 3 {
            approximated(&mut reports, "title case", Some(FeatureId::TextCaps));
        }
    }
    // Tracking is a fraction of the font size.
    if let Some(tracking) = inherited(chain, |p| (p.tracking.as_ref(), None)).map(|x| f64::from(*x))
        && tracking != 0.0
        && !tracking.is_nan()
    {
        props.letter_spacing = Some(text_pt(tracking * size));
        features.push(FeatureId::TextLetterSpacing);
    }
    if let Some(background) = color(inherited(chain, |p| {
        (p.background_color.as_ref(), p.background_color_null)
    })) && background.a.steps() > 0
    {
        props.highlight = Some(Some(Color::Rgb(background)));
        features.push(FeatureId::TextHighlight);
    }
    match shadow(inherited(chain, |p| (p.shadow.as_ref(), p.shadow_null))) {
        // Nothing under a run holds a shadow (the defaults and list styles state none), so none is left unstated.
        ShadowRead::Off => {}
        ShadowRead::On(shadow) => {
            props.shadow = Some(Some(shadow));
            features.push(FeatureId::TextShadow);
        }
        ShadowRead::NotDrawn(reason) => reports.push(Report {
            code: codes::KN1304,
            feature: Some(FeatureId::TextShadow),
            style: None,
            reason: Some(reason),
        }),
    }
    if inherited(chain, |p| (p.outline.as_ref(), None)).is_some_and(|x| f64::from(*x) > 0.0) {
        approximated(&mut reports, "outline", None);
    }
    // A gradient or image text fill (`tsd_fill`) is read for presence only: the text keeps its colour.
    if let Some(text_fill) = inherited(chain, |p| (p.tsd_fill.as_ref(), p.tsd_fill_null)) {
        if let Some(gradient) = &text_fill.gradient {
            let kind = if gradient.r#type == Some(1) {
                "radial"
            } else {
                "linear"
            };
            reports.push(Report {
                code: codes::KN1301,
                feature: None,
                style: None,
                reason: Some(format!("a text fill ({kind})")),
            });
        } else if text_fill.image.is_some() && text_fill.color.is_none() {
            approximated(&mut reports, "an image fill", None);
        }
    }
    let exact =
        !(bold == Some(true) && face.weight < 700) && !(italic == Some(true) && !face.italic);
    RunStyle {
        props,
        size,
        postscript_name: face.postscript_name,
        named: font_name.is_some(),
        exact,
        features,
        reports,
    }
}

// TSWP.ListStyleArchive.LabelType
const LABEL_IMAGE: i32 = 1;
const LABEL_STRING: i32 = 2;
const LABEL_NUMBER: i32 = 3;
const IMAGE_BULLET: &str = "\u{2022}";
/// TSWP.ListStyleArchive.NumberType 0–14: (decimal, upper/lower roman, upper/lower alpha) × (`1.`, `(1)`, `1)`).
const NUMBER_SCHEMES: [slidock_ir::text::AutoNumScheme; 15] = {
    use slidock_ir::text::AutoNumScheme::*;
    [
        ArabicPeriod,
        ArabicParenBoth,
        ArabicParenR,
        RomanUcPeriod,
        RomanUcParenBoth,
        RomanUcParenR,
        RomanLcPeriod,
        RomanLcParenBoth,
        RomanLcParenR,
        AlphaUcPeriod,
        AlphaUcParenBoth,
        AlphaUcParenR,
        AlphaLcPeriod,
        AlphaLcParenBoth,
        AlphaLcParenR,
    ]
};

struct ListPlace {
    bullet: Bullet,
    indent: f64,
    label_width: f64,
}

/// The `level`th value of a list style's per-level array, the array of the first style in the chain that sets it.
fn at_level<'p, T>(
    chain: &[&'p tswp::ListStyleArchive],
    level: usize,
    get: impl Fn(&'p tswp::ListStyleArchive) -> &'p Vec<T>,
) -> Option<&'p T> {
    inherited(chain, |style| {
        let values = get(style);
        ((!values.is_empty()).then_some(values), None)
    })
    .and_then(|values| values.get(level))
}

/// A paragraph's list label: none, a string, or a number counted by number-lists. The label hangs in a space of
/// `text_indents` ems of the paragraph's first run, after an indent of `indents` points. A label's own shadow is not
/// drawn (KN1304).
fn list_place(
    ctx: &mut Lowering<'_>,
    reference: Option<&tsp::Reference>,
    level: usize,
    first_size: f64,
    node: &NodeId,
) -> ListPlace {
    let chain = style_chain::<ListStyle>(ctx.graph, reference);
    let label = at_level(&chain, level, |s| &s.label_types)
        .copied()
        .unwrap_or(0);
    if !matches!(label, LABEL_IMAGE | LABEL_STRING | LABEL_NUMBER) {
        return ListPlace {
            bullet: Bullet::None {},
            indent: 0.0,
            label_width: 0.0,
        };
    }
    if shadow(inherited(&chain, |s| (s.shadow.as_ref(), s.shadow_null))) != ShadowRead::Off {
        ctx.report(
            codes::KN1304,
            DiagnosticSite::new()
                .node(node.clone())
                .feature(FeatureId::TextShadow)
                .data("reason", "bullet shadow"),
        );
    }
    let indent = at_level(&chain, level, |s| &s.indents).map_or(0.0, |x| f64::from(*x));
    let label_width =
        at_level(&chain, level, |s| &s.text_indents).map_or(0.0, |x| f64::from(*x)) * first_size;
    if label == LABEL_NUMBER {
        ctx.see(FeatureId::TextBulletAuto);
        let number_type = at_level(&chain, level, |s| &s.number_types)
            .copied()
            .unwrap_or(0);
        let scheme = usize::try_from(number_type)
            .ok()
            .and_then(|i| NUMBER_SCHEMES.get(i))
            .copied();
        if scheme.is_none() {
            ctx.report(
                codes::KN1208,
                DiagnosticSite::new()
                    .node(node.clone())
                    .feature(FeatureId::TextBulletAuto)
                    .data("type", f64::from(number_type)),
            );
        }
        return ListPlace {
            bullet: Bullet::Auto {
                scheme: scheme.unwrap_or(slidock_ir::text::AutoNumScheme::ArabicPeriod),
                start_at: 1,
                font: None,
                color: None,
                size: None,
            },
            indent,
            label_width,
        };
    }
    if label == LABEL_IMAGE {
        ctx.see(FeatureId::TextBulletImage);
        ctx.report(
            codes::KN1203,
            DiagnosticSite::new()
                .node(node.clone())
                .feature(FeatureId::TextBulletImage),
        );
    } else {
        ctx.see(FeatureId::TextBulletChar);
    }
    let string = if label == LABEL_STRING {
        at_level(&chain, level, |s| &s.strings).filter(|s| !s.is_empty() && s.chars().count() <= 8)
    } else {
        None
    };
    ListPlace {
        bullet: Bullet::Char {
            char: string.map_or_else(|| IMAGE_BULLET.to_owned(), String::clone),
            font: None,
            color: None,
            size: None,
        },
        indent,
        label_width,
    }
}

fn line_spacing(
    archive: Option<&tswp::LineSpacingArchive>,
    ctx: &mut Lowering<'_>,
    node: &NodeId,
) -> Option<LineSpacing> {
    let archive = archive?;
    let amount = wide(archive.amount)?;
    let points = text_size(js_max(0.0, amount));
    let mode = archive.mode.unwrap_or(0);
    let spacing = match mode {
        0 => LineSpacing::Multiple {
            value: ratio(js_max(0.0, amount)),
            basis: KEYNOTE_LINE_BASIS,
        },
        1 => LineSpacing::AtLeast { value: points },
        2 => LineSpacing::Exact { value: points },
        3 => LineSpacing::AtMost { value: points },
        _ => {
            // "Space between lines" adds to the natural height, which the IR's spacing kinds cannot say.
            ctx.report(
                codes::KN1209,
                DiagnosticSite::new()
                    .node(node.clone())
                    .feature(FeatureId::TextLineSpacing)
                    .data("mode", f64::from(mode)),
            );
            single_spacing()
        }
    };
    if spacing != single_spacing() {
        ctx.see(FeatureId::TextLineSpacing);
    }
    Some(spacing)
}

/// The PostScript names text uses: every name a run asks for (the presentation's font requests), and the names of
/// runs that draw visible characters in printed text, which an export of the presentation must embed (the bench's
/// oracle check).
#[derive(Debug)]
pub struct FontUses<'a> {
    pub requested: &'a mut BTreeSet<String>,
    pub drawn: Option<&'a mut BTreeSet<String>>,
}

type StyleKey = (Option<u64>, Option<u64>);

struct Reading<'f, 'u> {
    node: &'f NodeId,
    run_styles: BTreeMap<StyleKey, RunStyle>,
    /// The styles an inline drew with: their features are the text's, their losses reported once.
    used: BTreeSet<StyleKey>,
    fonts: &'f mut FontUses<'u>,
}

impl Reading<'_, '_> {
    fn style(
        &mut self,
        ctx: &Lowering<'_>,
        char_ref: Option<&tsp::Reference>,
        para_ref: Option<&tsp::Reference>,
    ) -> StyleKey {
        let key = (
            referenced(char_ref).map(|id| id.0),
            referenced(para_ref).map(|id| id.0),
        );
        self.run_styles
            .entry(key)
            .or_insert_with(|| run_style(ctx, char_ref, para_ref));
        key
    }

    fn get(&self, key: StyleKey) -> Option<&RunStyle> {
        self.run_styles.get(&key)
    }

    /// Records that an inline draws with the style.
    fn use_style(&mut self, ctx: &mut Lowering<'_>, key: StyleKey) {
        if !self.used.insert(key) {
            return;
        }
        let Some(style) = self.run_styles.get(&key) else {
            return;
        };
        for feature in &style.features {
            ctx.see(*feature);
        }
        for report in &style.reports {
            let mut site = DiagnosticSite::new().node(self.node.clone());
            if let Some(feature) = report.feature {
                site = site.feature(feature);
            }
            if let Some(style) = report.style {
                site = site.data("style", style);
            }
            if let Some(reason) = &report.reason {
                site = site.data("reason", reason.as_str());
            }
            ctx.report(report.code, site);
        }
        if style.named {
            self.fonts.requested.insert(style.postscript_name.clone());
        }
    }

    /// Records the face of the characters an inline draws, when its PostScript name is known.
    fn drawn(&mut self, key: StyleKey, text: &str) {
        let Some(style) = self.run_styles.get(&key) else {
            return;
        };
        if style.named
            && style.exact
            && text.chars().any(|c| !slidock_canon::is_js_whitespace(c))
            && let Some(drawn) = self.fonts.drawn.as_deref_mut()
        {
            drawn.insert(style.postscript_name.clone());
        }
    }
}

/// UTF-16 code units as text; a half of a surrogate pair cut apart is dropped.
fn text_of(units: &[u16]) -> String {
    char::decode_utf16(units.iter().copied())
        .filter_map(Result::ok)
        .collect()
}

/// A storage's text.
pub fn storage_text(storage: &tswp::StorageArchive) -> String {
    storage.text.concat()
}

/// A number property along a chain, widened.
fn number<'p, P>(chain: &[&'p P], get: impl Fn(&'p P) -> Option<&'p f32>) -> Option<f64> {
    inherited(chain, |p| (get(p), None)).map(|x| f64::from(*x))
}

/// The paragraphs of a storage; an empty text is one empty paragraph. `natural` is where natural alignment puts
/// left-to-right text, mirrored in right-to-left text: the left, but the right for a number in a table cell.
pub fn read_paragraphs(
    ctx: &mut Lowering<'_>,
    storage: &tswp::StorageArchive,
    node: &NodeId,
    fonts: &mut FontUses<'_>,
    natural: Align,
) -> Result<Vec<SourceParagraph>, LowerError> {
    let text = storage_text(storage);
    let units: Vec<u16> = text.encode_utf16().collect();
    if units.len() > ctx.limits.text_per_body {
        return Err(LowerError::Limit(format!(
            "A text holds {} characters",
            units.len()
        )));
    }
    let mut unsorted = false;
    let mut para_styles = Cursor::new(sorted(
        object_entries(storage.table_para_style.as_ref()),
        &mut unsorted,
    ));
    let mut char_styles = Cursor::new(sorted(
        object_entries(storage.table_char_style.as_ref()),
        &mut unsorted,
    ));
    let mut links = Cursor::new(sorted(
        object_entries(storage.table_smartfield.as_ref()),
        &mut unsorted,
    ));
    let mut attachments = Cursor::new(sorted(
        object_entries(storage.table_attachment.as_ref()),
        &mut unsorted,
    ));
    let mut lists = Cursor::new(sorted(
        object_entries(storage.table_list_style.as_ref()),
        &mut unsorted,
    ));
    let mut levels = Cursor::new(sorted(
        storage
            .table_para_data
            .as_ref()
            .map(|table| table.entries.as_slice())
            .unwrap_or_default(),
        &mut unsorted,
    ));
    if unsorted {
        ctx.report(codes::KN1202, DiagnosticSite::new().node(node.clone()));
    }
    let graph = ctx.graph;
    let mut reading = Reading {
        node,
        run_styles: BTreeMap::new(),
        used: BTreeSet::new(),
        fonts,
    };
    let slide_number = ctx.slide_number.to_string();

    let mut paragraphs: Vec<SourceParagraph> = Vec::new();
    let mut start = 0;
    loop {
        let mut end = start;
        while units
            .get(end)
            .is_some_and(|unit| !is_paragraph_separator(*unit))
        {
            end += 1;
        }
        let para_ref = para_styles.style(start);
        let para_chain = style_chain::<ParagraphStyle>(graph, para_ref.as_ref());
        let first_char = char_styles.object(start);
        let first = reading.style(ctx, first_char.as_ref(), para_ref.as_ref());

        let mut inlines: Vec<slidock_ir::text::SourceInline> = Vec::new();
        let mut tabs = false;
        let mut i = start;
        while i < end {
            let char_entry = char_styles.at(i);
            let link_entry = links.at(i);
            let char_ref = char_styles
                .entry(char_entry)
                .and_then(|e| e.object().cloned());
            let style = reading.style(ctx, char_ref.as_ref(), para_ref.as_ref());
            let link_ref = links.entry(link_entry).and_then(|e| e.object().cloned());
            let url = graph
                .get::<tswp::HyperlinkFieldArchive>(link_ref.as_ref())
                .and_then(|field| field.url_ref.as_deref())
                .filter(|url| !url.is_empty());
            let mut props = reading
                .get(style)
                .map(|style| style.props.clone())
                .unwrap_or_default();
            if let Some(url) = url {
                props.link = Some(Some(Link::Url {
                    url: utf16::clip(url, 2048).to_owned(),
                }));
            }
            // The style's features are the text's once an inline draws with it.
            let drew = |reading: &mut Reading<'_, '_>, ctx: &mut Lowering<'_>| {
                reading.use_style(ctx, style);
                if url.is_some() {
                    ctx.see(FeatureId::TextLink);
                }
            };
            let mut chunk: Vec<u16> = Vec::new();
            let flush = |chunk: &mut Vec<u16>,
                         inlines: &mut Vec<slidock_ir::text::SourceInline>,
                         reading: &mut Reading<'_, '_>,
                         ctx: &mut Lowering<'_>| {
                if chunk.is_empty() {
                    return;
                }
                let text = text_of(chunk);
                chunk.clear();
                ctx.see(FeatureId::TextRun);
                drew(reading, ctx);
                reading.drawn(style, &text);
                inlines.push(Inline::Run {
                    text,
                    props: props.clone(),
                    line_height: None,
                    leading_skew: None,
                });
            };
            let mut j = i;
            while j < end && char_styles.at(j) == char_entry && links.at(j) == link_entry {
                let Some(&unit) = units.get(j) else {
                    break;
                };
                match unit {
                    LINE_SEPARATOR => {
                        flush(&mut chunk, &mut inlines, &mut reading, ctx);
                        inlines.push(Inline::Break {
                            props: Some(props.clone()),
                        });
                        ctx.see(FeatureId::TextBreak);
                        drew(&mut reading, ctx);
                    }
                    TAB => {
                        flush(&mut chunk, &mut inlines, &mut reading, ctx);
                        inlines.push(Inline::Tab {});
                        ctx.see(FeatureId::TextTab);
                        tabs = true;
                    }
                    OBJECT_REPLACEMENT => {
                        flush(&mut chunk, &mut inlines, &mut reading, ctx);
                        let attachment = attachments
                            .exactly(j)
                            .and_then(|entry| entry.object().cloned());
                        let attachment = graph.type_name(attachment.as_ref());
                        if attachment.is_some_and(|name| SLIDE_NUMBER_ATTACHMENTS.contains(&name)) {
                            inlines.push(Inline::Field {
                                field: FieldKind::SlideNumber,
                                cached: slide_number.clone(),
                                props: props.clone(),
                                line_height: None,
                                leading_skew: None,
                            });
                            ctx.see(FeatureId::TextField);
                            drew(&mut reading, ctx);
                            reading.drawn(style, &slide_number);
                        } else {
                            ctx.report(
                                codes::KN1201,
                                DiagnosticSite::new()
                                    .node(node.clone())
                                    .data("attachment", attachment.unwrap_or("missing")),
                            );
                        }
                    }
                    _ => chunk.push(unit),
                }
                j += 1;
            }
            flush(&mut chunk, &mut inlines, &mut reading, ctx);
            i = j;
        }

        let list_ref = lists.object(start).or_else(|| {
            inherited(&para_chain, |p| (p.list_style.as_ref(), p.list_style_null)).cloned()
        });
        let level_entry = levels.at(start);
        let level = levels
            .entry(level_entry)
            .and_then(|entry| entry.first)
            .unwrap_or(0)
            .min(8);
        let first_size = reading.get(first).map_or(DEFAULT_SIZE, |style| style.size);
        let list = list_place(ctx, list_ref.as_ref(), level as usize, first_size, node);
        let rtl = inherited(&para_chain, |p| (p.writing_direction.as_ref(), None)) == Some(&1);
        if rtl {
            ctx.see(FeatureId::TextRtl);
        }
        let alignment = inherited(&para_chain, |p| (p.alignment.as_ref(), None))
            .copied()
            .unwrap_or(NATURAL);
        let spacing = line_spacing(
            inherited(&para_chain, |p| {
                (p.line_spacing.as_ref(), p.line_spacing_null)
            }),
            ctx,
            node,
        );
        let tab_stops = inherited(&para_chain, |p| (p.tabs.as_ref(), p.tabs_null))
            .map_or(0, |tabs| tabs.tabs.len());
        if tabs && tab_stops > 0 {
            ctx.report(
                codes::KN1204,
                DiagnosticSite::new()
                    .node(node.clone())
                    .feature(FeatureId::TextTab),
            );
        }
        let left_indent = number(&para_chain, |p| p.left_indent.as_ref()).unwrap_or(0.0);
        let first_line_indent =
            number(&para_chain, |p| p.first_line_indent.as_ref()).unwrap_or(0.0);
        let align = if alignment == NATURAL {
            match (rtl, natural) {
                (true, Align::Left) => Align::Right,
                (true, Align::Right) => Align::Left,
                (_, natural) => natural,
            }
        } else {
            usize::try_from(alignment)
                .ok()
                .and_then(|i| ALIGNMENTS.get(i))
                .copied()
                .unwrap_or(Align::Left)
        };
        let has_bullet = !matches!(list.bullet, Bullet::None {});
        let space_before = if paragraphs.is_empty() {
            Pt::ZERO
        } else {
            text_pt(js_max(
                0.0,
                number(&para_chain, |p| p.space_before.as_ref()).unwrap_or(0.0),
            ))
        };
        paragraphs.push(SourceParagraph {
            level,
            props: PartialParaProps {
                align: Some(align),
                margin_left: Some(text_pt(left_indent + list.indent + list.label_width)),
                margin_right: Some(text_pt(
                    number(&para_chain, |p| p.right_indent.as_ref()).unwrap_or(0.0),
                )),
                indent: Some(text_pt(if has_bullet {
                    -list.label_width
                } else {
                    first_line_indent
                })),
                // Keynote leaves out the space before a text's first paragraph.
                space_before: Some(Spacing::Pt {
                    value: space_before,
                }),
                space_after: Some(Spacing::Pt {
                    value: text_pt(js_max(
                        0.0,
                        number(&para_chain, |p| p.space_after.as_ref()).unwrap_or(0.0),
                    )),
                }),
                line_spacing: spacing,
                tab_size: Some(text_size(js_max(
                    0.0,
                    number(&para_chain, |p| p.default_tab_stops.as_ref()).unwrap_or(DEFAULT_TAB),
                ))),
                bullet: Some(list.bullet),
                rtl: Some(rtl),
                ..PartialParaProps::default()
            },
            inlines,
            // An empty paragraph keeps its line at the size of its paragraph's text.
            end_props: Some(
                reading
                    .get(first)
                    .map(|style| style.props.clone())
                    .unwrap_or_default(),
            ),
        });
        // Text that ends with a paragraph separator ends with an empty line, which Keynote lays out.
        if end >= units.len() {
            break;
        }
        start = end + 1;
    }
    Ok(paragraphs)
}

/// The box's frame in its container's coordinates, which place the anchor a text box Keynote sizes to its text grows
/// from, and where those coordinates land on the slide.
#[derive(Clone, Copy, Debug)]
pub struct BoxPlace {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub to_slide: SlideMap,
}

/// A shape's text with its text-box properties, or `None` when it holds none. A box Keynote sizes to its text keeps
/// its anchor (the first paragraph's alignment across, its vertical alignment down) and, growing in width, wraps at
/// the slide's edge on the side it grows toward, or at the nearer edge on both sides when centred. The anchor is
/// measured on the slide: a box in a group wraps at the slide's edges, not its group's. A box anchored off the slide,
/// past the edge it grows toward, has no edge ahead of it and is not capped: Keynote draws it on the lines its text
/// takes on the slide, as a Magic Move that brings it in shows.
#[allow(
    clippy::too_many_arguments,
    reason = "the parts of a text box, each its own input"
)]
pub fn text_body(
    ctx: &mut Lowering<'_>,
    storage_ref: Option<&tsp::Reference>,
    style_ref: Option<&tsp::Reference>,
    geometry_flags: Option<u32>,
    place: BoxPlace,
    node: &NodeId,
    fonts: &mut FontUses<'_>,
) -> Result<Option<TextBody>, LowerError> {
    let graph = ctx.graph;
    let Some(storage) = graph.get::<tswp::StorageArchive>(storage_ref) else {
        return Ok(None);
    };
    if storage.text.iter().all(String::is_empty) {
        return Ok(None);
    }
    let paragraphs = read_paragraphs(ctx, storage, node, fonts, Align::Left)?;
    let shape = style_chain::<TextShapeStyle>(graph, style_ref);
    let padding = inherited(&shape, |p| (p.padding.as_ref(), p.padding_null));
    let no_padding = padding.is_none() && shape.iter().any(|p| p.padding_null == Some(true));
    let side = |value: Option<f32>| {
        text_pt(js_max(
            0.0,
            wide(value).unwrap_or(if no_padding { 0.0 } else { DEFAULT_PADDING }),
        ))
    };
    let flags = geometry_flags.unwrap_or(WIDTH_VALID | HEIGHT_VALID);
    let grow_width = flags & WIDTH_VALID == 0;
    let grow_height = flags & HEIGHT_VALID == 0;
    let mut autofit = Autofit::None {};
    let mut max_width = None;
    if inherited(&shape, |p| (p.shrink_to_fit.as_ref(), None)) == Some(&true) {
        ctx.see(FeatureId::TextAutofitShrink);
        // Keynote stores no scale: it fits the text at a whole percent of its size.
        autofit = Autofit::Shrink {
            font_scale: None,
            line_spacing_reduction: None,
            fitted: None,
            steps: Some(ShrinkSteps::Percent),
        };
    } else if grow_width || grow_height {
        ctx.see(FeatureId::TextAutofitResize);
        let anchor_x = match paragraphs.first().and_then(|p| p.props.align) {
            Some(Align::Center) => AnchorX::Center,
            Some(Align::Right) => AnchorX::Right,
            _ => AnchorX::Left,
        };
        autofit = Autofit::Resize {
            width: grow_width,
            height: grow_height,
            anchor_x,
        };
        if grow_width {
            let fraction = match anchor_x {
                AnchorX::Left => 0.0,
                AnchorX::Right => 1.0,
                AnchorX::Center => 0.5,
            };
            let at = place
                .to_slide
                .x(place.x + place.w * fraction, place.y + place.h / 2.0);
            let limit = match anchor_x {
                AnchorX::Left => ctx.slide_width - at,
                AnchorX::Right => at,
                AnchorX::Center => 2.0 * js_min(at, ctx.slide_width - at),
            };
            max_width = (limit > 0.0).then(|| text_size(limit));
        }
    }
    if inherited(&shape, |p| (p.vertical_text.as_ref(), None)) == Some(&true) {
        ctx.see(FeatureId::TextVertical);
        ctx.report(
            codes::KN1206,
            DiagnosticSite::new()
                .node(node.clone())
                .feature(FeatureId::TextVertical),
        );
    }
    Ok(Some(TextBody {
        body: PartialBodyProps {
            insets: Some(PartialInsets {
                l: Some(side(padding.and_then(|p| p.left))),
                t: Some(side(padding.and_then(|p| p.top))),
                r: Some(side(padding.and_then(|p| p.right))),
                b: Some(side(padding.and_then(|p| p.bottom))),
            }),
            anchor: Some(vertical(
                inherited(&shape, |p| (p.vertical_alignment.as_ref(), None)).copied(),
            )),
            wrap: Some(Wrap::Square),
            autofit: Some(autofit),
            overflow: None,
            max_width,
        },
        list_style: None,
        paragraphs,
        // The drawable's flips mirror its text with it.
        flips_with_frame: true,
        rotation: None,
    }))
}

/// A slide's speaker notes as rich text, or `None` when it has none. The notes' text features stay out of the
/// slide's: the census counts notes as `slide.notes` alone.
pub fn notes_body(
    ctx: &mut Lowering<'_>,
    note_ref: Option<&tsp::Reference>,
    node: &NodeId,
) -> Result<Option<TextBody>, LowerError> {
    let graph = ctx.graph;
    let Some(storage) = graph
        .get::<kn::NoteArchive>(note_ref)
        .and_then(|note| graph.get::<tswp::StorageArchive>(note.contained_storage.as_ref()))
    else {
        return Ok(None);
    };
    if js_trim(&storage_text(storage)).is_empty() {
        return Ok(None);
    }
    let seen = std::mem::take(&mut ctx.reporter.seen);
    let mut requested = BTreeSet::new();
    let mut fonts = FontUses {
        requested: &mut requested,
        drawn: None,
    };
    let paragraphs = read_paragraphs(ctx, storage, node, &mut fonts, Align::Left);
    ctx.reporter.seen = seen;
    let paragraphs = paragraphs?;
    ctx.see(FeatureId::SlideNotes);
    Ok(Some(TextBody {
        body: PartialBodyProps::default(),
        list_style: None,
        paragraphs,
        flips_with_frame: false,
        rotation: None,
    }))
}
