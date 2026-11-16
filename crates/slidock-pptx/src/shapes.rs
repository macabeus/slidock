//! The shape tree (`p:spTree`) in z-order: shapes, connectors, groups, pictures and graphic frames. Geometry, paint and
//! lines are read as declared; a style reference fills in what the shape leaves unset. Graphic frames hold tables;
//! charts, diagrams, OLE objects and a table without a grid become Foreign nodes (with an OLE object's picture as
//! fallback), which the core diagnoses.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use slidock_canon::is_js_whitespace;
use slidock_core_api::lowering::LowerError;
use slidock_core_api::visit::walk_elements;
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::elements::{ForeignFallback, ForeignType};
use slidock_ir::features::FeatureId;
use slidock_ir::formats::FileFormat;
use slidock_ir::geometry::Geometry;
use slidock_ir::json_value::{Ext, JsonValue};
use slidock_ir::origin::Origin;
use slidock_ir::paint::Fill;
use slidock_ir::shadow::SourceShadow;
use slidock_ir::source::{Element as IrElement, Foreign, Group, Shape};

use crate::clip;
use crate::context::Lowering;
use crate::element::{Base, BodyCx, element_base, frame_of, no_frame};
use crate::fill::{PaintCx, fill_element, fill_features, read_fill};
use crate::geometry::{ReadGeometry, read_child_space, read_frame, read_geometry};
use crate::line::read_line;
use crate::picture::lower_picture;
use crate::report::Report;
use crate::shadow::element_shadow;
use crate::style_refs::read_style;
use crate::table::{LoweredTable, lower_table};
use crate::text::read_text_body;
use crate::xml::{El, Element, Ns};

/// Where in the tree an element is, and what its enclosing groups pass down.
#[derive(Clone)]
struct Scope {
    path: String,
    depth: usize,
    group_fill: Option<Fill>,
}

/// The prefixes of `mc:Choice` requirements this reader understands; anything else reads the fallback.
const UNDERSTOOD: [&str; 5] = ["p14", "p15", "a14", "a16", "ahyp"];

fn graphic_type(uri: &str) -> ForeignType {
    match uri {
        "http://schemas.openxmlformats.org/drawingml/2006/table" => ForeignType::Table,
        "http://schemas.openxmlformats.org/drawingml/2006/chart"
        | "http://schemas.microsoft.com/office/drawing/2014/chartex" => ForeignType::Chart,
        "http://schemas.openxmlformats.org/drawingml/2006/diagram" => ForeignType::Smartart,
        "http://schemas.openxmlformats.org/presentationml/2006/ole" => ForeignType::Ole,
        _ => ForeignType::Unknown,
    }
}

fn requirements(choice: &Element) -> Vec<&str> {
    choice
        .attr("Requires")
        .unwrap_or("")
        .split(is_js_whitespace)
        .filter(|prefix| !prefix.is_empty())
        .collect()
}

/// The branch of an `mc:AlternateContent` to read: the first choice whose requirements are understood, else the
/// fallback (PX2005).
fn alternative<'e>(element: &'e Element, report: &mut dyn Report) -> Option<(&'e Element, String)> {
    let mut position = 0;
    for choice in element.kids() {
        if !choice.is(Ns::Mc, "Choice") {
            continue;
        }
        position += 1;
        if requirements(choice)
            .iter()
            .all(|prefix| UNDERSTOOD.contains(prefix))
        {
            return Some((choice, format!("mc:Choice[{position}]")));
        }
    }
    let fallback = element.child(Ns::Mc, "Fallback")?;
    let requires: Vec<&str> = element
        .kids()
        .filter(|choice| choice.is(Ns::Mc, "Choice"))
        .filter_map(|choice| choice.attr("Requires"))
        .filter(|requires| !requires.is_empty())
        .collect();
    report.report(
        codes::PX2005,
        DiagnosticSite::new().data("requires", requires.join(", ")),
    );
    Some((fallback, "mc:Fallback".to_owned()))
}

/// The element children of a tree or group with their paths, alternate content resolved.
pub(crate) fn tree_children<'e>(
    parent: &'e Element,
    report: &mut dyn Report,
    path: &str,
) -> Vec<(&'e Element, String)> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut out = Vec::new();
    for element in parent.kids() {
        let n = counts.entry(&element.name).or_insert(0);
        *n += 1;
        let own = format!("{path}/{}[{n}]", element.name);
        if element.is(Ns::Mc, "AlternateContent") {
            if let Some((branch, step)) = alternative(element, report) {
                out.extend(tree_children(branch, report, &format!("{own}/{step}")));
            }
        } else {
            out.push((&**element, own));
        }
    }
    out
}

/// How a shape's paint reports, resolves pictures and fills in `grpFill`.
fn paint_for<'l, 'a>(
    ctx: &'l mut Lowering<'a>,
    node: &Base,
    scope: &Scope,
    feature: FeatureId,
) -> PaintCx<'l, 'a> {
    PaintCx::new(ctx, Some(node.id.clone()))
        .with_pictures(feature)
        .with_group_fill(scope.group_fill.clone())
}

fn geometry_feature(geometry: &Geometry) -> FeatureId {
    match geometry {
        Geometry::Preset(preset) if preset.name.is_connector() => FeatureId::GeometryConnector,
        Geometry::Preset(_) => FeatureId::GeometryPreset,
        _ => FeatureId::GeometryCustom,
    }
}

/// A connector's ends: the shapes it is glued to, kept in `ext.pptx.connection`.
fn connection(element: &Element) -> Option<JsonValue> {
    let props = element.path(&[(Ns::P, "nvCxnSpPr"), (Ns::P, "cNvCxnSpPr")]);
    let end = |local: &str| {
        let glued = props.child(Ns::A, local)?;
        let mut out = IndexMap::new();
        out.insert(
            "id".to_owned(),
            JsonValue::string(glued.attr("id").unwrap_or("")),
        );
        out.insert(
            "idx".to_owned(),
            JsonValue::number(glued.int_attr("idx").unwrap_or(0) as f64),
        );
        Some(JsonValue::Object(out))
    };
    let start = end("stCxn");
    let finish = end("endCxn");
    if start.is_none() && finish.is_none() {
        return None;
    }
    let mut out = IndexMap::new();
    if let Some(start) = start {
        out.insert("start".to_owned(), start);
    }
    if let Some(finish) = finish {
        out.insert("end".to_owned(), finish);
    }
    Some(JsonValue::Object(out))
}

fn foreign(
    base: Base,
    frame: Option<slidock_ir::frame::Frame>,
    foreign_type: ForeignType,
) -> Foreign {
    Foreign {
        id: base.id,
        origin: base.origin,
        name: base.name,
        placeholder: base.placeholder,
        inherits: None,
        match_key: None,
        hidden: base.hidden,
        opacity: None,
        link: base.link,
        alt: base.alt,
        ext: None,
        frame,
        dialect: FileFormat::Pptx,
        foreign_type,
        payload: None,
        fallback: None,
    }
}

fn object(entries: Vec<(&str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn lower_shape(
    element: &Element,
    path: &str,
    scope: &Scope,
    ctx: &mut Lowering<'_>,
) -> Result<IrElement, LowerError> {
    let base = element_base(element, path, ctx)?;
    let sp_pr = element.child(Ns::P, "spPr").map(|rc| &**rc);
    let xfrm = sp_pr.child(Ns::A, "xfrm");
    let frame = frame_of(xfrm, &base, ctx);
    let ext = xfrm.child(Ns::A, "ext");
    let size = (
        ext.int_attr("cx").unwrap_or(0),
        ext.int_attr("cy").unwrap_or(0),
    );
    let geometry = match read_geometry(sp_pr, size, ctx.limits.path_segments)? {
        ReadGeometry::Unknown(detail) => {
            ctx.seen(FeatureId::ElementOther);
            let mut node = foreign(base, frame, ForeignType::Unknown);
            node.payload = Some(object(vec![(
                "geometry",
                JsonValue::string(clip(&detail, 256)),
            )]));
            return Ok(IrElement::Foreign(Box::new(node)));
        }
        ReadGeometry::Geometry(geometry, nan) => {
            ctx.not_a_number(&base.id, "geometry", nan);
            Some(geometry)
        }
        ReadGeometry::None => None,
    };
    ctx.seen(FeatureId::ElementShape);
    if let Some(geometry) = &geometry {
        ctx.seen(geometry_feature(geometry));
    }

    let mut paint = paint_for(ctx, &base, scope, FeatureId::FillImage);
    let style = read_style(element.child(Ns::P, "style").map(|rc| &**rc), &mut paint)?;
    if style.is_some() && base.placeholder.is_some() {
        paint
            .ctx
            .report_on(&base.id, codes::PX2006, DiagnosticSite::new());
    }
    let own = fill_element(sp_pr);
    for feature in fill_features(own) {
        paint.ctx.seen(feature);
    }
    let fill = match own {
        Some(own) => read_fill(own, &mut paint, None)?,
        None => style.as_ref().and_then(|style| style.fill.clone()),
    };
    let stroke = read_line(
        sp_pr.child(Ns::A, "ln"),
        style.as_ref().and_then(|style| style.line.as_ref()),
        &mut paint,
        true,
    );
    let shadow = element_shadow(
        sp_pr,
        style.as_ref().and_then(|style| style.effect.as_ref()),
        &base.id,
        ctx,
    );
    let text = match element.child(Ns::P, "txBody") {
        Some(tx_body) => {
            let font = style
                .as_ref()
                .and_then(|style| style.font.as_ref())
                .map(|font| (font.slot, font.color.as_ref()));
            let mut cx = BodyCx {
                ctx,
                node: base.id.clone(),
            };
            Some(read_text_body(tx_body, &mut cx, font)?)
        }
        None => None,
    };

    let linked = if &*element.local == "cxnSp" {
        connection(element)
    } else {
        None
    };
    let mut pptx = IndexMap::new();
    if let Some(style) = style {
        pptx.insert("styleRef".to_owned(), style.raw);
    }
    if let Some(linked) = linked {
        pptx.insert("connection".to_owned(), linked);
    }
    Ok(IrElement::Shape(Box::new(Shape {
        id: base.id,
        origin: base.origin,
        name: base.name,
        placeholder: base.placeholder,
        inherits: None,
        match_key: None,
        hidden: base.hidden,
        opacity: None,
        link: base.link,
        alt: base.alt,
        ext: (!pptx.is_empty()).then(|| Ext {
            pptx: Some(pptx),
            ..Ext::default()
        }),
        frame,
        geometry,
        fill,
        stroke,
        text,
        shadow,
    })))
}

/// A group into `out`. Its children lower through [`lower_children`]; what this frame holds stays small, as
/// groups recurse (an element is kilobytes, and an unoptimised build keeps every temporary).
fn lower_group(
    element: &Element,
    path: &str,
    scope: &Scope,
    ctx: &mut Lowering<'_>,
    out: &mut Vec<IrElement>,
) -> Result<(), LowerError> {
    let limit = ctx.limits.group_depth;
    if scope.depth + 1 > limit {
        return Err(LowerError::Limit(format!(
            "groups nest deeper than {limit}"
        )));
    }
    let base = element_base(element, path, ctx)?;
    let grp_sp_pr = element.child(Ns::P, "grpSpPr").map(|rc| &**rc);
    let xfrm = grp_sp_pr.child(Ns::A, "xfrm");
    let frame = frame_of(xfrm, &base, ctx).unwrap_or_else(no_frame);
    let child_space = read_child_space(xfrm, read_frame(xfrm).as_ref());
    let group_fill = match fill_element(grp_sp_pr) {
        Some(own) => {
            let mut paint = paint_for(ctx, &base, scope, FeatureId::FillImage);
            read_fill(own, &mut paint, None)?
        }
        None => scope.group_fill.clone(),
    };
    let shadow = element_shadow(grp_sp_pr, None, &base.id, ctx);
    ctx.seen(FeatureId::ElementGroup);
    let inner = Scope {
        path: path.to_owned(),
        depth: scope.depth + 1,
        group_fill,
    };
    let children = lower_children(element, &inner, ctx)?;
    push_group(base, frame, child_space, children, shadow, out);
    Ok(())
}

#[inline(never)]
fn push_group(
    base: Base,
    frame: slidock_ir::frame::Frame,
    child_space: Option<slidock_ir::frame::ChildSpace>,
    children: Vec<IrElement>,
    shadow: Option<Option<SourceShadow>>,
    out: &mut Vec<IrElement>,
) {
    out.push(IrElement::Group(Box::new(Group {
        id: base.id,
        origin: base.origin,
        name: base.name,
        placeholder: base.placeholder,
        inherits: None,
        match_key: None,
        hidden: base.hidden,
        opacity: None,
        link: base.link,
        alt: base.alt,
        ext: None,
        frame: Some(frame),
        child_space,
        children,
        shadow,
    })));
}

/// The picture an OLE object shows in its place, wherever its writer put it.
fn ole_picture(graphic_data: Option<&Element>) -> Option<&Element> {
    for element in graphic_data.kids() {
        if element.is(Ns::P, "pic") {
            return Some(element);
        }
        if let Some(found) = ole_picture(Some(element)) {
            return Some(found);
        }
    }
    None
}

fn lower_graphic_frame(
    element: &Element,
    path: &str,
    scope: &Scope,
    ctx: &mut Lowering<'_>,
) -> Result<IrElement, LowerError> {
    let base = element_base(element, path, ctx)?;
    let xfrm = element.child(Ns::P, "xfrm").map(|rc| &**rc);
    let graphic_data = element.path(&[(Ns::A, "graphic"), (Ns::A, "graphicData")]);
    let uri = graphic_data.attr("uri").unwrap_or("");
    let foreign_type = graphic_type(uri);
    let tbl = graphic_data
        .child(Ns::A, "tbl")
        .filter(|_| foreign_type == ForeignType::Table);
    let mut reason = None;
    if let Some(tbl) = tbl {
        let paint = paint_for(ctx, &base, scope, FeatureId::FillImage);
        match lower_table(tbl, &base, xfrm, paint)? {
            LoweredTable::Table(table) => return Ok(IrElement::Table(Box::new(*table))),
            LoweredTable::Refused(why) => reason = Some(why),
        }
    }
    let frame = frame_of(xfrm, &base, ctx);
    let feature = foreign_type.feature();
    ctx.seen(feature);
    let picture = if foreign_type == ForeignType::Ole {
        ole_picture(graphic_data)
    } else {
        None
    };
    let media = match picture.child(Ns::P, "blipFill") {
        Some(blip_fill) => crate::element::blip_media(blip_fill, ctx, Some(&base.id), feature)?,
        None => None,
    };
    let mut entries = vec![("uri", JsonValue::string(clip(uri, 256)))];
    if let Some(reason) = reason {
        entries.push(("reason", JsonValue::string(reason)));
    }
    let mut node = foreign(base, frame, foreign_type);
    node.payload = Some(object(entries));
    node.fallback = media.map(|media| ForeignFallback {
        media: Some(media),
        elements: None,
    });
    Ok(IrElement::Foreign(Box::new(node)))
}

/// An element that holds no other element, into `out`: kept out of the recursion's frames.
#[inline(never)]
fn lower_leaf(
    element: &Element,
    path: &str,
    scope: &Scope,
    ctx: &mut Lowering<'_>,
    out: &mut Vec<IrElement>,
) -> Result<(), LowerError> {
    let lowered = match &*element.local {
        "sp" | "cxnSp" => Some(lower_shape(element, path, scope, ctx)?),
        "graphicFrame" => Some(lower_graphic_frame(element, path, scope, ctx)?),
        "pic" => lower_picture(element, path, ctx)?,
        "contentPart" => {
            ctx.seen(FeatureId::ElementOther);
            let origin = Origin::Source {
                part: ctx.part.clone(),
                path: Some(clip(path, 512)),
                object: None,
            };
            ctx.report(
                codes::PX2009,
                DiagnosticSite::new()
                    .origin(origin)
                    .data("element", &*element.name),
            );
            None
        }
        _ => None,
    };
    out.extend(lowered);
    Ok(())
}

fn lower_children(
    parent: &Element,
    scope: &Scope,
    ctx: &mut Lowering<'_>,
) -> Result<Vec<IrElement>, LowerError> {
    let mut out = Vec::new();
    for (element, path) in tree_children(parent, ctx, &scope.path) {
        if element.ns != Ns::P {
            continue;
        }
        if &*element.local == "grpSp" {
            lower_group(element, &path, scope, ctx, &mut out)?;
        } else {
            lower_leaf(element, &path, scope, ctx, &mut out)?;
        }
    }
    Ok(out)
}

/// A container's shape tree; more than `elements_per_container` elements fail the container.
pub(crate) fn lower_tree(
    sp_tree: Option<&Element>,
    path: &str,
    ctx: &mut Lowering<'_>,
) -> Result<Vec<IrElement>, LowerError> {
    let Some(sp_tree) = sp_tree else {
        return Ok(Vec::new());
    };
    let scope = Scope {
        path: path.to_owned(),
        depth: 0,
        group_fill: None,
    };
    let elements = lower_children(sp_tree, &scope, ctx)?;
    let limit = ctx.limits.elements_per_container;
    if walk_elements(&elements).count() > limit {
        return Err(LowerError::Limit(format!("more than {limit} elements")));
    }
    Ok(elements)
}
