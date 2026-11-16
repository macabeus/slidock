//! Pages (`draw:page`) as slides: the master page they show (as their layout), whether they are hidden, whether the
//! master's objects show behind them, their background, their drawables and their speaker notes. A page that shows its
//! master's page number, footer or date gets a copy of that master object as its own element, under a derived id
//! (`od-<page>.m.<key>`): the field text is content, which inheritance leaves on the master. Transitions and animations
//! go through [`crate::motion`].

use slidock_canon::utf16;
use slidock_core_api::lowering::LowerError;
use slidock_ir::color::Color;
use slidock_ir::features::FeatureId;
use slidock_ir::ids::{IdPart, NodeId, derived_id};
use slidock_ir::origin::Origin;
use slidock_ir::paint::Fill;
use slidock_ir::presentation::SlideSize;
use slidock_ir::source::{Element as IrElement, Slide};
use slidock_ir::text::{Inline, PartialBodyProps, TextBody};

use crate::context::Lowering;
use crate::document::{MasterRef, PageRef, STYLES, page_props};
use crate::element::{role_of, style_of};
use crate::graphic::{Paint, read_fill};
use crate::ids::id_failure;
use crate::motion::{self, MotionMode, MotionSource};
use crate::names::{QName, qn};
use crate::odf::{Element, Ns, XmlNode};
use crate::shapes::{Keyed, keyed_drawables, lower_copy, lower_drawables};
use crate::styles::{PropertySet, StyleScope, get};
use crate::table::table_slots;
use crate::text::{ShapeText, TextContext, read_text_body};
use crate::units::rgb;

/// A page that hides its master's background shows blank paper, as LibreOffice draws it.
fn paper() -> Fill {
    Fill::Solid {
        color: Color::Rgb(rgb(1.0, 1.0, 1.0, 1.0)),
    }
}

/// The master objects a page may show, with LibreOffice's default for a page that leaves the flag unset.
const SHOWN_FROM_MASTER: [(&str, QName, bool); 3] = [
    (
        "page-number",
        qn!("presentation:display-page-number"),
        false,
    ),
    ("footer", qn!("presentation:display-footer"), true),
    ("date-time", qn!("presentation:display-date-time"), true),
];

/// Whether a master object would show anything on this page: a field, a declared footer or date, or text.
fn shows_something<'a>(element: &'a Element, ctx: &Lowering<'a, '_>) -> bool {
    fn visit<'a>(node: &'a Element, ctx: &Lowering<'a, '_>) -> bool {
        node.children.iter().any(|part| match part {
            XmlNode::Text(text) => {
                !slidock_canon::js_trim(text).is_empty() && node.in_namespace(Ns::Text)
            }
            XmlNode::Element(part) => {
                if part.in_namespace(Ns::Text)
                    && ["page-number", "page-count", "date", "time"].contains(&part.local.as_str())
                {
                    return true;
                }
                if part.is(Ns::Presentation, "footer") {
                    return ctx
                        .declarations
                        .is_some_and(|d| d.footer.as_ref().is_some_and(|f| !f.is_empty()));
                }
                if part.is(Ns::Presentation, "date-time") {
                    return ctx.declarations.is_some_and(|d| d.date_time.is_some());
                }
                visit(part, ctx)
            }
        })
    }
    visit(element, ctx)
}

/// The master objects a page may copy: the first of each class, by `SHOWN_FROM_MASTER`.
fn copiable<'a>(master: &MasterRef<'a>) -> [Option<Keyed<'a>>; 3] {
    let mut drawables = keyed_drawables(master.element, &master.path);
    SHOWN_FROM_MASTER.map(|(class, _, _)| {
        drawables
            .iter()
            .position(|keyed| keyed.element.attr(qn!("presentation:class")) == Some(class))
            .map(|at| drawables.remove(at))
    })
}

/// The grid slots of the tables a page's slide may lower: its own, and those of the master objects it may copy.
pub(crate) fn slide_slots(page: &PageRef<'_>, master: Option<&MasterRef<'_>>) -> u64 {
    let sources = master.map(copiable);
    sources
        .iter()
        .flatten()
        .flatten()
        .fold(table_slots(page.element), |total, source| {
            slidock_core_api::limits::add_slots(total, table_slots(source.element))
        })
}

/// Copies of the master's page number, footer and date the page shows, for each role its own elements lack.
fn shown_from_master<'a, 'd>(
    slide: &NodeId,
    props: &PropertySet,
    own: &[IrElement],
    master: &MasterRef<'a>,
    master_styles: &'d StyleScope<'a>,
    ctx: &Lowering<'a, 'd>,
) -> Result<Vec<IrElement>, LowerError> {
    let sources = copiable(master);
    let mut out = Vec::new();
    for ((_, flag, fallback), source) in SHOWN_FROM_MASTER.iter().zip(sources) {
        let shown = get(props, *flag).map_or(*fallback, |written| written == "true");
        if !shown {
            continue;
        }
        let Some(source) = source else { continue };
        if !shows_something(source.element, ctx) {
            continue;
        }
        let role = role_of(source.element);
        if own
            .iter()
            .any(|element| element.placeholder().map(|p| p.role) == role)
        {
            continue;
        }
        let id = derived_id(slide, IdPart::Str(&source.key)).map_err(id_failure)?;
        let copy = lower_copy(
            source.element,
            &source.path,
            id,
            &ctx.through_styles(master_styles, STYLES),
        )?;
        out.extend(copy);
    }
    Ok(out)
}

/// The speaker notes: the text of the notes page's notes object, when it holds some.
fn read_notes<'a>(
    page: &'a Element,
    ctx: &Lowering<'a, '_>,
) -> Result<Option<TextBody>, LowerError> {
    let frame = page.child(Ns::Presentation, "notes").and_then(|notes| {
        notes.kids().find(|element| {
            element.is(Ns::Draw, "frame")
                && element.attr(qn!("presentation:class")) == Some("notes")
        })
    });
    let Some(frame) = frame else { return Ok(None) };
    let Some(text_box) = frame.child(Ns::Draw, "text-box") else {
        return Ok(None);
    };
    let style = style_of(frame, ctx);
    let text = read_text_body(
        &ShapeText {
            holder: text_box,
            shape: frame,
            style: &style,
            custom_shape: false,
        },
        &TextContext::notes(*ctx),
    )?;
    let Some(text) = text else { return Ok(None) };
    let written = text.paragraphs.iter().any(|paragraph| {
        paragraph.inlines.iter().any(|inline| match inline {
            Inline::Field { .. } => true,
            Inline::Run { text, .. } => !slidock_canon::js_trim(text).is_empty(),
            _ => false,
        })
    });
    if !written {
        return Ok(None);
    }
    ctx.see(FeatureId::SlideNotes);
    Ok(Some(TextBody {
        body: PartialBodyProps::default(),
        list_style: None,
        paragraphs: text.paragraphs,
        flips_with_frame: false,
        rotation: None,
    }))
}

/// A page with the layout and master page it shows.
pub(crate) struct SlideSource<'a, 'd> {
    pub page: &'d PageRef<'a>,
    pub layout: Option<NodeId>,
    pub master: Option<&'d MasterRef<'a>>,
    pub master_styles: &'d StyleScope<'a>,
    pub size: &'d SlideSize,
    pub motion: MotionMode,
}

pub(crate) fn lower_slide<'a, 'd>(
    source: &SlideSource<'a, 'd>,
    ctx: &Lowering<'a, 'd>,
) -> Result<Slide, LowerError> {
    let page = source.page;
    let props = page_props(page.element, ctx.styles);
    let hidden = get(&props, qn!("presentation:visibility")) == Some("hidden");
    if hidden {
        ctx.see(FeatureId::PresentationHiddenSlide);
    }
    let paint = Paint::of(*ctx, None);
    let own = read_fill(&props, &paint)?.fill;
    let background = own.or_else(|| {
        (get(&props, qn!("presentation:background-visible")) == Some("false")).then(paper)
    });
    if background
        .as_ref()
        .is_some_and(|fill| !matches!(fill, Fill::None))
    {
        ctx.see(FeatureId::SlideBackground);
    }
    let motion_source = |elements| MotionSource {
        page: page.element,
        path: &page.path,
        props: &props,
        elements,
        size: source.size,
    };
    // The parity path reports the page's motion before its drawables, as it always has.
    let detected = (source.motion == MotionMode::Detect)
        .then(|| motion::read(&motion_source(&[]), MotionMode::Detect, ctx));
    let elements = lower_drawables(page.element, &page.path, ctx)?;
    let motion = match detected {
        Some(motion) => motion,
        None => motion::read(&motion_source(&elements), source.motion, ctx),
    };
    let copies = match source.master {
        Some(master) => shown_from_master(
            ctx.container(),
            &props,
            &elements,
            master,
            source.master_styles,
            ctx,
        )?,
        None => Vec::new(),
    };
    let notes = read_notes(page.element, ctx)?;
    let mut all = elements;
    all.extend(copies);
    Ok(Slide {
        id: ctx.container().clone(),
        origin: Origin::Source {
            part: ctx.part.to_owned(),
            path: Some(utf16::clip(&page.path, 512).to_owned()),
            object: None,
        },
        layout: source.layout.clone(),
        hidden,
        show_layout: get(&props, qn!("presentation:background-objects-visible")) != Some("false"),
        color_map_override: None,
        notes,
        transition: motion.transition,
        timeline: motion.timeline,
        ext: None,
        background,
        elements: all,
        title: None,
        reading_order: None,
        failed: None,
    })
}
