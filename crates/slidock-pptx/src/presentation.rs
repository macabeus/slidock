//! The presentation part: the slide size, the show order (`sldIdLst` is the only source of order), the masters, the
//! format's text defaults and the embedded fonts. A list names each part once: an entry naming a part its list named
//! before is dropped (PX2013), so a small package cannot lower one slide part many times over.

use std::collections::BTreeSet;
use std::rc::Rc;

use indexmap::IndexMap;
use slidock_core_api::lowering::{IdScope, LowerError};
use slidock_ir::color::MappedSlot;
use slidock_ir::fonts::{FaceRequest, FaceStyle, FontEntry, FontRequest, font_key};
use slidock_ir::formats::IdPrefix;
use slidock_ir::ids::{IdPart, NodeId};
use slidock_ir::presentation::SlideSize;
use slidock_ir::text::{
    Align, Anchor, Autofit, BodyProps, Bullet, Caps, Decoration, DefaultTextStyles, Insets,
    LevelDefaults, LevelStyles, LineBasis, LineSpacing, Overflow, ParaProps, PartialParaProps,
    PartialRunProps, RunProps, Spacing, ThemeFontSlot, Wrap,
};
use slidock_ir::units::{Pt, Ratio, Size, UnitRatio};

use crate::clip;
use crate::color::theme_color;
use crate::package::{Package, PartError};
use crate::text::{TextCx, read_level_styles, theme_font};
use crate::units::emu_to_size;
use crate::xml::{El, Element, Ns};

/// A container part and the id its list gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PartRef {
    pub id: NodeId,
    pub part: String,
    pub duplicate: bool,
}

/// A slide of the show order, with its part (`None` when its relationship is missing or external).
#[derive(Clone, Debug)]
pub(crate) struct SlideRef {
    pub id: NodeId,
    pub part: Option<String>,
    pub duplicate: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct EmbeddedFontEntry {
    pub family: String,
    pub faces: Vec<FaceRequest>,
    /// Each face whose part resolves, with the part's path.
    pub parts: Vec<(FaceRequest, String)>,
}

/// An entry of a list that names a part the list named before; it is dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Relisted {
    pub list: &'static str,
    pub part: String,
}

pub(crate) struct PptxPresentation {
    pub part: String,
    pub size: SlideSize,
    pub slides: Vec<SlideRef>,
    pub masters: Vec<PartRef>,
    /// The entries dropped from `sldIdLst` and `sldMasterIdLst`, in list order.
    pub relisted: Vec<Relisted>,
    /// The one namespace of container ids; layouts take theirs from it as their masters list them.
    pub ids: IdScope,
    pub default_text_style: Option<Rc<Element>>,
    pub embedded_fonts: Vec<EmbeddedFontEntry>,
}

/// PowerPoint's default slide (10 × 7.5 in), for a missing or empty `sldSz`.
fn default_size() -> SlideSize {
    SlideSize {
        w: Size::from_steps(720_000),
        h: Size::from_steps(540_000),
    }
}

/// The presentation part's path, from the package's officeDocument relationship, else `ppt/presentation.xml`.
pub(crate) fn presentation_part(pkg: &Package<'_>) -> Result<Option<String>, PartError> {
    if let Some(rel) = pkg.first("", "officeDocument")?
        && pkg.has(&rel.target)
    {
        return Ok(Some(rel.target));
    }
    const DEFAULT: &str = "ppt/presentation.xml";
    Ok(pkg.has(DEFAULT).then(|| DEFAULT.to_owned()))
}

fn id_error(error: impl std::fmt::Display) -> LowerError {
    LowerError::Other(error.to_string())
}

pub(crate) fn read_pptx_presentation(
    pkg: &Package<'_>,
    part: &str,
    root: &Element,
) -> Result<PptxPresentation, LowerError> {
    let size = root.child(Ns::P, "sldSz");
    let cx = size.int_attr("cx");
    let cy = size.int_attr("cy");
    let mut scope = IdScope::new(IdPrefix::Pp);
    let mut relisted = Vec::new();
    let mut listed = |list: &'static str, item: &str| -> Result<Vec<SlideRef>, LowerError> {
        let mut refs = Vec::new();
        let mut parts = BTreeSet::new();
        for element in root.child(Ns::P, list).children(Ns::P, item) {
            let rel = pkg.target(part, element.rel_attr("id"), None)?;
            let target = rel
                .as_ref()
                .filter(|rel| !rel.external)
                .map(|rel| &rel.target);
            if let Some(target) = target
                && !parts.insert(target.clone())
            {
                relisted.push(Relisted {
                    list,
                    part: target.clone(),
                });
                continue;
            }
            let taken = scope
                .take(&[IdPart::Str(element.attr("id").unwrap_or("_"))])
                .map_err(id_error)?;
            refs.push(SlideRef {
                id: taken.id,
                part: rel.filter(|rel| !rel.external).map(|rel| rel.target),
                duplicate: taken.duplicate,
            });
        }
        Ok(refs)
    };
    let masters = listed("sldMasterIdLst", "sldMasterId")?
        .into_iter()
        .filter_map(|slide| {
            Some(PartRef {
                id: slide.id,
                part: slide.part?,
                duplicate: slide.duplicate,
            })
        })
        .collect();
    let slides = listed("sldIdLst", "sldId")?;
    let embedded_fonts = root
        .child(Ns::P, "embeddedFontLst")
        .children(Ns::P, "embeddedFont")
        .into_iter()
        .filter_map(|entry| {
            let family = entry
                .child(Ns::P, "font")
                .attr("typeface")
                .filter(|family| !family.is_empty())?;
            let slots: Vec<(FaceRequest, Option<&str>)> = [
                ("regular", 400, FaceStyle::Normal),
                ("bold", 700, FaceStyle::Normal),
                ("italic", 400, FaceStyle::Italic),
                ("boldItalic", 700, FaceStyle::Italic),
            ]
            .into_iter()
            .filter_map(|(local, weight, style)| {
                let slot = entry.child(Ns::P, local)?;
                Some((FaceRequest { weight, style }, slot.rel_attr("id")))
            })
            .collect();
            let parts = slots
                .iter()
                .filter_map(|(face, id)| {
                    let rel = pkg.target(part, *id, Some("font")).ok().flatten()?;
                    (!rel.external).then(|| (face.clone(), rel.target))
                })
                .collect();
            Some(EmbeddedFontEntry {
                family: clip(family, 256),
                faces: slots.into_iter().map(|(face, _)| face).collect(),
                parts,
            })
        })
        .collect();
    let size = match (cx, cy) {
        (Some(cx), Some(cy)) if cx > 0 && cy > 0 => SlideSize {
            w: emu_to_size(cx),
            h: emu_to_size(cy),
        },
        _ => default_size(),
    };
    Ok(PptxPresentation {
        part: part.to_owned(),
        size,
        slides,
        masters,
        relisted,
        ids: scope,
        default_text_style: root.child(Ns::P, "defaultTextStyle").cloned(),
        embedded_fonts,
    })
}

/// ECMA-376's paragraph defaults.
fn ecma_para() -> ParaProps {
    ParaProps {
        align: Align::Left,
        margin_left: Pt::ZERO,
        margin_right: Pt::ZERO,
        indent: Pt::ZERO,
        space_before: Spacing::Pt { value: Pt::ZERO },
        space_after: Spacing::Pt { value: Pt::ZERO },
        line_spacing: LineSpacing::Multiple {
            value: Ratio::from_steps(100_000),
            basis: LineBasis::Size1_2,
        },
        tab_size: Size::from_steps(72_000),
        bullet: Bullet::None {},
        rtl: false,
        line_height: None,
        space_before_pt: None,
        space_after_pt: None,
        leading_skew: None,
        line_tail_pt: None,
    }
}

/// ECMA-376's run defaults: 18 pt in the theme's minor font, tx1.
fn ecma_run() -> RunProps {
    RunProps {
        font: theme_font(ThemeFontSlot::Minor),
        size: Size::from_steps(18_000),
        weight: 400,
        italic: false,
        underline: Decoration::None,
        strike: Decoration::None,
        color: theme_color(MappedSlot::Tx1, Vec::new()),
        highlight: None,
        baseline: Ratio::ZERO,
        caps: Caps::None,
        letter_spacing: Pt::ZERO,
        link: None,
        synthesize_bold: true,
        // PowerPoint draws a raised or lowered run smaller; 58% is the height LibreOffice gives it reading OOXML.
        script_scale: UnitRatio::from_steps(58_000),
        shadow: None,
    }
}

/// `bodyPr` defaults: insets of 0.1 in left and right and 0.05 in top and bottom, top-anchored, wrapped.
pub(crate) fn ecma_body() -> BodyProps<Autofit> {
    BodyProps {
        insets: Insets {
            l: Pt::from_steps(7_200),
            t: Pt::from_steps(3_600),
            r: Pt::from_steps(7_200),
            b: Pt::from_steps(3_600),
        },
        anchor: Anchor::Top,
        wrap: Wrap::Square,
        autofit: Autofit::None {},
        overflow: Overflow::Spread,
        max_width: None,
    }
}

fn with_para(base: ParaProps, over: &PartialParaProps) -> ParaProps {
    let PartialParaProps {
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
        line_tail_pt,
    } = over.clone();
    ParaProps {
        align: align.unwrap_or(base.align),
        margin_left: margin_left.unwrap_or(base.margin_left),
        margin_right: margin_right.unwrap_or(base.margin_right),
        indent: indent.unwrap_or(base.indent),
        space_before: space_before.unwrap_or(base.space_before),
        space_after: space_after.unwrap_or(base.space_after),
        line_spacing: line_spacing.unwrap_or(base.line_spacing),
        tab_size: tab_size.unwrap_or(base.tab_size),
        bullet: bullet.unwrap_or(base.bullet),
        rtl: rtl.unwrap_or(base.rtl),
        line_height: line_height.or(base.line_height),
        space_before_pt: space_before_pt.or(base.space_before_pt),
        space_after_pt: space_after_pt.or(base.space_after_pt),
        leading_skew: leading_skew.or(base.leading_skew),
        line_tail_pt: line_tail_pt.or(base.line_tail_pt),
    }
}

fn with_run(base: RunProps, over: &PartialRunProps) -> RunProps {
    let PartialRunProps {
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
        shadow,
    } = over.clone();
    RunProps {
        font: font.unwrap_or(base.font),
        size: size.unwrap_or(base.size),
        weight: weight.unwrap_or(base.weight),
        italic: italic.unwrap_or(base.italic),
        underline: underline.unwrap_or(base.underline),
        strike: strike.unwrap_or(base.strike),
        color: color.unwrap_or(base.color),
        highlight: highlight.unwrap_or(base.highlight),
        baseline: baseline.unwrap_or(base.baseline),
        caps: caps.unwrap_or(base.caps),
        letter_spacing: letter_spacing.unwrap_or(base.letter_spacing),
        link: link.unwrap_or(base.link),
        synthesize_bold: synthesize_bold.unwrap_or(base.synthesize_bold),
        script_scale: script_scale.unwrap_or(base.script_scale),
        shadow: shadow.unwrap_or(base.shadow),
    }
}

/// The presentation's text defaults: ECMA's, with the presentation's `defaultTextStyle` over them.
pub(crate) fn text_defaults(pptx: &PptxPresentation, cx: &mut dyn TextCx) -> DefaultTextStyles {
    let declared = read_level_styles(pptx.default_text_style.as_deref(), cx).unwrap_or_default();
    let level = |index: usize| {
        let style = level_of(&declared, index);
        LevelDefaults {
            para: with_para(
                ecma_para(),
                &style
                    .and_then(|style| style.para.clone())
                    .unwrap_or_default(),
            ),
            run: with_run(
                ecma_run(),
                &style
                    .and_then(|style| style.run.clone())
                    .unwrap_or_default(),
            ),
        }
    };
    DefaultTextStyles {
        l0: level(0),
        l1: level(1),
        l2: level(2),
        l3: level(3),
        l4: level(4),
        l5: level(5),
        l6: level(6),
        l7: level(7),
        l8: level(8),
    }
}

fn level_of(styles: &LevelStyles, index: usize) -> Option<&slidock_ir::text::LevelStyle> {
    styles.get(index)
}

/// Embedded fonts as requests; their bytes stay in the package (Google's are EOT-wrapped).
pub(crate) fn embedded_font_entries(pptx: &PptxPresentation) -> IndexMap<String, FontEntry> {
    let mut out = IndexMap::new();
    for font in &pptx.embedded_fonts {
        out.insert(
            font_key(&font.family),
            FontEntry {
                request: FontRequest {
                    family: font.family.clone(),
                    faces: font.faces.clone(),
                    postscript_names: Vec::new(),
                    embedded: true,
                },
                binding: None,
            },
        );
    }
    out
}
