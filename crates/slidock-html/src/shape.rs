//! A shape: a plain rectangle is the div itself (its fill a background an editor restyles inline); any other geometry
//! is an SVG layer of its lowered paths. Text is a layer above that geometry, else the div itself. A shadow is the
//! root's: the box's own for an opaque plain rectangle, else that of everything the shape draws.

use slidock_format::paths::file_id;
use slidock_ir::geometry::{FillRule, Path, PathRole};
use slidock_ir::paint::{ResolvedFill, ResolvedStroke};
use slidock_ir::resolved::Shape;
use slidock_ir::units::Pt;

use crate::attributes::{Kind, Root, common, element_attributes};
use crate::color::css_color;
use crate::context::PrintCtx;
use crate::css::{MaybeDecl, decl, fixed, style};
use crate::error::Result;
use crate::frame::{Autosize, box_decls, is_empty_box, opacity_decl};
use crate::gradient::PaintBox;
use crate::paint::{stroke_attributes, stroke_color, strokes, svg_fill};
use crate::paint_motion::Hook;
use crate::shadow::{ShadowForm, shadow_decl};
use crate::svg::{path_data, rect, svg_layer};
use crate::text::{print_text, text_element, text_placement};
use crate::tree::{Attrs, Element, lines};

/// Whether a path is exactly the frame's rectangle, as lower-geometry writes a `rect`.
fn is_frame_rect(path: &Path, shape: &Shape) -> bool {
    let (w, h) = (
        Pt::from_steps(shape.frame.w.steps()),
        Pt::from_steps(shape.frame.h.steps()),
    );
    path.fill && path.segments == rect(w, h)
}

fn is_plain_rect(shape: &Shape) -> bool {
    let geometry = &shape.geometry;
    geometry
        .preset
        .is_some_and(|preset| preset.as_str() == "rect")
        && matches!(geometry.paths.as_slice(), [path] if is_frame_rect(path, shape))
        && strokes(shape.stroke.as_ref().and_then(Option::as_ref)).is_none()
        && matches!(
            shape.fill,
            None | Some(ResolvedFill::None) | Some(ResolvedFill::Solid { .. })
        )
}

struct PathPaint {
    fill: String,
    stroke: Option<String>,
    attributes: Attrs,
}

fn path_element(path: &Path, paint: PathPaint) -> Result<Element> {
    let css = style([
        decl("fill", paint.fill),
        paint.stroke.map(|stroke| ("stroke", stroke)),
    ]);
    Ok(Element::new(
        "path",
        Attrs::new()
            .val("d", path_data(&path.segments, 1.0)?)
            .val(
                "fill-rule",
                (path.fill_rule == FillRule::Evenodd).then(|| "evenodd".to_owned()),
            )
            .with(paint.attributes)
            .val("style", css),
        Vec::new(),
    ))
}

/// The SVG layer of a shape's paths, or nothing when no path paints.
fn geometry_layer(shape: &Shape, ctx: &mut PrintCtx<'_>) -> Result<Option<Element>> {
    let frame = &shape.frame;
    let id = format!("g-{}", file_id(shape.id.as_str()));
    let area = PaintBox::of(frame.w.to_f64(), frame.h.to_f64()).with_paths(&shape.geometry.paths);
    let fill = svg_fill(shape.fill.as_ref(), &area, &id, ctx)?;
    let mut fill = fill;
    if let Some(value) = ctx.hook(shape.id.as_str(), Hook::Fill) {
        fill.value = Hook::Fill.var(value);
    }
    let line_hook = ctx
        .hook(shape.id.as_str(), Hook::Line)
        .map(|value| Hook::Line.var(value));
    let stroke_paint = |stroke: &ResolvedStroke| -> Result<String> {
        match &line_hook {
            Some(var) => Ok(var.clone()),
            None => stroke_color(stroke),
        }
    };
    let stroke: Option<&ResolvedStroke> = strokes(shape.stroke.as_ref().and_then(Option::as_ref));
    let mut paths = Vec::new();
    for path in &shape.geometry.paths {
        if path.role == Some(PathRole::LineEnd) {
            // Arrowheads paint in the stroke's colour, filled or outlined as their path says.
            let Some(stroke) = stroke else { continue };
            let color = stroke_paint(stroke)?;
            let attributes = if path.stroke {
                Attrs::new().val("stroke-width", fixed(stroke.width.to_f64())?)
            } else {
                Attrs::new()
            };
            paths.push(path_element(
                path,
                PathPaint {
                    fill: if path.fill {
                        color.clone()
                    } else {
                        "none".to_owned()
                    },
                    stroke: path.stroke.then_some(color),
                    attributes,
                },
            )?);
            continue;
        }
        let fills = path.fill && fill.value != "none";
        let outlined = if path.stroke { stroke } else { None };
        if !fills && outlined.is_none() {
            continue;
        }
        let paint = match outlined {
            Some(stroke) => PathPaint {
                fill: if fills {
                    fill.value.clone()
                } else {
                    "none".to_owned()
                },
                stroke: Some(stroke_paint(stroke)?),
                attributes: stroke_attributes(stroke, fixed)?,
            },
            None => PathPaint {
                fill: fill.value.clone(),
                stroke: None,
                attributes: Attrs::new(),
            },
        };
        paths.push(path_element(path, paint)?);
    }
    if paths.is_empty() {
        return Ok(None);
    }
    let mut children = Vec::with_capacity(paths.len() + 1);
    if !fill.defs.is_empty() {
        children.push(Element::new("defs", Attrs::new(), lines(fill.defs)));
    }
    children.extend(paths);
    Ok(Some(svg_layer(
        frame.w.to_f64(),
        frame.h.to_f64(),
        children,
    )?))
}

/// How a shape's shadow draws: a plain rectangle whose opaque fill paints its whole box casts its box, unless it holds
/// text whose overflow casts too; anything else casts what it draws (geometry, outline, line ends, glyphs, overflowing
/// text), its translucent fill showing the shadow through.
fn shadow_form(shape: &Shape, plain: bool, autosize: Option<&Autosize>) -> ShadowForm {
    let opaque = matches!(
        &shape.fill,
        Some(ResolvedFill::Solid { color }) if color.rgb().a.to_f64() >= 1.0
    );
    let overflow_casts = shape.text.is_some()
        && shape
            .shadow
            .as_ref()
            .is_some_and(|shadow| shadow.overflow_casts);
    if plain && opaque && !overflow_casts && !is_empty_box(&shape.frame, autosize) {
        ShadowForm::Box
    } else {
        ShadowForm::Drop
    }
}

pub fn print(shape: &Shape, ctx: &mut PrintCtx<'_>) -> Result<Element> {
    let text = match &shape.text {
        Some(body) => Some(print_text(body, &shape.frame, ctx, &shape.id)?),
        None => None,
    };
    let plain = is_plain_rect(shape);
    let background = match (&shape.fill, plain) {
        (Some(ResolvedFill::Solid { color }), true) => {
            Some(match ctx.hook(shape.id.as_str(), Hook::Fill) {
                Some(value) => Hook::Fill.var(value),
                None => css_color(color)?,
            })
        }
        _ => None,
    };
    let geometry = if plain {
        None
    } else {
        geometry_layer(shape, ctx)?
    };
    let autosize = text.as_ref().and_then(|text| text.autosize.as_ref());
    // A hooked element declares its hooks' static values, which its paint reads (the deck path only).
    let mut decls: Vec<MaybeDecl> = ctx
        .hooks(shape.id.as_str())
        .into_iter()
        .flatten()
        .map(|(hook, value)| Some((hook.property(), value.clone())))
        .collect();
    decls.extend(box_decls(&shape.frame, autosize)?);
    decls.push(opacity_decl(shape.opacity)?);
    decls.push(background.map(|color| ("background-color", color)));
    let shadow = shadow_decl(
        shape.shadow.as_ref(),
        &ctx.axes.then_frame(&shape.frame),
        shadow_form(shape, plain, autosize),
    )?;
    // The shape is its own text box, unless its geometry layer would paint over the text in flow, it is hidden (an
    // inline display would show it), or its text turns apart from it.
    match text {
        Some(text)
            if geometry.is_none() && shape.hidden != Some(true) && text.text_box.turn.is_none() =>
        {
            let text_box = text.text_box;
            let padded = text_box.padding.is_some();
            decls.push(padded.then(|| ("box-sizing", "border-box".to_owned())));
            decls.push(text_box.padding);
            decls.extend(text_box.column);
            decls.extend(text_box.inherited);
            decls.push(shadow);
            let attrs = element_attributes(
                common!(shape, Kind::Shape(shape.text.as_ref())),
                ctx,
                Root::Div,
            )?
            .with(text_box.attributes)
            .val("style", style(decls));
            Ok(Element::new("div", attrs, lines(text_box.blocks)))
        }
        text => {
            let mut layers: Vec<Element> = geometry.into_iter().collect();
            if let Some(text) = text {
                let placement = text_placement(text.autosize.as_ref());
                layers.push(text_element(text.text_box, placement));
            }
            decls.push(shadow);
            let attrs = element_attributes(
                common!(shape, Kind::Shape(shape.text.as_ref())),
                ctx,
                Root::Div,
            )?
            .val("style", style(decls));
            Ok(Element::new("div", attrs, lines(layers)))
        }
    }
}
