//! Masters, layouts and slides: what they share (CT_CommonSlideData and the colour mapping: the name, the background
//! and the shape tree), a master's text styles and the layouts it lists, a layout's type, and a slide's visibility,
//! speaker notes and transition.

use slidock_core_api::lowering::{IdScope, LowerError};
use slidock_ir::color::{ColorMap, SchemeSlot};
use slidock_ir::diagnostics::{DiagnosticCode, DiagnosticSite, codes};
use slidock_ir::features::FeatureId;
use slidock_ir::ids::{IdPart, LayoutId, MasterId, ThemeId};
use slidock_ir::origin::Origin;
use slidock_ir::paint::Fill;
use slidock_ir::presentation::TextStyles;
use slidock_ir::source::{Element as IrElement, Layout, Master, Slide};
use slidock_ir::text::{Inline, TextBody};

use crate::clip;
use crate::context::Lowering;
use crate::fill::{PaintCx, fill_element, read_fill};
use crate::package::{Package, PartError};
use crate::presentation::PartRef;
use crate::report::Report;
use crate::shadow::background_effects;
use crate::shapes::lower_tree;
use crate::style_refs::background_ref;
use crate::text::{TextCx, read_level_styles, read_text_body};
use crate::timing::{apply_playback, drop_timing, read_timeline};
use crate::transition::{read_native_transition, read_transition};
use crate::xml::{El, Element, Ns};

/// The scheme slots in a colour map's order, with the names a map writes them by.
const MAPPED: [&str; 16] = [
    "dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6",
    "hlink", "folHlink", "bg1", "tx1", "bg2", "tx2",
];

pub(crate) fn scheme_slot(name: &str) -> Option<SchemeSlot> {
    Some(match name {
        "dk1" => SchemeSlot::Dk1,
        "lt1" => SchemeSlot::Lt1,
        "dk2" => SchemeSlot::Dk2,
        "lt2" => SchemeSlot::Lt2,
        "accent1" => SchemeSlot::Accent1,
        "accent2" => SchemeSlot::Accent2,
        "accent3" => SchemeSlot::Accent3,
        "accent4" => SchemeSlot::Accent4,
        "accent5" => SchemeSlot::Accent5,
        "accent6" => SchemeSlot::Accent6,
        "hlink" => SchemeSlot::Hlink,
        "folHlink" => SchemeSlot::FolHlink,
        _ => return None,
    })
}

/// The slot each role takes when a map omits it: the usual mapping here, else the slot of the same name.
fn identity(name: &str) -> SchemeSlot {
    match name {
        "bg1" => SchemeSlot::Lt1,
        "tx1" => SchemeSlot::Dk1,
        "bg2" => SchemeSlot::Lt2,
        "tx2" => SchemeSlot::Dk2,
        other => scheme_slot(other).unwrap_or(SchemeSlot::Dk1),
    }
}

/// A `p:clrMap` or `a:overrideClrMapping` as a complete map.
pub(crate) fn color_map(element: Option<&Element>) -> Option<ColorMap> {
    let element = element?;
    let slots: Vec<SchemeSlot> = MAPPED
        .iter()
        .map(|name| {
            element
                .attr(name)
                .and_then(scheme_slot)
                .unwrap_or_else(|| identity(name))
        })
        .collect();
    let slot = |index: usize| slots.get(index).copied().unwrap_or(SchemeSlot::Dk1);
    Some(ColorMap {
        dk1: slot(0),
        lt1: slot(1),
        dk2: slot(2),
        lt2: slot(3),
        accent1: slot(4),
        accent2: slot(5),
        accent3: slot(6),
        accent4: slot(7),
        accent5: slot(8),
        accent6: slot(9),
        hlink: slot(10),
        fol_hlink: slot(11),
        bg1: slot(12),
        tx1: slot(13),
        bg2: slot(14),
        tx2: slot(15),
    })
}

/// A layout's or slide's `p:clrMapOvr`: an override map, or `None` to keep the master's mapping.
fn color_map_override(root: &Element) -> Option<ColorMap> {
    color_map(
        root.child(Ns::P, "clrMapOvr")
            .child(Ns::A, "overrideClrMapping"),
    )
}

fn common_data(root: &Element) -> Option<&Element> {
    root.child(Ns::P, "cSld").map(|rc| &**rc)
}

fn container_name(c_sld: Option<&Element>) -> Option<String> {
    c_sld
        .attr("name")
        .filter(|name| !name.is_empty())
        .map(|name| clip(name, 256))
}

/// `p:bg`: its own fill (`p:bgPr`) or a fill style of the theme (`p:bgRef`). A background's shadow is reported.
fn background(c_sld: Option<&Element>, ctx: &mut Lowering<'_>) -> Result<Option<Fill>, LowerError> {
    let bg = c_sld.child(Ns::P, "bg");
    let bg_ref = bg.child(Ns::P, "bgRef");
    let bg_pr = bg.child(Ns::P, "bgPr");
    background_effects(bg_pr, ctx);
    let element = fill_element(bg_pr);
    let mut paint = PaintCx::new(ctx, None).with_pictures(FeatureId::SlideBackground);
    if let Some(element) = element {
        if &*element.local != "noFill" {
            paint.ctx.seen(FeatureId::SlideBackground);
        }
        return read_fill(element, &mut paint, None);
    }
    let fill = match bg_ref {
        Some(bg_ref) => background_ref(bg_ref, &mut paint)?,
        None => None,
    };
    if fill
        .as_ref()
        .is_some_and(|fill| !matches!(fill, Fill::None))
    {
        paint.ctx.seen(FeatureId::SlideBackground);
    }
    Ok(fill)
}

/// The shape tree of a container's root (`p:sld`, `p:sldLayout`, `p:sldMaster`).
fn elements(root: &Element, ctx: &mut Lowering<'_>) -> Result<Vec<IrElement>, LowerError> {
    let c_sld = common_data(root);
    let sp_tree = c_sld.child(Ns::P, "spTree");
    let name = |element: Option<&Element>| {
        element.map_or_else(
            || "undefined".to_owned(),
            |element| element.name.to_string(),
        )
    };
    let path = format!("/{}/{}/{}", root.name, name(c_sld), name(sp_tree));
    lower_tree(sp_tree, &path, ctx)
}

/// How text outside a shape reads (a master's text styles, a slide's notes): reports land on the container.
pub(crate) struct ContainerText<'l, 'a>(pub &'l mut Lowering<'a>);

impl Report for ContainerText<'_, '_> {
    fn report(&mut self, code: DiagnosticCode, site: DiagnosticSite) {
        Report::report(self.0, code, site);
    }

    fn dropped(&mut self, modifier: &str) {
        Report::dropped(self.0, modifier);
    }
}

impl TextCx for ContainerText<'_, '_> {
    fn text_limit(&self) -> usize {
        self.0.limits.text_per_body
    }
}

/// The layouts a master lists, with the ids `sldLayoutIdLst` gives them.
pub(crate) fn listed_layouts(
    pkg: &Package<'_>,
    master: &PartRef,
    root: &Element,
    scope: &mut IdScope,
) -> Result<Vec<PartRef>, LowerError> {
    let mut out = Vec::new();
    for element in root
        .child(Ns::P, "sldLayoutIdLst")
        .children(Ns::P, "sldLayoutId")
    {
        let Some(rel) = pkg.target(&master.part, element.rel_attr("id"), Some("slideLayout"))?
        else {
            continue;
        };
        let taken = scope
            .take(&[IdPart::Str(element.attr("id").unwrap_or("_"))])
            .map_err(|error| LowerError::Other(error.to_string()))?;
        out.push(PartRef {
            id: taken.id,
            part: rel.target,
            duplicate: taken.duplicate,
        });
    }
    Ok(out)
}

pub(crate) fn lower_master(
    root: &Element,
    theme: Option<ThemeId>,
    ctx: &mut Lowering<'_>,
) -> Result<Master, LowerError> {
    let c_sld = common_data(root);
    let name = container_name(c_sld);
    let styles = root.child(Ns::P, "txStyles");
    let mut text = ContainerText(ctx);
    let mut style = |local: &str| read_level_styles(styles.child(Ns::P, local), &mut text);
    let title = style("titleStyle");
    let body = style("bodyStyle");
    let other = style("otherStyle");
    let map = color_map(root.child(Ns::P, "clrMap").map(|rc| &**rc));
    let fill = background(c_sld, ctx)?;
    let any = title.is_some() || body.is_some() || other.is_some();
    Ok(Master {
        id: ctx.container.clone(),
        origin: Origin::source(ctx.part.clone()),
        name,
        theme,
        color_map: map,
        text_styles: any.then_some(TextStyles { title, body, other }),
        ext: None,
        background: fill,
        elements: elements(root, ctx)?,
        failed: None,
    })
}

pub(crate) fn lower_layout(
    root: &Element,
    master: Option<MasterId>,
    ctx: &mut Lowering<'_>,
) -> Result<Layout, LowerError> {
    let c_sld = common_data(root);
    let name = container_name(c_sld);
    let layout_type = root
        .attr("type")
        .filter(|kind| !kind.is_empty())
        .map(|kind| clip(kind, 64));
    let override_map = color_map_override(root);
    let fill = background(c_sld, ctx)?;
    Ok(Layout {
        id: ctx.container.clone(),
        origin: Origin::source(ctx.part.clone()),
        name,
        layout_type,
        master,
        show_master: Some(root).bool_attr("showMasterSp").unwrap_or(true),
        color_map_override: override_map,
        ext: None,
        background: fill,
        elements: elements(root, ctx)?,
        failed: None,
    })
}

/// Whether a slide part's root says the slide is hidden (`show="0"`).
pub(crate) fn is_hidden(root: &Element) -> bool {
    root.is(Ns::P, "sld") && Some(root).bool_attr("show") == Some(false)
}

/// Speaker notes: the body placeholder of the slide's notes slide, as text content. Notes skip the cascade and the
/// slide's render, so their text stays as read and their features stay off the slide's list.
fn read_notes(ctx: &mut Lowering<'_>) -> Result<Option<TextBody>, LowerError> {
    let Some(rel) = ctx
        .rels
        .values()
        .find(|rel| rel.kind == "notesSlide" && !rel.external)
        .cloned()
    else {
        return Ok(None);
    };
    let notes = match ctx.pkg.xml(&rel.target) {
        Ok(notes) => notes,
        Err(PartError { part, reason }) => {
            ctx.report(
                codes::PX2010,
                DiagnosticSite::new()
                    .data("part", part)
                    .data("reason", reason),
            );
            return Ok(None);
        }
    };
    let body = notes
        .path(&[(Ns::P, "cSld"), (Ns::P, "spTree")])
        .children(Ns::P, "sp")
        .into_iter()
        .find(|sp| {
            sp.path(&[(Ns::P, "nvSpPr"), (Ns::P, "nvPr"), (Ns::P, "ph")])
                .attr("type")
                == Some("body")
        });
    let Some(tx_body) = body.child(Ns::P, "txBody") else {
        return Ok(None);
    };
    let text = read_text_body(tx_body, &mut ContainerText(ctx), None)?;
    let written = text.paragraphs.iter().any(|paragraph| {
        paragraph.inlines.iter().any(|inline| match inline {
            Inline::Tab {} | Inline::Break { .. } => false,
            Inline::Field { cached, .. } => !slidock_canon::js_trim(cached).is_empty(),
            Inline::Run { text, .. } => !slidock_canon::js_trim(text).is_empty(),
        })
    });
    if !written {
        return Ok(None);
    }
    ctx.seen(FeatureId::SlideNotes);
    Ok(Some(TextBody {
        body: Default::default(),
        list_style: None,
        paragraphs: text.paragraphs,
        flips_with_frame: false,
        rotation: None,
    }))
}

pub(crate) fn lower_slide(
    root: &Element,
    layout: Option<LayoutId>,
    ctx: &mut Lowering<'_>,
) -> Result<Slide, LowerError> {
    let c_sld = common_data(root);
    let hidden = is_hidden(root);
    if hidden {
        ctx.seen(FeatureId::PresentationHiddenSlide);
    }
    let override_map = color_map_override(root);
    let fill = background(c_sld, ctx)?;
    let notes = read_notes(ctx)?;
    let (transition, elements, timeline) = match ctx.motion {
        None => {
            let transition = read_transition(root, ctx);
            drop_timing(root, ctx);
            (transition, elements(root, ctx)?, Vec::new())
        }
        Some(size) => {
            let transition = read_native_transition(root, ctx);
            let mut elements = elements(root, ctx)?;
            let (timeline, playback) = read_timeline(root, &elements, size, ctx);
            apply_playback(&mut elements, &playback);
            (transition, elements, timeline)
        }
    };
    Ok(Slide {
        id: ctx.container.clone(),
        origin: Origin::source(ctx.part.clone()),
        layout,
        hidden,
        show_layout: Some(root).bool_attr("showMasterSp").unwrap_or(true),
        color_map_override: override_map,
        notes,
        transition,
        timeline,
        ext: None,
        background: fill,
        elements,
        title: None,
        reading_order: None,
        failed: None,
    })
}
