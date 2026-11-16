//! The drawables of a page or master page, in z-order: custom shapes, rectangles, ellipses, lines, polylines, polygons,
//! paths and connectors as shapes; frames by what they hold (pictures, media, embedded objects and tables); groups,
//! whose children ODF places on the page, so the group's box is their union and their frames are moved into it.
//! Embedded objects, tables whose grid the IR cannot hold, 3D scenes and custom shapes outside the preset map become
//! Foreign nodes (with a replacement image as fallback when the frame holds one), which the core diagnoses.

use indexmap::IndexMap;
use slidock_core_api::lowering::LowerError;
use slidock_core_api::visit::walk_elements;
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::elements::{ForeignFallback, ForeignType, MediaKind};
use slidock_ir::features::FeatureId;
use slidock_ir::formats::FileFormat;
use slidock_ir::frame::Frame;
use slidock_ir::geometry::Geometry;
use slidock_ir::ids::{NodeId, Sha256};
use slidock_ir::json_value::JsonValue;
use slidock_ir::source::{Element as IrElement, Foreign, Group, MediaElement, Picture, Shape};
use slidock_ir::text::Link;
use slidock_ir::units::Angle;

use crate::context::Lowering;
use crate::document::CONTENT;
use crate::element::{Base, Flips, element_base, frame_of, no_frame, style_of};
use crate::geometry::{
    ReadGeometry, custom_shape_geometry, ellipse_geometry, line_geometry, path_geometry,
    rect_geometry,
};
use crate::graphic::{
    Paint, read_fill, read_shadow, read_stroke, report_undrawn_shadow, shadow_field,
};
use crate::ids::native_id;
use crate::links::read_href;
use crate::media::{SvgCheck, check_svg_part};
use crate::names::qn;
use crate::odf::{Element, Ns};
use crate::package::{Reference, resolve_reference};
use crate::picture::{lower_picture, picture_media};
use crate::svg_path::Budget;
use crate::table::lower_table;
use crate::text::{ShapeText, TextContext, read_text_body};
use crate::transform::bounds;
use crate::units::{pt, size, to_pt};

/// Where in the tree a drawable is, and what its enclosing hyperlink and groups pass down.
#[derive(Clone)]
struct Scope {
    path: String,
    depth: usize,
    link: Option<Link>,
}

/// The drawing elements that count as drawables (their index is the id fallback).
const DRAWABLES: [&str; 16] = [
    "custom-shape",
    "rect",
    "line",
    "polyline",
    "polygon",
    "regular-polygon",
    "path",
    "circle",
    "ellipse",
    "connector",
    "caption",
    "measure",
    "control",
    "page-thumbnail",
    "frame",
    "g",
];

fn is_drawable(element: &Element) -> bool {
    (element.in_namespace(Ns::Draw) && DRAWABLES.contains(&element.local.as_str()))
        || element.is(Ns::Dr3d, "scene")
}

/// A payload of string members.
fn payload(members: &[(&str, &str)]) -> JsonValue {
    JsonValue::Object(
        members
            .iter()
            .map(|(k, v)| ((*k).to_owned(), JsonValue::string(*v)))
            .collect::<IndexMap<_, _>>(),
    )
}

fn foreign<'a>(
    base: Base,
    frame: Option<Frame>,
    foreign_type: ForeignType,
    payload: JsonValue,
    ctx: &Lowering<'a, '_>,
    fallback: Option<Sha256>,
) -> IrElement {
    ctx.see(foreign_type.feature());
    IrElement::Foreign(Box::new(Foreign {
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
        dialect: FileFormat::Odp,
        foreign_type,
        payload: Some(payload),
        fallback: fallback.map(|media| ForeignFallback {
            media: Some(media),
            elements: None,
        }),
    }))
}

/// A shape: its geometry, the fill, line and effects of its style chain, and its text (in a frame, its text box).
fn lower_shape<'a>(
    element: &'a Element,
    base: Base,
    read: ReadGeometry,
    frame: Option<Frame>,
    ctx: &Lowering<'a, '_>,
    text_box: Option<&'a Element>,
) -> Result<IrElement, LowerError> {
    let (geometry, dropped_modifiers) = match read {
        ReadGeometry::Unknown(detail) => {
            return Ok(foreign(
                base,
                frame,
                ForeignType::Unknown,
                payload(&[("drawType", &detail)]),
                ctx,
                None,
            ));
        }
        ReadGeometry::Unreadable { kind, reason } => {
            return Ok(foreign(
                base,
                frame,
                ForeignType::Unknown,
                payload(&[("drawType", &kind), ("reason", &reason)]),
                ctx,
                None,
            ));
        }
        ReadGeometry::Geometry {
            geometry,
            dropped_modifiers,
            ..
        } => (geometry, dropped_modifiers),
    };
    ctx.see(FeatureId::ElementShape);
    ctx.see(match &geometry {
        Geometry::Preset(preset) if preset.name.is_connector() => FeatureId::GeometryConnector,
        Geometry::Preset(_) => FeatureId::GeometryPreset,
        _ => FeatureId::GeometryCustom,
    });
    if dropped_modifiers {
        let name = match &geometry {
            Geometry::Preset(preset) => preset.name.as_str(),
            _ => "",
        };
        ctx.report(
            codes::OD9109,
            DiagnosticSite::new()
                .node(base.id.clone())
                .data("preset", name),
        );
    }
    let props = style_of(element, ctx);
    let paint = Paint::of(*ctx, Some(&base.id));
    let fill = read_fill(&props.graphic, &paint)?;
    let line = read_stroke(&props.graphic, &paint);
    for feature in fill.features.iter().chain(&line.features).copied() {
        ctx.see(feature);
    }
    let shadow = shadow_field(read_shadow(&props.graphic), base.placeholder.is_some(), ctx);
    let custom_shape = element.is(Ns::Draw, "custom-shape");
    let text = read_text_body(
        &ShapeText {
            holder: text_box.unwrap_or(element),
            shape: element,
            style: &props,
            custom_shape,
        },
        &TextContext::on(*ctx, &base.id),
    )?;
    let columns = props.columns.unwrap_or(1.0);
    if text.is_some() && columns > 1.0 {
        ctx.see(FeatureId::TextColumns);
        ctx.report(
            codes::OD9208,
            DiagnosticSite::new()
                .node(base.id.clone())
                .feature(FeatureId::TextColumns)
                .data("columns", columns),
        );
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
        ext: None,
        frame,
        geometry: Some(geometry),
        fill: fill.fill,
        stroke: line.stroke,
        text,
        shadow,
    })))
}

/// A line or connector: its box from its two ends, which also set its flips.
fn lower_line<'a>(
    element: &'a Element,
    base: Base,
    name: &str,
    ctx: &Lowering<'a, '_>,
) -> Result<IrElement, LowerError> {
    let (frame, geometry) = line_geometry(element, name);
    if frame.flip_h || frame.flip_v {
        ctx.see(FeatureId::FrameFlip);
    }
    lower_shape(element, base, geometry, Some(frame), ctx, None)
}

/// Whether a reference names a vector stand-in (`.svm`, `.wmf`, `.emf`, `.svg`).
fn is_vector_name(href: &str) -> bool {
    let lower = href.to_ascii_lowercase();
    [".svm", ".wmf", ".emf", ".svg"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

/// The first picture of a frame the output shows: a raster, or an SVG that passes the allow-list (LibreOffice writes
/// an SVG picture first and its PNG preview after it); else the first picture, a vector stand-in it declines. Each
/// SVG the allow-list refuses before the picture shown is reported on `node` (OD9306).
fn frame_image<'a>(
    frame: &'a Element,
    ctx: &Lowering<'a, '_>,
    node: &NodeId,
) -> Option<&'a Element> {
    let mut refused = Vec::new();
    let shown = frame.children_of(Ns::Draw, "image").find(|image| {
        let href = image.attr(qn!("xlink:href")).unwrap_or("");
        if !is_vector_name(href) {
            return true;
        }
        let Reference::Internal(path) = resolve_reference(href) else {
            return false;
        };
        match check_svg_part(ctx.pkg, &path) {
            SvgCheck::Shown => true,
            SvgCheck::Refused(reason) => {
                refused.push((path, reason));
                false
            }
            SvgCheck::Other => false,
        }
    });
    let Some(shown) = shown else {
        return frame.children_of(Ns::Draw, "image").next();
    };
    for (path, reason) in refused {
        ctx.report(
            codes::OD9306,
            DiagnosticSite::new()
                .node(node.clone())
                .data("part", slidock_canon::utf16::clip(&path, 256).to_owned())
                .data("reason", reason),
        );
    }
    Some(shown)
}

/// The replacement image of an embedded object, as fallback media; `None`, quietly, when unreadable.
fn replacement<'a>(
    frame: &'a Element,
    base: &Base,
    ctx: &Lowering<'a, '_>,
) -> Result<Option<Sha256>, LowerError> {
    match frame_image(frame, ctx, &base.id)
        .and_then(|image| image.attr(qn!("xlink:href")))
        .filter(|href| !href.is_empty())
    {
        Some(href) => picture_media(href, ctx, FeatureId::ElementOther, Some(&base.id), true),
        None => Ok(None),
    }
}

/// The kind of document an embedded object holds, from the package manifest.
fn object_type<'a>(object: &'a Element, ctx: &Lowering<'a, '_>) -> ForeignType {
    if object.is(Ns::Draw, "object-ole") {
        return ForeignType::Ole;
    }
    let declared = match resolve_reference(object.attr(qn!("xlink:href")).unwrap_or("")) {
        Reference::Internal(path) => ctx.pkg.declared_type(&path),
        Reference::External(_) => None,
    };
    match declared {
        Some(kind) if kind.ends_with("opendocument.chart") => ForeignType::Chart,
        Some(kind) if kind.ends_with("opendocument.formula") => ForeignType::Math,
        _ => ForeignType::Ole,
    }
}

fn picture_of(base: Base, frame: Option<Frame>, media: Sha256) -> IrElement {
    IrElement::Picture(Box::new(Picture {
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
        media,
        fallback: None,
        crop: None,
        clip: None,
        mask: None,
        stroke: None,
        shadow: None,
    }))
}

/// A frame's movie or sound (`draw:plugin`): media with its replacement image as poster.
fn lower_plugin<'a>(
    frame: &'a Element,
    plugin: &'a Element,
    base: Base,
    frame_box: Option<Frame>,
    ctx: &Lowering<'a, '_>,
) -> Result<Option<IrElement>, LowerError> {
    let poster = match frame_image(frame, ctx, &base.id)
        .and_then(|image| image.attr(qn!("xlink:href")))
        .filter(|href| !href.is_empty())
    {
        Some(href) => picture_media(href, ctx, FeatureId::ElementVideo, Some(&base.id), true)?,
        None => None,
    };
    let written = plugin.attr(qn!("xlink:href"));
    let reference = resolve_reference(written.unwrap_or(""));
    let media = match &reference {
        Reference::Internal(path) => {
            picture_media(path, ctx, FeatureId::ElementVideo, Some(&base.id), true)?
        }
        Reference::External(_) => None,
    };
    let media_type = media
        .and_then(|media| {
            ctx.media
                .borrow()
                .entry(&media)
                .map(|entry| entry.mime.as_str().to_owned())
        })
        .or_else(|| plugin.attr(qn!("draw:mime-type")).map(str::to_owned))
        .unwrap_or_default();
    if media_type.starts_with("audio/") {
        ctx.see(FeatureId::ElementAudio);
        ctx.report(codes::OD9105, DiagnosticSite::new().node(base.id.clone()));
        let Some(poster) = poster else {
            return Ok(None);
        };
        ctx.see(FeatureId::ElementPicture);
        return Ok(Some(picture_of(base, frame_box, poster)));
    }
    ctx.see(FeatureId::ElementVideo);
    if media.is_none() {
        let target = match &reference {
            Reference::External(target) => target.clone(),
            Reference::Internal(_) => written.unwrap_or("(none)").to_owned(),
        };
        ctx.report(
            codes::OD9101,
            DiagnosticSite::new().node(base.id.clone()).data(
                "target",
                slidock_canon::utf16::clip(&target, 256).to_owned(),
            ),
        );
    }
    let param = |name: &str| {
        plugin
            .children_of(Ns::Draw, "param")
            .find(|candidate| candidate.attr(qn!("draw:name")) == Some(name))
            .and_then(|param| param.attr(qn!("draw:value")))
            == Some("true")
    };
    Ok(Some(IrElement::Media(Box::new(MediaElement {
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
        frame: frame_box,
        media_kind: MediaKind::Video,
        media,
        poster,
        trim: None,
        looped: param("Loop"),
        muted: param("Mute"),
        autoplay: false,
        across_slides: false,
        shadow: None,
    }))))
}

/// A frame by what it holds: an embedded object, a table (its replacement image unread), media, a text box or a
/// picture.
fn lower_frame<'a>(
    element: &'a Element,
    base: Base,
    ctx: &Lowering<'a, '_>,
) -> Result<Option<IrElement>, LowerError> {
    let object = element
        .child(Ns::Draw, "object")
        .or_else(|| element.child(Ns::Draw, "object-ole"));
    let undrawn = |what: &str| {
        report_undrawn_shadow(
            &style_of(element, ctx).graphic,
            what,
            &Paint::of(*ctx, Some(&base.id)),
        );
    };
    if let Some(object) = object {
        undrawn("object");
        let kind = object_type(object, ctx);
        let fallback = replacement(element, &base, ctx)?;
        let frame = frame_of(element, &base, ctx, Flips::default());
        let payload = payload(&[("element", &object.name)]);
        return Ok(Some(foreign(base, frame, kind, payload, ctx, fallback)));
    }
    if let Some(table) = element.child(Ns::Table, "table") {
        undrawn("table");
        let frame = frame_of(element, &base, ctx, Flips::default());
        return Ok(Some(
            match lower_table(table, &base, frame.as_ref(), ctx)? {
                Ok(table) => IrElement::Table(Box::new(table)),
                Err(reason) => {
                    let payload = payload(&[("element", "table:table"), ("reason", &reason)]);
                    let fallback = replacement(element, &base, ctx)?;
                    foreign(base, frame, ForeignType::Table, payload, ctx, fallback)
                }
            },
        ));
    }
    if let Some(plugin) = element.child(Ns::Draw, "plugin") {
        // LibreOffice draws no shadow on media.
        undrawn("media");
        let frame = frame_of(element, &base, ctx, Flips::default());
        return lower_plugin(element, plugin, base, frame, ctx);
    }
    let applet = element
        .child(Ns::Draw, "applet")
        .or_else(|| element.child(Ns::Draw, "floating-frame"));
    if let Some(applet) = applet {
        undrawn("object");
        let frame = frame_of(element, &base, ctx, Flips::default());
        let payload = payload(&[("element", &applet.name)]);
        return Ok(Some(foreign(
            base,
            frame,
            ForeignType::Unknown,
            payload,
            ctx,
            None,
        )));
    }
    let text_box = element.child(Ns::Draw, "text-box");
    if text_box.is_none()
        && let Some(image) = frame_image(element, ctx, &base.id)
    {
        return lower_picture(element, image, base, &style_of(element, ctx), ctx);
    }
    let frame = frame_of(element, &base, ctx, Flips::default());
    let geometry = rect_geometry(element, frame.as_ref());
    lower_shape(element, base, geometry, frame, ctx, text_box).map(Some)
}

/// A group: its children placed on the page, its box their union, their frames then moved into it.
fn lower_group<'a>(
    element: &'a Element,
    base: Base,
    scope: &Scope,
    ctx: &Lowering<'a, '_>,
) -> Result<IrElement, LowerError> {
    let limit = ctx.limits.group_depth;
    if scope.depth + 1 > limit {
        return Err(LowerError::Limit(format!(
            "groups nest deeper than {limit}"
        )));
    }
    ctx.see(FeatureId::ElementGroup);
    // LibreOffice draws no shadow on a group, whatever its style says.
    report_undrawn_shadow(
        &style_of(element, ctx).graphic,
        "group",
        &Paint::of(*ctx, Some(&base.id)),
    );
    let inner = Scope {
        depth: scope.depth + 1,
        ..scope.clone()
    };
    let kids = lower_children(element, &inner, ctx)?;
    let boxes: Vec<_> = kids
        .iter()
        .filter_map(|kid| kid.frame().as_ref().map(bounds))
        .collect();
    let group = |frame: Frame, children: Vec<IrElement>| {
        IrElement::Group(Box::new(Group {
            id: base.id.clone(),
            origin: base.origin.clone(),
            name: base.name.clone(),
            placeholder: base.placeholder.clone(),
            inherits: None,
            match_key: None,
            hidden: base.hidden,
            opacity: None,
            link: base.link.clone(),
            alt: base.alt.clone(),
            ext: None,
            frame: Some(frame),
            child_space: None,
            children,
            shadow: None,
        }))
    };
    let Some(first) = boxes.first() else {
        return Ok(group(no_frame(), kids));
    };
    let (mut x0, mut y0, mut x1, mut y1) = (first.x0, first.y0, first.x1, first.y1);
    for b in &boxes {
        x0 = x0.min(b.x0);
        y0 = y0.min(b.y0);
        x1 = x1.max(b.x1);
        y1 = y1.max(b.y1);
    }
    let frame = Frame {
        x: pt(to_pt(x0)),
        y: pt(to_pt(y0)),
        w: size(to_pt(x1 - x0)),
        h: size(to_pt(y1 - y0)),
        rotation: Angle::ZERO,
        flip_h: false,
        flip_v: false,
    };
    let (fx, fy) = (frame.x.to_f64(), frame.y.to_f64());
    let moved = kids
        .into_iter()
        .map(|mut kid| {
            if let Some(kid_frame) = kid.frame_mut() {
                kid_frame.x = pt(to_pt(kid_frame.x.to_f64() - fx));
                kid_frame.y = pt(to_pt(kid_frame.y.to_f64() - fy));
            }
            kid
        })
        .collect();
    Ok(group(frame, moved))
}

fn lower_drawable<'a>(
    element: &'a Element,
    path: &str,
    scope: &Scope,
    ctx: &Lowering<'a, '_>,
    id: Option<NodeId>,
) -> Result<Option<IrElement>, LowerError> {
    let base = element_base(element, path, ctx, scope.link.as_ref(), id)?;
    // Groups recurse through this frame alone; every other drawable lowers in a frame of its own, off the
    // recursion's stack.
    if element.is(Ns::Draw, "g") {
        return lower_group(element, base, scope, ctx).map(Some);
    }
    lower_leaf(element, base, ctx)
}

/// A drawable that is not a group.
#[inline(never)]
fn lower_leaf<'a>(
    element: &'a Element,
    base: Base,
    ctx: &Lowering<'a, '_>,
) -> Result<Option<IrElement>, LowerError> {
    if element.in_namespace(Ns::Dr3d) {
        let frame = frame_of(element, &base, ctx, Flips::default());
        let payload = payload(&[("element", &element.name)]);
        return Ok(Some(foreign(
            base,
            frame,
            ForeignType::ThreeD,
            payload,
            ctx,
            None,
        )));
    }
    let lowered = match element.local.as_str() {
        "custom-shape" => {
            let mut budget = Budget {
                left: i64::try_from(ctx.limits.path_segments).unwrap_or(i64::MAX),
            };
            let read = custom_shape_geometry(element, &mut budget)?;
            let flips = match &read {
                ReadGeometry::Geometry { flip_h, flip_v, .. } => Flips {
                    flip_h: *flip_h,
                    flip_v: *flip_v,
                },
                ReadGeometry::Unknown(_) | ReadGeometry::Unreadable { .. } => Flips::default(),
            };
            let frame = frame_of(element, &base, ctx, flips);
            lower_shape(element, base, read, frame, ctx, None)?
        }
        "rect" => {
            let frame = frame_of(element, &base, ctx, Flips::default());
            let geometry = rect_geometry(element, frame.as_ref());
            lower_shape(element, base, geometry, frame, ctx, None)?
        }
        "circle" | "ellipse" => {
            let geometry = ellipse_geometry(element);
            let frame = frame_of(element, &base, ctx, Flips::default());
            lower_shape(element, base, geometry, frame, ctx, None)?
        }
        "polyline" | "polygon" | "path" => {
            let mut budget = Budget {
                left: i64::try_from(ctx.limits.path_segments).unwrap_or(i64::MAX),
            };
            let geometry = path_geometry(element, &mut budget)?;
            let frame = frame_of(element, &base, ctx, Flips::default());
            lower_shape(element, base, geometry, frame, ctx, None)?
        }
        "line" => lower_line(element, base, "line", ctx)?,
        "connector" => {
            if element.attr(qn!("draw:type")).unwrap_or("standard") != "line" {
                ctx.report(
                    codes::OD9110,
                    DiagnosticSite::new()
                        .node(base.id.clone())
                        .feature(FeatureId::GeometryConnector),
                );
            }
            lower_line(element, base, "straightConnector1", ctx)?
        }
        "frame" => return lower_frame(element, base, ctx),
        _ => {
            let frame = frame_of(element, &base, ctx, Flips::default());
            let payload = payload(&[("element", &element.name)]);
            foreign(base, frame, ForeignType::Unknown, payload, ctx, None)
        }
    };
    Ok(Some(lowered))
}

fn lower_children<'a>(
    parent: &'a Element,
    scope: &Scope,
    ctx: &Lowering<'a, '_>,
) -> Result<Vec<IrElement>, LowerError> {
    let mut out = Vec::new();
    // A hyperlink (`draw:a`) opens in place: its drawables lower in order, under its link and its path. The links
    // are walked with a stack of their own, so nesting costs no call depth.
    let mut open = vec![(parent.positioned(&scope.path).into_iter(), scope.clone())];
    while let Some((items, at)) = open.last_mut() {
        let Some((element, path)) = items.next() else {
            open.pop();
            continue;
        };
        if element.is(Ns::Draw, "a") {
            let link =
                read_href(element.attr(qn!("xlink:href")), ctx.pages).or_else(|| at.link.clone());
            let inner = Scope {
                depth: at.depth,
                link,
                path,
            };
            open.push((element.positioned(&inner.path).into_iter(), inner));
            continue;
        }
        if !is_drawable(element) || is_empty_presentation_object(element, ctx) {
            continue;
        }
        let at = at.clone();
        if let Some(lowered) = lower_drawable(element, &path, &at, ctx, None)? {
            out.push(lowered);
        }
    }
    Ok(out)
}

/// A slide's empty presentation object (`presentation:placeholder="true"`), which LibreOffice shows only while
/// editing, as its prompt: it draws nothing, whatever its style (LibreOffice 25.2, PDF export).
fn is_empty_presentation_object(element: &Element, ctx: &Lowering<'_, '_>) -> bool {
    ctx.part == CONTENT && element.attr(qn!("presentation:placeholder")) == Some("true")
}

/// A drawable copied from a master page onto a page, under the id its copy takes there.
pub(crate) fn lower_copy<'a>(
    element: &'a Element,
    path: &str,
    id: NodeId,
    ctx: &Lowering<'a, '_>,
) -> Result<Option<IrElement>, LowerError> {
    let scope = Scope {
        path: path.to_owned(),
        depth: 0,
        link: None,
    };
    lower_drawable(element, path, &scope, ctx, Some(id))
}

/// A drawable with the key its id takes (its native id, else its index), in the order lowering counts them.
pub(crate) struct Keyed<'a> {
    pub element: &'a Element,
    pub path: String,
    pub key: String,
}

/// The drawables of a page or master page with the keys their ids take.
pub(crate) fn keyed_drawables<'a>(parent: &'a Element, path: &str) -> Vec<Keyed<'a>> {
    fn visit<'a>(node: &'a Element, at: &str, out: &mut Vec<Keyed<'a>>) {
        for (element, own) in node.positioned(at) {
            if element.is(Ns::Draw, "a") {
                visit(element, &own, out);
            } else if is_drawable(element) {
                let key =
                    native_id(element).map_or_else(|| (out.len() + 1).to_string(), str::to_owned);
                let group = element.is(Ns::Draw, "g");
                out.push(Keyed {
                    element,
                    path: own.clone(),
                    key,
                });
                if group {
                    visit(element, &own, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    visit(parent, path, &mut out);
    out
}

/// A page's or master page's drawables, failing the container past the element limit.
pub(crate) fn lower_drawables<'a>(
    parent: &'a Element,
    path: &str,
    ctx: &Lowering<'a, '_>,
) -> Result<Vec<IrElement>, LowerError> {
    let scope = Scope {
        path: path.to_owned(),
        depth: 0,
        link: None,
    };
    let elements = lower_children(parent, &scope, ctx)?;
    let limit = ctx.limits.elements_per_container;
    if walk_elements(&elements).count() > limit {
        return Err(LowerError::Limit(format!("more than {limit} elements")));
    }
    Ok(elements)
}
