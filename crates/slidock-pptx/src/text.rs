//! DrawingML text: paragraph properties (`a:pPr`, `a:lvlNpPr`), run properties (`a:rPr`, `a:defRPr`), list styles
//! (levels 1–9 → `"0"`–`"8"`, `a:defPPr` under each) and text bodies (body properties, paragraphs of runs, breaks, tabs
//! and fields). Everything a source declares is kept partial: the cascade (presentation defaults → master text styles →
//! list styles → paragraph → run) is cascade-text's.

use slidock_canon::utf16;
use slidock_core_api::lowering::LowerError;
use slidock_ir::color::{Color, MappedSlot};
use slidock_ir::diagnostics::{DataValue, DiagnosticCode, DiagnosticSite, codes};
use slidock_ir::features::FeatureId;
use slidock_ir::text::{
    Align, Anchor, AnchorX, AutoNumScheme, Autofit, Bullet, BulletSize, Caps, Decoration,
    FamilyFont, FieldKind, FontRef, Inline, LevelStyle, LevelStyles, LineBasis, LineSpacing, Link,
    MAX_LEVEL, MAX_STRING, OrText, PartialBodyProps, PartialInsets, PartialParaProps,
    PartialRunProps, ShrinkSteps, SourceInline, SourceParagraph, Spacing, TextBody, ThemeFont,
    ThemeFontSlot, Wrap,
};
use slidock_ir::units::Size;

use crate::clip;
use crate::color::{color_element, plain_color, read_color, rgb, theme_color};
use crate::report::Report;
use crate::shadow::{run_has_shadow, run_shadow};
use crate::units::{centi_pt, centi_size, emu_to_pt, emu_to_size, percent, percent_unit, ratio};
use crate::xml::{El, Element, Ns};

/// How text readers report and read links, and what a text body adds: the features its source holds, and the
/// per-body character limit.
pub(crate) trait TextCx: Report {
    /// A hyperlink element as an IR link, when the IR models its target.
    fn link(&mut self, _hlink: &Element) -> Option<Link> {
        None
    }

    fn seen(&mut self, _feature: FeatureId) {}

    fn text_limit(&self) -> usize {
        usize::MAX
    }
}

/// A report with only data.
pub(crate) fn data_site(data: &[(&'static str, DataValue)]) -> DiagnosticSite {
    data.iter()
        .fold(DiagnosticSite::new(), |site, (key, value)| {
            site.data(*key, value.clone())
        })
}

fn align(name: &str) -> Option<Align> {
    match name {
        "l" => Some(Align::Left),
        "ctr" => Some(Align::Center),
        "r" => Some(Align::Right),
        "just" | "justLow" | "dist" | "thaiDist" => Some(Align::Justify),
        _ => None,
    }
}

pub(crate) fn theme_font(slot: ThemeFontSlot) -> FontRef {
    FontRef::Theme(ThemeFont {
        kind: slidock_ir::color::ThemeTag,
        slot,
    })
}

/// `+mj-lt` and `+mn-lt` (and their East Asian and complex-script kin) name the theme's fonts.
pub(crate) fn font_ref(typeface: Option<&str>) -> Option<FontRef> {
    let typeface = typeface.filter(|face| !face.is_empty())?;
    let script = |prefix: &str| {
        typeface
            .strip_prefix(prefix)
            .is_some_and(|rest| matches!(rest, "lt" | "ea" | "cs"))
    };
    Some(if script("+mj-") {
        theme_font(ThemeFontSlot::Major)
    } else if script("+mn-") {
        theme_font(ThemeFontSlot::Minor)
    } else {
        FontRef::Family(FamilyFont::new(clip(typeface, 256)))
    })
}

fn color<'a>(parent: impl El<'a>, cx: &mut dyn Report) -> Option<Color> {
    color_element(parent).map(|element| plain_color(read_color(element, cx)))
}

fn line_spacing(element: Option<&Element>) -> Option<LineSpacing> {
    if let Some(pct) = element.child(Ns::A, "spcPct").int_attr("val") {
        return Some(LineSpacing::Multiple {
            value: percent(pct.max(0)),
            basis: LineBasis::Size1_2,
        });
    }
    let pts = element.child(Ns::A, "spcPts").int_attr("val")?;
    Some(LineSpacing::Exact {
        value: centi_size(pts.max(0)),
    })
}

fn spacing(element: Option<&Element>) -> Option<Spacing> {
    if let Some(pct) = element.child(Ns::A, "spcPct").int_attr("val") {
        return Some(Spacing::Pct {
            value: percent(pct.max(0)),
        });
    }
    let pts = element.child(Ns::A, "spcPts").int_attr("val")?;
    Some(Spacing::Pt {
        value: centi_pt(pts),
    })
}

fn auto_num_scheme(name: &str) -> Option<AutoNumScheme> {
    Some(match name {
        "arabicPeriod" => AutoNumScheme::ArabicPeriod,
        "arabicParenR" => AutoNumScheme::ArabicParenR,
        "arabicParenBoth" => AutoNumScheme::ArabicParenBoth,
        "arabicPlain" => AutoNumScheme::ArabicPlain,
        "alphaLcPeriod" => AutoNumScheme::AlphaLcPeriod,
        "alphaUcPeriod" => AutoNumScheme::AlphaUcPeriod,
        "alphaLcParenR" => AutoNumScheme::AlphaLcParenR,
        "alphaUcParenR" => AutoNumScheme::AlphaUcParenR,
        "alphaLcParenBoth" => AutoNumScheme::AlphaLcParenBoth,
        "alphaUcParenBoth" => AutoNumScheme::AlphaUcParenBoth,
        "romanLcPeriod" => AutoNumScheme::RomanLcPeriod,
        "romanUcPeriod" => AutoNumScheme::RomanUcPeriod,
        "romanLcParenR" => AutoNumScheme::RomanLcParenR,
        "romanUcParenR" => AutoNumScheme::RomanUcParenR,
        "romanLcParenBoth" => AutoNumScheme::RomanLcParenBoth,
        "romanUcParenBoth" => AutoNumScheme::RomanUcParenBoth,
        _ => return None,
    })
}

/// Bullet properties are separate elements in DrawingML; the IR's bullet holds them with its type.
fn bullet(p_pr: &Element, cx: &mut dyn TextCx) -> Option<Bullet> {
    let mut font: Option<OrText<FontRef>> = None;
    let mut fill: Option<OrText<Color>> = None;
    let mut size: Option<BulletSize> = None;
    let mut kind: Option<&Element> = None;
    for element in p_pr.kids() {
        match &*element.local {
            "buFontTx" => font = Some(OrText::Text),
            "buFont" => font = font_ref(element.attr("typeface")).map(OrText::Value),
            "buClrTx" => fill = Some(OrText::Text),
            "buClr" => fill = color(&**element, cx).map(OrText::Value),
            "buSzTx" => size = Some(BulletSize::Text {}),
            "buSzPct" => {
                size = Some(BulletSize::Pct {
                    value: percent(element.int_attr("val").unwrap_or(100_000).max(0)),
                })
            }
            "buSzPts" => {
                size = Some(BulletSize::Pt {
                    value: centi_size(element.int_attr("val").unwrap_or(0).max(0)),
                })
            }
            "buNone" | "buChar" | "buAutoNum" | "buBlip" => kind = Some(element),
            _ => {}
        }
    }
    let Some(kind) = kind else {
        // Properties alone would restyle an inherited bullet; the IR sets a bullet's style together with its type.
        let named: Vec<&str> = [
            font.as_ref().map(|_| "font"),
            fill.as_ref().map(|_| "color"),
            size.as_ref().map(|_| "size"),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !named.is_empty() {
            cx.report(
                codes::PX2209,
                data_site(&[("properties", named.join(", ").into())]),
            );
        }
        return None;
    };
    Some(match &*kind.local {
        "buNone" => Bullet::None {},
        "buChar" => {
            let written = kind.attr("char").filter(|c| !c.is_empty()).unwrap_or("•");
            Bullet::Char {
                char: written.chars().take(1).collect(),
                font,
                color: fill,
                size,
            }
        }
        "buAutoNum" => {
            let written = kind.attr("type").unwrap_or("arabicPeriod");
            let scheme = auto_num_scheme(written);
            if scheme.is_none() {
                cx.report(codes::PX2208, data_site(&[("scheme", written.into())]));
            }
            let start_at = kind.int_attr("startAt").unwrap_or(1).clamp(1, 32_767);
            Bullet::Auto {
                scheme: scheme.unwrap_or(AutoNumScheme::ArabicPeriod),
                start_at: u32::try_from(start_at).unwrap_or(1),
                font,
                color: fill,
                size,
            }
        }
        _ => {
            cx.report(codes::PX2204, DiagnosticSite::new());
            Bullet::Char {
                char: "•".to_owned(),
                font,
                color: fill,
                size,
            }
        }
    })
}

/// A paragraph's or a level's properties, as declared.
pub(crate) fn read_para_props(p_pr: Option<&Element>, cx: &mut dyn TextCx) -> PartialParaProps {
    let mut out = PartialParaProps::default();
    let Some(p_pr) = p_pr else { return out };
    out.align = align(p_pr.attr("algn").unwrap_or(""));
    out.margin_left = p_pr.int_attr("marL").map(emu_to_pt);
    out.margin_right = p_pr.int_attr("marR").map(emu_to_pt);
    out.indent = p_pr.int_attr("indent").map(emu_to_pt);
    out.rtl = Some(p_pr).bool_attr("rtl");
    out.tab_size = p_pr.int_attr("defTabSz").map(emu_to_size);
    out.line_spacing = line_spacing(p_pr.child(Ns::A, "lnSpc").map(|rc| &**rc));
    out.space_before = spacing(p_pr.child(Ns::A, "spcBef").map(|rc| &**rc));
    out.space_after = spacing(p_pr.child(Ns::A, "spcAft").map(|rc| &**rc));
    out.bullet = bullet(p_pr, cx);
    if !p_pr.child(Ns::A, "tabLst").kids().is_empty() {
        cx.report(codes::PX2205, DiagnosticSite::new());
    }
    out
}

fn underline(name: &str) -> Option<Decoration> {
    match name {
        "none" => Some(Decoration::None),
        "sng" => Some(Decoration::Single),
        "dbl" => Some(Decoration::Double),
        _ => None,
    }
}

fn strike(name: &str) -> Option<Decoration> {
    match name {
        "noStrike" => Some(Decoration::None),
        "sngStrike" => Some(Decoration::Single),
        "dblStrike" => Some(Decoration::Double),
        _ => None,
    }
}

fn caps(name: &str) -> Option<Caps> {
    match name {
        "none" => Some(Caps::None),
        "small" => Some(Caps::Small),
        "all" => Some(Caps::All),
        _ => None,
    }
}

const FILLS: [&str; 6] = [
    "noFill",
    "solidFill",
    "gradFill",
    "pattFill",
    "blipFill",
    "grpFill",
];

/// The text colour of a run: a solid fill, transparent for `noFill`, a gradient or pattern by its first colour.
fn run_color(r_pr: &Element, cx: &mut dyn TextCx) -> Option<Color> {
    let fill = r_pr
        .kids()
        .find(|kid| kid.ns == Ns::A && FILLS.contains(&&*kid.local))?;
    match &*fill.local {
        "solidFill" => return color(&**fill, cx),
        "noFill" => return Some(Color::Rgb(rgb(0.0, 0.0, 0.0, 0.0))),
        _ => {}
    }
    let name = match &*fill.local {
        "gradFill" => "gradient",
        "pattFill" => "pattern",
        "blipFill" => "picture",
        "grpFill" => "group",
        other => other,
    };
    cx.report(codes::PX2211, data_site(&[("fill", name.into())]));
    match &*fill.local {
        "gradFill" => color(fill.child(Ns::A, "gsLst").child(Ns::A, "gs"), cx),
        "pattFill" => color(fill.child(Ns::A, "fgClr"), cx),
        _ => None,
    }
}

/// An underline filled with `a:uFill/a:noFill` is invisible.
fn invisible_underline(r_pr: &Element) -> bool {
    r_pr.child(Ns::A, "uFill").child(Ns::A, "noFill").is_some()
}

fn keeps_text_color(element: &Element) -> bool {
    element.kids().any(|item| {
        (&*item.local == "hlinkClr" && item.attr("val") == Some("tx")) || keeps_text_color(item)
    })
}

/// A run's (or a level's default) properties, as declared.
pub(crate) fn read_run_props(r_pr: Option<&Element>, cx: &mut dyn TextCx) -> PartialRunProps {
    let mut out = PartialRunProps::default();
    let Some(r_pr) = r_pr else { return out };
    if let Some(size) = r_pr.int_attr("sz") {
        let points = centi_size(size.max(0));
        out.size = Some(points.min(Size::from_steps(4_000_000)));
    }
    out.weight = Some(r_pr)
        .bool_attr("b")
        .map(|bold| if bold { 700 } else { 400 });
    out.italic = Some(r_pr).bool_attr("i");
    if let Some(written) = r_pr.attr("u") {
        let style = underline(written);
        out.underline = Some(if invisible_underline(r_pr) {
            Decoration::None
        } else {
            style.unwrap_or(Decoration::Single)
        });
        if style.is_none() {
            cx.report(codes::PX2210, data_site(&[("style", written.into())]));
        }
    }
    out.strike = strike(r_pr.attr("strike").unwrap_or(""));
    out.baseline = r_pr
        .int_attr("baseline")
        .map(|baseline| ratio((baseline as f64 / 100_000.0).clamp(-1.0, 1.0)));
    out.caps = caps(r_pr.attr("cap").unwrap_or(""));
    out.letter_spacing = r_pr.int_attr("spc").map(centi_pt);
    out.color = run_color(r_pr, cx);
    if let Some(highlight) = r_pr.child(Ns::A, "highlight") {
        out.highlight = Some(color(&**highlight, cx));
    }
    out.shadow = run_shadow(r_pr, cx);
    let latin = r_pr.child(Ns::A, "latin").attr("typeface");
    out.font = font_ref(latin);
    for script in ["ea", "cs"] {
        let other = r_pr.child(Ns::A, script).attr("typeface");
        if let (Some(other), Some(latin)) = (other, latin)
            && !other.is_empty()
            && !latin.is_empty()
            && other != latin
        {
            cx.report(
                codes::PX2207,
                data_site(&[("script", script.into()), ("typeface", other.into())]),
            );
        }
    }
    if let Some(hlink) = r_pr.child(Ns::A, "hlinkClick")
        && let Some(link) = cx.link(hlink)
    {
        out.link = Some(Some(link));
        // Linked text shows in the theme's hyperlink colour, unless the link keeps the text's (Office 2019's
        // hlinkClr).
        if !keeps_text_color(hlink) {
            out.color = Some(theme_color(MappedSlot::Hlink, Vec::new()));
        }
    }
    out
}

/// `PartialParaProps` with `over`'s declared fields over `base`'s.
pub(crate) fn overlay_para(base: &PartialParaProps, over: &PartialParaProps) -> PartialParaProps {
    macro_rules! pick {
        ($($field:ident),*) => {
            PartialParaProps { $($field: over.$field.clone().or_else(|| base.$field.clone()),)* }
        };
    }
    pick!(
        align,
        margin_left,
        margin_right,
        indent,
        space_before,
        space_after,
        line_spacing,
        tab_size,
        bullet,
        rtl,
        line_height,
        space_before_pt,
        space_after_pt,
        leading_skew,
        line_tail_pt
    )
}

/// `PartialRunProps` with `over`'s declared fields over `base`'s.
pub(crate) fn overlay_run(base: &PartialRunProps, over: &PartialRunProps) -> PartialRunProps {
    macro_rules! pick {
        ($($field:ident),*) => {
            PartialRunProps { $($field: over.$field.clone().or_else(|| base.$field.clone()),)* }
        };
    }
    pick!(
        font,
        size,
        weight,
        italic,
        underline,
        strike,
        color,
        highlight,
        baseline,
        caps,
        letter_spacing,
        link,
        synthesize_bold,
        script_scale,
        shadow
    )
}

fn para_is_empty(props: &PartialParaProps) -> bool {
    props == &PartialParaProps::default()
}

fn run_is_empty(props: &PartialRunProps) -> bool {
    props == &PartialRunProps::default()
}

/// One level of a list style: its paragraph properties and its run defaults.
fn level_style(p_pr: Option<&Element>, cx: &mut dyn TextCx) -> LevelStyle {
    let para = read_para_props(p_pr, cx);
    let run = read_run_props(p_pr.child(Ns::A, "defRPr"), cx);
    LevelStyle {
        para: (!para_is_empty(&para)).then_some(para),
        run: (!run_is_empty(&run)).then_some(run),
    }
}

/// A text context that drops PX2209: list styles commonly set a bullet's colour, font or size alone, for the
/// bullets paragraphs declare.
struct ListStyleCx<'c>(&'c mut dyn TextCx);

impl Report for ListStyleCx<'_> {
    fn report(&mut self, code: DiagnosticCode, site: DiagnosticSite) {
        if code != codes::PX2209 {
            self.0.report(code, site);
        }
    }

    fn dropped(&mut self, modifier: &str) {
        self.0.dropped(modifier);
    }
}

impl TextCx for ListStyleCx<'_> {
    fn link(&mut self, hlink: &Element) -> Option<Link> {
        self.0.link(hlink)
    }

    fn seen(&mut self, feature: FeatureId) {
        self.0.seen(feature);
    }

    fn text_limit(&self) -> usize {
        self.0.text_limit()
    }
}

/// A list style (`a:lstStyle`, a master text style, `p:defaultTextStyle`): `a:defPPr` under every level.
pub(crate) fn read_level_styles(
    element: Option<&Element>,
    text: &mut dyn TextCx,
) -> Option<LevelStyles> {
    let element = element?;
    let mut cx = ListStyleCx(text);
    let base = level_style(element.child(Ns::A, "defPPr").map(|rc| &**rc), &mut cx);
    let mut out = LevelStyles::default();
    let mut any = false;
    for index in 0..9 {
        let name = format!("lvl{}pPr", index + 1);
        let own = level_style(element.child(Ns::A, &name).map(|rc| &**rc), &mut cx);
        let para = overlay_para(
            &base.para.clone().unwrap_or_default(),
            &own.para.unwrap_or_default(),
        );
        let run = overlay_run(
            &base.run.clone().unwrap_or_default(),
            &own.run.unwrap_or_default(),
        );
        let style = LevelStyle {
            para: (!para_is_empty(&para)).then_some(para),
            run: (!run_is_empty(&run)).then_some(run),
        };
        if style.para.is_some() || style.run.is_some() {
            any = true;
            if let Some(slot) = out.slot_mut(index) {
                *slot = Some(Box::new(style));
            }
        }
    }
    any.then_some(out)
}

/// `ST_TextAnchoringType`, for a text body's `anchor` and a table cell's.
pub(crate) fn anchor(name: &str) -> Option<Anchor> {
    match name {
        "t" | "just" => Some(Anchor::Top),
        "ctr" | "dist" => Some(Anchor::Middle),
        "b" => Some(Anchor::Bottom),
        _ => None,
    }
}

/// `a:bodyPr` as declared: each side of the insets it states, the others inherit (ECMA's defaults at the end).
fn body_props(body_pr: Option<&Element>, cx: &mut dyn TextCx) -> PartialBodyProps {
    let mut out = PartialBodyProps::default();
    let Some(body_pr) = body_pr else { return out };
    let inset = |name: &str| body_pr.int_attr(name).map(emu_to_pt);
    let insets = PartialInsets {
        l: inset("lIns"),
        t: inset("tIns"),
        r: inset("rIns"),
        b: inset("bIns"),
    };
    if insets != PartialInsets::default() {
        out.insets = Some(insets);
    }
    out.anchor = anchor(body_pr.attr("anchor").unwrap_or(""));
    out.wrap = match body_pr.attr("wrap") {
        Some("square") => Some(Wrap::Square),
        Some("none") => Some(Wrap::None),
        _ => None,
    };
    if body_pr.child(Ns::A, "noAutofit").is_some() {
        out.autofit = Some(Autofit::None {});
    }
    if let Some(shrink) = body_pr.child(Ns::A, "normAutofit") {
        cx.seen(FeatureId::TextAutofitShrink);
        // PowerPoint writes the scale it fitted the text at; a bare `normAutofit` stores none, so it is fitted later
        // by PowerPoint's steps.
        let scale = shrink.int_attr("fontScale");
        let reduction = shrink.int_attr("lnSpcReduction");
        let stored = scale.is_some() || reduction.is_some();
        out.autofit = Some(Autofit::Shrink {
            font_scale: stored
                .then(|| percent_unit(scale.unwrap_or(100_000).clamp(1_000, 100_000))),
            line_spacing_reduction: stored
                .then(|| percent_unit(reduction.unwrap_or(0).clamp(0, 100_000))),
            fitted: None,
            steps: Some(ShrinkSteps::Office),
        });
    }
    if body_pr.child(Ns::A, "spAutoFit").is_some() {
        cx.seen(FeatureId::TextAutofitResize);
        out.autofit = Some(Autofit::Resize {
            width: false,
            height: true,
            anchor_x: AnchorX::Left,
        });
    }
    let columns = body_pr.int_attr("numCol").unwrap_or(1);
    if columns > 1 {
        cx.seen(FeatureId::TextColumns);
        cx.report(codes::PX2201, data_site(&[("columns", columns.into())]));
    }
    let vert = body_pr.attr("vert").unwrap_or("horz");
    if vert != "horz" {
        cx.seen(FeatureId::TextVertical);
        cx.report(codes::PX2202, data_site(&[("vert", vert.into())]));
    }
    if Some(body_pr).bool_attr("anchorCtr") == Some(true) {
        cx.report(codes::PX2203, DiagnosticSite::new());
    }
    out
}

/// The features a run's declared properties hold in the source.
fn run_features(r_pr: Option<&Element>, cx: &mut dyn TextCx) {
    let Some(r_pr) = r_pr else { return };
    let r = Some(r_pr);
    if r.bool_attr("b") == Some(true) {
        cx.seen(FeatureId::TextWeight);
    }
    if r.bool_attr("i") == Some(true) {
        cx.seen(FeatureId::TextItalic);
    }
    if r_pr
        .attr("u")
        .is_some_and(|u| u != "none" && !invisible_underline(r_pr))
    {
        cx.seen(FeatureId::TextUnderline);
    }
    if matches!(r_pr.attr("strike"), Some("sngStrike" | "dblStrike")) {
        cx.seen(FeatureId::TextStrike);
    }
    if r_pr.child(Ns::A, "highlight").is_some() {
        cx.seen(FeatureId::TextHighlight);
    }
    if run_has_shadow(r_pr) {
        cx.seen(FeatureId::TextShadow);
    }
    if r.int_attr("baseline").unwrap_or(0) != 0 {
        cx.seen(FeatureId::TextBaseline);
    }
    if matches!(r_pr.attr("cap"), Some("all" | "small")) {
        cx.seen(FeatureId::TextCaps);
    }
    if r.int_attr("spc").unwrap_or(0) != 0 {
        cx.seen(FeatureId::TextLetterSpacing);
    }
}

/// `slidenum` is the slide number; `datetime`, `datetime1` … `datetime9` are dates, `datetime10` … `datetime13`
/// times.
fn field_kind(kind: &str) -> Option<FieldKind> {
    if kind == "slidenum" {
        return Some(FieldKind::SlideNumber);
    }
    let digits = kind.strip_prefix("datetime")?;
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number: f64 = if digits.is_empty() {
        0.0
    } else {
        digits.parse().unwrap_or(0.0)
    };
    Some(if number >= 10.0 {
        FieldKind::Time
    } else {
        FieldKind::Date
    })
}

fn text_of(run: &Element, chars: &mut usize, cx: &mut dyn TextCx) -> Result<String, LowerError> {
    let holder = run.child(Ns::A, "t").map_or(run, |rc| &**rc);
    let text = holder.text_content();
    let length = utf16::len(&text);
    *chars = chars.saturating_add(length);
    let limit = cx.text_limit();
    if *chars > limit {
        return Err(LowerError::Limit(format!(
            "a text body holds more than {limit} characters"
        )));
    }
    if length > MAX_STRING {
        return Err(LowerError::Limit(format!(
            "a run holds more than {MAX_STRING} characters"
        )));
    }
    Ok(text)
}

/// A run's text, its tabs as tab inlines.
fn run_inlines(text: &str, props: &PartialRunProps, out: &mut Vec<SourceInline>) {
    for (index, piece) in text.split('\t').enumerate() {
        if index > 0 {
            out.push(Inline::Tab {});
        }
        if !piece.is_empty() || text.is_empty() {
            out.push(Inline::Run {
                text: piece.to_owned(),
                props: props.clone(),
                line_height: None,
                leading_skew: None,
            });
        }
    }
}

fn has_text(text: &str) -> bool {
    text.chars().any(|c| c != '\t')
}

fn inlines_of(
    p: &Element,
    chars: &mut usize,
    cx: &mut dyn TextCx,
) -> Result<Vec<SourceInline>, LowerError> {
    let mut out = Vec::new();
    for element in p.kids() {
        if element.ns != Ns::A {
            continue;
        }
        let r_pr = element.child(Ns::A, "rPr").map(|rc| &**rc);
        match &*element.local {
            "r" => {
                run_features(r_pr, cx);
                let props = read_run_props(r_pr, cx);
                if props.link.is_some() {
                    cx.seen(FeatureId::TextLink);
                }
                let text = text_of(element, chars, cx)?;
                if has_text(&text) {
                    cx.seen(FeatureId::TextRun);
                }
                if text.contains('\t') {
                    cx.seen(FeatureId::TextTab);
                }
                run_inlines(&text, &props, &mut out);
            }
            "br" => {
                run_features(r_pr, cx);
                cx.seen(FeatureId::TextBreak);
                let props = read_run_props(r_pr, cx);
                if props.link.is_some() {
                    cx.seen(FeatureId::TextLink);
                }
                out.push(Inline::Break {
                    props: (!run_is_empty(&props)).then_some(props),
                });
            }
            "fld" => {
                run_features(r_pr, cx);
                let props = read_run_props(r_pr, cx);
                if props.link.is_some() {
                    cx.seen(FeatureId::TextLink);
                }
                let cached = text_of(element, chars, cx)?;
                let kind = element.attr("type").unwrap_or("");
                match field_kind(kind) {
                    Some(field) => {
                        cx.seen(FeatureId::TextField);
                        out.push(Inline::Field {
                            field,
                            cached: clip(&cached, 256),
                            props,
                            line_height: None,
                            leading_skew: None,
                        });
                    }
                    None => {
                        let shown = clip(kind, 64);
                        let shown = if shown.is_empty() {
                            "(none)".to_owned()
                        } else {
                            shown
                        };
                        cx.report(codes::PX2206, data_site(&[("type", shown.into())]));
                        if has_text(&cached) {
                            cx.seen(FeatureId::TextRun);
                        }
                        run_inlines(&cached, &props, &mut out);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// The features a paragraph's own properties hold in the source.
fn paragraph_features(props: &PartialParaProps, p_pr: Option<&Element>, cx: &mut dyn TextCx) {
    if p_pr.child(Ns::A, "buChar").is_some() {
        cx.seen(FeatureId::TextBulletChar);
    }
    if p_pr.child(Ns::A, "buAutoNum").is_some() {
        cx.seen(FeatureId::TextBulletAuto);
    }
    if p_pr.child(Ns::A, "buBlip").is_some() {
        cx.seen(FeatureId::TextBulletImage);
    }
    if props.rtl == Some(true) {
        cx.seen(FeatureId::TextRtl);
    }
    if let Some(line) = &props.line_spacing {
        let single = matches!(line, LineSpacing::Multiple { value, .. } if value.to_f64() == 1.0);
        if !single {
            cx.seen(FeatureId::TextLineSpacing);
        }
    }
}

fn paragraph(
    p: &Element,
    chars: &mut usize,
    cx: &mut dyn TextCx,
) -> Result<SourceParagraph, LowerError> {
    let p_pr = p.child(Ns::A, "pPr").map(|rc| &**rc);
    let props = read_para_props(p_pr, cx);
    paragraph_features(&props, p_pr, cx);
    let end_props = read_run_props(p.child(Ns::A, "endParaRPr").map(|rc| &**rc), cx);
    let level = p_pr
        .int_attr("lvl")
        .unwrap_or(0)
        .clamp(0, i64::from(MAX_LEVEL));
    Ok(SourceParagraph {
        level: u32::try_from(level).unwrap_or(0),
        props,
        inlines: inlines_of(p, chars, cx)?,
        end_props: (!run_is_empty(&end_props)).then_some(end_props),
    })
}

/// Run properties a style sets on each level where the list style leaves them unset.
pub(crate) fn with_run_defaults(
    list_style: Option<&LevelStyles>,
    run: &PartialRunProps,
) -> LevelStyles {
    let mut out = LevelStyles::default();
    for index in 0..9 {
        let own = list_style.and_then(|styles| styles.get(index));
        let style = LevelStyle {
            para: own.and_then(|own| own.para.clone()),
            run: Some(overlay_run(
                run,
                &own.and_then(|own| own.run.clone()).unwrap_or_default(),
            )),
        };
        if let Some(slot) = out.slot_mut(index) {
            *slot = Some(Box::new(style));
        }
    }
    out
}

/// A style's font reference sets each level's font and colour where the list style leaves them unset.
fn with_font_ref(
    list_style: Option<&LevelStyles>,
    slot: ThemeFontSlot,
    color: Option<&Color>,
) -> LevelStyles {
    let run = PartialRunProps {
        font: Some(theme_font(slot)),
        color: color.cloned(),
        ..PartialRunProps::default()
    };
    with_run_defaults(list_style, &run)
}

/// A text body's list style and paragraphs, as declared: what a table cell's text holds besides its cell's margins.
pub(crate) fn read_text_content(
    tx_body: &Element,
    cx: &mut dyn TextCx,
) -> Result<(Option<LevelStyles>, Vec<SourceParagraph>), LowerError> {
    let list_style = read_level_styles(tx_body.child(Ns::A, "lstStyle").map(|rc| &**rc), cx);
    let mut chars = 0;
    let paragraphs = tx_body
        .kids()
        .filter(|kid| &*kid.local == "p" && kid.ns == Ns::A)
        .map(|p| paragraph(p, &mut chars, cx))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((list_style, paragraphs))
}

/// A text body (`p:txBody`): body properties, list style and paragraphs, everything as declared. `font` is a shape
/// style's font reference.
pub(crate) fn read_text_body(
    tx_body: &Element,
    cx: &mut dyn TextCx,
    font: Option<(ThemeFontSlot, Option<&Color>)>,
) -> Result<TextBody, LowerError> {
    let body = body_props(tx_body.child(Ns::A, "bodyPr").map(|rc| &**rc), cx);
    let (declared, paragraphs) = read_text_content(tx_body, cx)?;
    let list_style = match font {
        Some((slot, color)) => Some(with_font_ref(declared.as_ref(), slot, color)),
        None => declared,
    };
    Ok(TextBody {
        body,
        list_style,
        paragraphs,
        flips_with_frame: false,
        rotation: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_theme_font_names_and_field_kinds() {
        assert!(matches!(font_ref(Some("+mj-ea")), Some(FontRef::Theme(_))));
        assert!(matches!(font_ref(Some("+mj-xx")), Some(FontRef::Family(_))));
        assert_eq!(font_ref(Some("")), None);
        assert_eq!(field_kind("datetime"), Some(FieldKind::Date));
        assert_eq!(field_kind("datetime9"), Some(FieldKind::Date));
        assert_eq!(field_kind("datetime10"), Some(FieldKind::Time));
        assert_eq!(field_kind("datetime007"), Some(FieldKind::Date));
        assert_eq!(field_kind("datetime1x"), None);
        assert_eq!(field_kind("footer"), None);
    }
}
