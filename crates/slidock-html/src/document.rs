//! Slide and layout documents in the format's exact shell: one fixed reset rule in the head, and a `<main>` box holding
//! the background and the elements in reading order. A failed container prints its background and a marker that names
//! why. The build steps are deck data, not part of the document.

use indexmap::IndexMap;
use slidock_format::RESET_STYLE;
use slidock_format::markers::{BACKGROUND, FAILED, attr};
use slidock_format::paths::file_id;
use slidock_ir::color::{Color, ResolvedColor, ResolvedThemeColor, ThemeTag};
use slidock_ir::ids::ElementId;
use slidock_ir::paint::{
    EllipseLowered, Fill, GradientStop, LinearLowered, RadialLowered, RectLowered, ResolvedFill,
};
use slidock_ir::resolved::{Body, Element as IrElement, Layout, Master, Slide};
use slidock_ir::timeline::TimelineStep;
use slidock_ir::units::Pt;

use crate::context::{Motion, PrintCtx};
use crate::css::{MaybeDecl, decl, pt, style};
use crate::error::Result;
use crate::gradient::PaintBox;
use crate::paint::{background_decls, svg_fill};
use crate::paint_motion;
use crate::serialize;
use crate::svg::{path_data, rect, svg_layer};
use crate::timeline::timeline_template;
use crate::tree::{Attr, Attrs, Element, lines, text};

fn full_box(ctx: &PrintCtx<'_>) -> Result<Vec<MaybeDecl>> {
    let size = &ctx.presentation.size;
    Ok(vec![
        decl("position", "absolute"),
        decl("left", "0pt"),
        decl("top", "0pt"),
        decl("width", pt(size.w.to_f64())?),
        decl("height", pt(size.h.to_f64())?),
    ])
}

/// A source colour in its resolved form, when it has its value.
fn resolved_color(color: &Color) -> Option<ResolvedColor> {
    match color {
        Color::Rgb(rgb) => Some(ResolvedColor::Rgb(rgb.clone())),
        Color::Theme(theme) => Some(ResolvedColor::Theme(ResolvedThemeColor {
            kind: ThemeTag,
            slot: theme.slot,
            mods: theme.mods.clone(),
            resolved: theme.resolved.clone()?,
        })),
    }
}

fn resolved_stops(stops: &[GradientStop<Color>]) -> Option<Vec<GradientStop<ResolvedColor>>> {
    stops
        .iter()
        .map(|stop| {
            Some(GradientStop {
                pos: stop.pos,
                color: resolved_color(&stop.color)?,
            })
        })
        .collect()
}

/// A failed container keeps its source background; it prints only when it is already in resolved form.
fn source_fill(fill: &Fill) -> Option<ResolvedFill> {
    match fill {
        Fill::None
        | Fill::LinearAngle(_)
        | Fill::RadialSource(_)
        | Fill::Path(_)
        | Fill::Odf(_) => None,
        Fill::Solid { color } => Some(ResolvedFill::Solid {
            color: resolved_color(color)?,
        }),
        Fill::Image { media, mode } => Some(ResolvedFill::Image {
            media: *media,
            mode: *mode,
        }),
        Fill::Linear(linear) => Some(ResolvedFill::Linear(LinearLowered {
            from: linear.from.clone(),
            to: linear.to.clone(),
            stops: resolved_stops(&linear.stops)?,
            steps: linear.steps,
        })),
        Fill::Radial(radial) => Some(ResolvedFill::Radial(RadialLowered {
            center: radial.center.clone(),
            radius: radial.radius,
            radius_y: radial.radius_y,
            focus: radial.focus.clone(),
            stops: resolved_stops(&radial.stops)?,
            steps: radial.steps,
        })),
        Fill::Rect(rect) => Some(ResolvedFill::Rect(Box::new(RectLowered {
            origin: rect.origin.clone(),
            axis_x: rect.axis_x.clone(),
            axis_y: rect.axis_y.clone(),
            inner: rect.inner.clone(),
            stops: resolved_stops(&rect.stops)?,
            steps: rect.steps,
        }))),
        Fill::Ellipse(ellipse) => Some(ResolvedFill::Ellipse(Box::new(EllipseLowered {
            origin: ellipse.origin.clone(),
            axis_x: ellipse.axis_x.clone(),
            axis_y: ellipse.axis_y.clone(),
            inner: ellipse.inner.clone(),
            stops: resolved_stops(&ellipse.stops)?,
            steps: ellipse.steps,
        }))),
    }
}

/// The background a container prints: a built one's fill, or a failed one's source fill in resolved form.
fn printable_fill<B>(
    body: &Body<B>,
    built: impl Fn(&B) -> Option<&ResolvedFill>,
) -> Option<ResolvedFill> {
    let fill = match body {
        Body::Built(b) => built(b).cloned(),
        Body::Failed { background, .. } => background.as_ref().and_then(source_fill),
    };
    fill.filter(|fill| !matches!(fill, ResolvedFill::None))
}

/// The background box: a colour or an image as CSS, a gradient as an SVG layer.
fn background_box(fill: &ResolvedFill, ctx: &mut PrintCtx<'_>) -> Result<Element> {
    let gradient = fill.is_gradient();
    let mut decls = full_box(ctx)?;
    let mut children = Vec::new();
    if gradient {
        let size = &ctx.presentation.size;
        let (w, h) = (size.w, size.h);
        let id = format!("g-{}-bg", file_id(ctx.container.as_str()));
        let paint = svg_fill(Some(fill), &PaintBox::of(w.to_f64(), h.to_f64()), &id, ctx)?;
        let outline = rect(Pt::from_steps(w.steps()), Pt::from_steps(h.steps()));
        let path = Element::new(
            "path",
            Attrs::new()
                .val("d", path_data(&outline, 1.0)?)
                .val("style", format!("fill:{}", paint.value)),
            Vec::new(),
        );
        let defs = Element::new("defs", Attrs::new(), lines(paint.defs));
        children.push(svg_layer(w.to_f64(), h.to_f64(), vec![defs, path])?.into());
    } else {
        decls.extend(background_decls(fill, ctx)?.into_iter().map(Some));
    }
    Ok(Element::new(
        "div",
        Attrs::new()
            .flag(BACKGROUND, true)
            .val("style", style(decls)),
        children,
    ))
}

fn failed_marker(code: &str, ctx: &PrintCtx<'_>) -> Result<Element> {
    let mut decls = full_box(ctx)?;
    decls.extend([
        decl("box-sizing", "border-box"),
        decl("outline", "2pt dashed #c00"),
        decl("outline-offset", "-2pt"),
        decl("font", "9pt sans-serif"),
        decl("color", "#c00"),
    ]);
    Ok(Element::new(
        "div",
        Attrs::new().flag(FAILED, true).val("style", style(decls)),
        vec![text(code)],
    ))
}

struct Contents<'a> {
    title: String,
    main: Attrs,
    background: Option<ResolvedFill>,
    elements: Vec<&'a IrElement>,
    /// Element ids in reading order: the document lists the elements so.
    reading_order: Option<&'a [ElementId]>,
    failed: Option<&'a str>,
    /// The timeline template, last in the head.
    timeline: Option<Element>,
}

/// The printed elements in reading order, so assistive technology reads them so, hidden ones (never read) last.
/// When that differs from the paint order, each element takes its paint position as its `z-index`, which keeps the
/// stacking and, being positive, keeps the slide above its layout's elements.
fn in_reading_order(
    elements: &[&IrElement],
    printed: Vec<Element>,
    reading_order: Option<&[ElementId]>,
) -> Vec<Element> {
    let mut rank: IndexMap<&ElementId, usize> = IndexMap::new();
    for (index, id) in reading_order.into_iter().flatten().enumerate() {
        rank.insert(id, index);
    }
    let mut order: Vec<(usize, usize)> = elements
        .iter()
        .enumerate()
        .take(printed.len())
        .map(|(z, element)| (z, rank.get(element.id()).copied().unwrap_or(usize::MAX)))
        .collect();
    order.sort_by_key(|&(z, rank)| (rank, z));
    if order.iter().enumerate().all(|(index, (z, _))| *z == index) {
        return printed;
    }
    let mut slots: Vec<Option<Element>> = printed.into_iter().map(Some).collect();
    order
        .into_iter()
        .filter_map(|(z, _)| {
            let mut node = slots.get_mut(z)?.take()?;
            let z_index = format!("z-index:{}", z + 1);
            let css = match node.attrs.get("style") {
                Some(Attr::Value(css)) if !css.is_empty() => format!("{css};{z_index}"),
                _ => z_index,
            };
            node.attrs.set("style", Attr::Value(css));
            Some(node)
        })
        .collect()
}

fn document(ctx: &mut PrintCtx<'_>, body: Contents<'_>) -> Result<String> {
    let mut children = Vec::new();
    if let Some(background) = &body.background {
        children.push(background_box(background, ctx)?);
    }
    match body.failed {
        Some(code) => children.push(failed_marker(code, ctx)?),
        None => {
            let printed = body
                .elements
                .iter()
                .map(|element| ctx.print(element))
                .collect::<Result<Vec<_>>>()?;
            children.extend(in_reading_order(
                &body.elements,
                printed,
                body.reading_order,
            ));
        }
    }
    let size = &ctx.presentation.size;
    let css = style([
        decl("position", "relative"),
        decl("width", pt(size.w.to_f64())?),
        decl("height", pt(size.h.to_f64())?),
        decl("overflow", "hidden"),
        // Below two device pixels per CSS pixel Chromium rounds each glyph advance to a whole pixel, so a line's
        // width and its breaks would depend on the zoom; presentation apps lay text out at exact advances.
        decl("text-rendering", "geometricPrecision"),
    ]);
    let main = Element::new("main", body.main.val("style", css), lines(children));
    let mut head = vec![
        Element::new(
            "meta",
            Attrs::new().val("charset", "utf-8".to_owned()),
            Vec::new(),
        ),
        Element::new("title", Attrs::new(), vec![text(body.title)]),
        Element::new("style", Attrs::new(), vec![text(RESET_STYLE)]),
    ];
    head.extend(body.timeline);
    let head = Element::new("head", Attrs::new(), lines(head));
    let html = Element::new(
        "html",
        Attrs::new(),
        lines(vec![
            head,
            Element::new("body", Attrs::new(), lines(vec![main])),
        ]),
    );
    Ok(serialize::document(&html))
}

/// A slide's document: its own background (when it has one) and elements; its layout composes behind it.
pub fn slide_document(slide: &Slide, ctx: &mut PrintCtx<'_>) -> Result<String> {
    let failed = slide.body.failed().map(|failed| failed.code.as_str());
    let (title, reading_order) = match &slide.body {
        Body::Built(built) => (built.title.clone(), Some(built.reading_order.as_slice())),
        Body::Failed { .. } => (
            slide
                .failed_a11y
                .as_ref()
                .and_then(|kept| kept.title.clone())
                .unwrap_or_default(),
            None,
        ),
    };
    let main = Attrs::new()
        .val(attr::SLIDE, slide.id.to_string())
        .val(attr::LAYOUT, slide.layout.as_ref().map(ToString::to_string))
        .val(
            attr::SHOW_LAYOUT,
            (!slide.show_layout).then(|| "false".to_owned()),
        )
        .val(attr::FAILED, failed.map(str::to_owned));
    let timeline = match ctx.motion {
        Motion::Off => None,
        Motion::On => {
            let hooks = paint_motion::slide_hooks(slide, ctx.presentation);
            ctx.set_hooks(hooks);
            let steps: Vec<TimelineStep> = paint_motion::played_steps(slide, ctx.presentation)
                .into_iter()
                .map(|played| played.step.into_owned())
                .collect();
            timeline_template(&steps)?
        }
    };
    let body = Contents {
        title,
        main,
        background: printable_fill(&slide.body, |b| b.background.as_ref()),
        elements: slide.body.elements().iter().collect(),
        reading_order,
        failed,
        timeline,
    };
    document(ctx, body)
}

/// A layout's document: its effective background, then the non-placeholder elements of its shown master and its
/// own.
pub fn layout_document(
    layout: &Layout,
    master: Option<&Master>,
    ctx: &mut PrintCtx<'_>,
) -> Result<String> {
    let failed = layout.body.failed().map(|failed| failed.code.as_str());
    let not_placeholder = |element: &&IrElement| element.placeholder().is_none();
    let shown = master.filter(|master| layout.show_master && master.body.failed().is_none());
    let mut elements: Vec<&IrElement> = shown
        .map(|master| {
            master
                .body
                .elements()
                .iter()
                .filter(not_placeholder)
                .collect()
        })
        .unwrap_or_default();
    elements.extend(layout.body.elements().iter().filter(not_placeholder));
    let background = printable_fill(&layout.body, |b| b.background.as_ref()).or_else(|| {
        master.and_then(|master| printable_fill(&master.body, |b| b.background.as_ref()))
    });
    let body = Contents {
        title: layout.name.clone().unwrap_or_else(|| layout.id.to_string()),
        main: Attrs::new()
            .val(attr::LAYOUT_ROOT, layout.id.to_string())
            .val(attr::FAILED, failed.map(str::to_owned)),
        background,
        elements,
        reading_order: None,
        failed,
        timeline: None,
    };
    document(ctx, body)
}
