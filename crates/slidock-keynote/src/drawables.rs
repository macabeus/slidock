//! Drawables as IR elements, in z-order (back to front). Each object is placed once per container however often it
//! is referenced. Text shapes, placeholders, shapes, connection lines, images, movies, groups and tables lower to
//! their IR kinds; charts, unreadable tables and outlines, and unknown drawables become Foreign nodes with their
//! frame, which the core declines. Frames are Keynote's: the top-left of the unrotated box, the angle
//! counter-clockwise (negated once here), the geometry's flips, and a group's children in the group's coordinates. A
//! table's frame spans its tracks.

use std::collections::{BTreeMap, BTreeSet};

use indexmap::IndexMap;
use slidock_core_api::limits::add_slots;
use slidock_core_api::lowering::{IdScope, LowerError};
use slidock_ir::Nullable;
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::elements::{ForeignType, MediaKind, Placeholder, PlaceholderRole, Trim};
use slidock_ir::features::FeatureId;
use slidock_ir::formats::FileFormat;
use slidock_ir::frame::Frame;
use slidock_ir::geometry::{Geometry, Path};
use slidock_ir::ids::{IdPart, NodeId, Sha256};
use slidock_ir::json_value::JsonValue;
use slidock_ir::origin::Origin;
use slidock_ir::paint::Fill;
use slidock_ir::shadow::SourceShadow;
use slidock_ir::source::{Element, Foreign, Group, MediaElement, Picture, Shape, Table};
use slidock_ir::table::grid_size;
use slidock_ir::text::Link;
use slidock_ir::units::{Pt, Size};

use crate::DOCUMENT_PATH;
use crate::context::{DataId, Lowering};
use crate::geometry::{SlideMap, flips};
use crate::graph::{ObjectGraph, Resolved, referenced};
use crate::iwa::{Archive, ObjectId};
use crate::num::{clamp, finite_or, js_max, ms, normalised_angle, pt, unit_ratio, wide};
use crate::paint::{Ends, FillBox, ShadowRead, fill, shadow, stroke};
use crate::paths::{callout_tail_unread, instant_alpha_geometry, mask_paths, shape_geometry};
use crate::proto::{kn, tsch, tsd, tsp, tst, tswp};
use crate::styles::{MediaStyle, ShapeStyle, inherited, style_chain};
use crate::table::{TableError, read_table};
use crate::text::{BoxPlace, FontUses, text_body};

/// What lowering one container's drawables keeps: the presentation's id scope, what it placed, and the fonts its
/// text names.
pub struct DrawableContext<'a> {
    pub low: Lowering<'a>,
    pub ids: &'a mut IdScope,
    /// The objects this container placed.
    pub placed: BTreeSet<ObjectId>,
    /// Every PostScript name the presentation's text asks for.
    pub requested: &'a mut BTreeSet<String>,
    /// The PostScript names the container's printed text draws.
    pub drawn: BTreeSet<String>,
    /// On a slide: the layout placeholder each placeholder kind binds to.
    pub targets: &'a BTreeMap<i32, NodeId>,
    /// Where the drawables being lowered (the container's, or a group's children) land on the slide.
    pub to_slide: SlideMap,
}

impl std::fmt::Debug for DrawableContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DrawableContext")
            .field("low", &self.low)
            .field("placed", &self.placed.len())
            .field("drawn", &self.drawn)
            .finish_non_exhaustive()
    }
}

/// `KN.PlaceholderArchive.Kind` → the IR's placeholder role.
fn role(kind: i32) -> PlaceholderRole {
    match kind {
        1 => PlaceholderRole::SldNum,
        2 => PlaceholderRole::Title,
        3 => PlaceholderRole::Body,
        4 => PlaceholderRole::Obj,
        _ => PlaceholderRole::Other,
    }
}

/// The template slide field naming each kind's placeholder.
pub fn placeholder_fields(template: &kn::SlideArchive) -> [(i32, Option<&tsp::Reference>); 4] {
    [
        (1, template.slide_number_placeholder.as_ref()),
        (2, template.title_placeholder.as_ref()),
        (3, template.body_placeholder.as_ref()),
        (4, template.object_placeholder.as_ref()),
    ]
}

fn fill_feature(fill: &Fill) -> Option<FeatureId> {
    match fill.kind() {
        "solid" => Some(FeatureId::FillSolid),
        "linear" => Some(FeatureId::FillLinear),
        "radial" => Some(FeatureId::FillRadial),
        "image" => Some(FeatureId::FillImage),
        _ => None,
    }
}

/// `MovieArchive.MovieLoopOption`: repeat, and back and forth.
const LOOPING: [i32; 2] = [1, 2];
const MAX_ALT: usize = 2048;

/// A coordinate the file holds, finite and inside the IR's range.
fn coordinate(x: Option<f32>) -> Pt {
    pt(clamp(finite_or(wide(x), 0.0), -100_000.0, 100_000.0))
}

fn length(x: Option<f32>) -> Size {
    crate::num::size(js_max(0.0, coordinate(x).to_f64()))
}

pub fn frame_of(geometry: Option<&tsd::GeometryArchive>) -> Frame {
    let position = geometry.and_then(|g| g.position.as_ref());
    let size = geometry.and_then(|g| g.size.as_ref());
    let (flip_h, flip_v) = flips(geometry);
    Frame {
        x: coordinate(position.and_then(|p| p.x)),
        y: coordinate(position.and_then(|p| p.y)),
        w: length(size.and_then(|s| s.width)),
        h: length(size.and_then(|s| s.height)),
        rotation: normalised_angle(-finite_or(geometry.and_then(|g| wide(g.angle)), 0.0)),
        flip_h,
        flip_v,
    }
}

/// What every element of a drawable shares.
#[derive(Clone, Debug)]
struct Base {
    id: NodeId,
    origin: Origin,
    frame: Frame,
    alt: Option<String>,
    link: Option<Link>,
}

fn object_origin(file: &str, object: ObjectId) -> Origin {
    Origin::Source {
        part: file.to_owned(),
        path: None,
        object: Some(object.to_string()),
    }
}

/// `/^(https?|mailto):/i`
fn is_linkable(url: &str) -> bool {
    let lower = |n: usize| url.get(..n).map(str::to_ascii_lowercase);
    lower(5).as_deref() == Some("http:")
        || lower(6).as_deref() == Some("https:")
        || lower(7).as_deref() == Some("mailto:")
}

fn base(
    ctx: &mut DrawableContext<'_>,
    object: &Resolved<'_>,
    id: &NodeId,
    drawable: Option<&tsd::DrawableArchive>,
) -> Base {
    let geometry = drawable.and_then(|d| d.geometry.as_ref());
    let frame = frame_of(geometry);
    if frame.rotation.steps() != 0 {
        ctx.low.see(FeatureId::FrameRotation);
    }
    if frame.flip_h || frame.flip_v {
        ctx.low.see(FeatureId::FrameFlip);
    }
    let mut link = None;
    match drawable.and_then(|d| d.hyperlink_url.as_deref()) {
        Some(url) if is_linkable(url) => {
            link = Some(Link::Url {
                url: slidock_canon::utf16::clip(url, 2048).to_owned(),
            });
            ctx.low.see(FeatureId::ElementLink);
        }
        Some(url) if !url.is_empty() => ctx.low.report(
            codes::KN1108,
            DiagnosticSite::new()
                .node(id.clone())
                .data("url", slidock_canon::utf16::clip(url, 256)),
        ),
        _ => {}
    }
    let alt = drawable
        .and_then(|d| d.accessibility_description.as_deref())
        .map(|alt| slidock_canon::utf16::clip(alt, MAX_ALT).to_owned())
        .filter(|alt| !alt.is_empty());
    if alt.is_some() {
        ctx.low.see(FeatureId::ElementAlt);
    }
    let caption = drawable
        .and_then(|d| d.caption.as_ref())
        .and_then(|caption| ctx.low.graph.type_name(Some(caption)));
    if caption.is_some_and(|name| name != "TSD.StandinCaptionArchive")
        && drawable.and_then(|d| d.caption_hidden) != Some(true)
    {
        ctx.low.report(
            codes::KN1109,
            DiagnosticSite::new()
                .node(id.clone())
                .data("what", "caption"),
        );
    }
    Base {
        id: id.clone(),
        origin: object_origin(object.file, object.id),
        frame,
        alt,
        link,
    }
}

fn payload(fields: &[(&str, JsonValue)]) -> JsonValue {
    JsonValue::Object(
        fields
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect::<IndexMap<_, _>>(),
    )
}

/// A drawable kept in place as a Foreign with its frame, for the core to decline.
fn foreign(
    b: Base,
    kind: ForeignType,
    payload: JsonValue,
    ctx: &mut DrawableContext<'_>,
) -> Element {
    ctx.low.see(match kind {
        ForeignType::Table => FeatureId::ElementTable,
        ForeignType::Chart => FeatureId::ElementChart,
        _ => FeatureId::ElementOther,
    });
    Element::Foreign(Box::new(Foreign {
        id: b.id,
        origin: b.origin,
        name: None,
        placeholder: None,
        inherits: None,
        match_key: None,
        hidden: None,
        opacity: None,
        link: b.link,
        alt: b.alt,
        ext: None,
        frame: Some(b.frame),
        dialect: FileFormat::Keynote,
        foreign_type: kind,
        payload: Some(payload),
        fallback: None,
    }))
}

/// An opacity below 1, as the IR writes it.
fn translucent(opacity: Option<f64>) -> Option<slidock_ir::units::UnitRatio> {
    opacity
        .filter(|opacity| *opacity < 1.0)
        .map(|opacity| unit_ratio(js_max(0.0, opacity)))
}

/// An element's shadow: drawn, or absent when none is drawn (a shadow the IR does not draw is reported, KN1304).
fn element_shadow(
    ctx: &mut DrawableContext<'_>,
    archive: Option<&tsd::ShadowArchive>,
    id: &NodeId,
) -> Nullable<SourceShadow> {
    match shadow(archive) {
        ShadowRead::Off => None,
        ShadowRead::On(shadow) => {
            ctx.low.see(FeatureId::EffectShadow);
            Some(Some(shadow))
        }
        ShadowRead::NotDrawn(reason) => {
            ctx.low.see(FeatureId::EffectShadow);
            ctx.low.report(
                codes::KN1304,
                DiagnosticSite::new()
                    .node(id.clone())
                    .data("reason", reason),
            );
            None
        }
    }
}

/// A shape's text: its storage, its geometry flags, and whether it prints.
#[derive(Clone, Copy, Debug)]
struct ShapeText<'g> {
    storage: Option<&'g tsp::Reference>,
    flags: Option<u32>,
    printed: bool,
}

fn shape_of(
    ctx: &mut DrawableContext<'_>,
    object: &Resolved<'_>,
    id: &NodeId,
    message: &tsd::ShapeArchive,
    text: Option<ShapeText<'_>>,
) -> Result<Element, LowerError> {
    let b = base(ctx, object, id, message.super_.as_ref());
    let source = message.pathsource.as_ref();
    let (w, h) = (b.frame.w.to_f64(), b.frame.h.to_f64());
    let Some(geometry) = shape_geometry(source, w, h, ctx.low.limits.path_segments)? else {
        let data = payload(&[
            ("type", object.archive.type_name().into()),
            ("reason", "outline".into()),
        ]);
        return Ok(foreign(b, ForeignType::Unknown, data, ctx));
    };
    if let Some(reason) = callout_tail_unread(source, w, h) {
        ctx.low.report(
            codes::KN1104,
            DiagnosticSite::new()
                .node(id.clone())
                .data("reason", reason),
        );
    }
    let graph = ctx.low.graph;
    let props = style_chain::<ShapeStyle>(graph, message.style.as_ref());
    let shape_fill = fill(
        inherited(&props, |p| (p.fill.as_ref(), None)),
        FillBox::Frame { w, h },
        &mut ctx.low,
        Some(id),
    )
    .unwrap_or(Fill::None);
    let ends = Ends {
        head: inherited(&props, |p| (p.head_line_end.as_ref(), None))
            .or(message.head_line_end.as_ref()),
        tail: inherited(&props, |p| (p.tail_line_end.as_ref(), None))
            .or(message.tail_line_end.as_ref()),
    };
    let shape_stroke = stroke(
        inherited(&props, |p| (p.stroke.as_ref(), None)),
        ends,
        &mut ctx.low,
    )
    .flatten();
    let opacity = inherited(&props, |p| (p.opacity.as_ref(), None)).map(|x| f64::from(*x));
    let shape_shadow = element_shadow(ctx, inherited(&props, |p| (p.shadow.as_ref(), None)), id);
    ctx.low.see(FeatureId::ElementShape);
    if let Some(feature) = fill_feature(&shape_fill) {
        ctx.low.see(feature);
    }
    ctx.low.see(match geometry {
        Geometry::Custom(_) => FeatureId::GeometryCustom,
        _ => FeatureId::GeometryPreset,
    });
    let opacity = translucent(opacity);
    if opacity.is_some() {
        ctx.low.see(FeatureId::ElementOpacity);
    }
    let body = match text {
        Some(text) => {
            let mut fonts = FontUses {
                requested: &mut *ctx.requested,
                drawn: text.printed.then_some(&mut ctx.drawn),
            };
            text_body(
                &mut ctx.low,
                text.storage,
                message.style.as_ref(),
                text.flags,
                BoxPlace {
                    x: b.frame.x.to_f64(),
                    y: b.frame.y.to_f64(),
                    w,
                    h,
                    to_slide: ctx.to_slide,
                },
                id,
                &mut fonts,
            )?
        }
        None => None,
    };
    Ok(Element::Shape(Box::new(Shape {
        id: b.id,
        origin: b.origin,
        name: None,
        placeholder: None,
        inherits: None,
        match_key: None,
        hidden: None,
        opacity,
        link: b.link,
        alt: b.alt,
        ext: None,
        frame: Some(b.frame),
        geometry: Some(geometry),
        fill: Some(shape_fill),
        stroke: Some(shape_stroke),
        text: body,
        shadow: shape_shadow,
    })))
}

fn text_shape_of(
    ctx: &mut DrawableContext<'_>,
    object: &Resolved<'_>,
    id: &NodeId,
    message: &tswp::ShapeInfoArchive,
    printed: bool,
) -> Result<Element, LowerError> {
    let default = tsd::ShapeArchive::default();
    let shape = message.super_.as_ref().unwrap_or(&default);
    let flags = shape
        .super_
        .as_ref()
        .and_then(|d| d.geometry.as_ref())
        .and_then(|g| g.flags);
    shape_of(
        ctx,
        object,
        id,
        shape,
        Some(ShapeText {
            storage: message.owned_storage.as_ref(),
            flags,
            printed,
        }),
    )
}

fn set_placeholder(element: &mut Element, placeholder: Placeholder) {
    let slot = match element {
        Element::Shape(e) => &mut e.placeholder,
        Element::Picture(e) => &mut e.placeholder,
        Element::Media(e) => &mut e.placeholder,
        Element::Table(e) => &mut e.placeholder,
        Element::Group(e) => &mut e.placeholder,
        Element::Foreign(e) => &mut e.placeholder,
    };
    *slot = Some(placeholder);
}

fn placeholder_of(
    ctx: &mut DrawableContext<'_>,
    object: &Resolved<'_>,
    id: &NodeId,
    message: &kn::PlaceholderArchive,
) -> Result<Element, LowerError> {
    let kind = message.kind.unwrap_or(0);
    let slide = ctx.low.layer == crate::context::Layer::Slide;
    let default = tswp::ShapeInfoArchive::default();
    // A layout's placeholders are unprinted prototypes.
    let mut element = text_shape_of(
        ctx,
        object,
        id,
        message.super_.as_ref().unwrap_or(&default),
        slide,
    )?;
    ctx.low.see(FeatureId::Placeholder);
    let target = if slide {
        ctx.targets.get(&kind).cloned()
    } else {
        None
    };
    set_placeholder(
        &mut element,
        Placeholder {
            role: role(kind),
            index: None,
            target,
        },
    );
    // Its style chain is complete: a placeholder without a drawn shadow states none, so it inherits none from the
    // placeholder it binds to.
    if let Some(shadow @ None) = element.shadow_mut() {
        *shadow = Some(None);
    }
    Ok(element)
}

/// A picture's media and the raster that stands in for it where no host decodes it.
struct PictureMedia {
    media: Sha256,
    fallback: Option<Sha256>,
}

/// An image's best picture: the adjusted one, the original, a vector's SVG original, or its raster thumbnail. A vector
/// image's `data` is a PDF, which the output cannot show; Keynote keeps the SVG it was made from beside it, and its
/// thumbnail is a small preview (256 pixels wide), so the SVG comes first when it passes the allow-list. A thumbnail
/// drawn in place of an SVG original the allow-list refuses is reported (KN1113). An image only a host may decode (a
/// HEIC photo) is the picture's media all the same, and the next one the output shows (the thumbnail of an inserted
/// photo) is its fallback: plan-media draws that where no host decoded it (MID5609).
fn picture_media(
    ctx: &mut DrawableContext<'_>,
    id: &NodeId,
    message: &tsd::ImageArchive,
) -> Option<PictureMedia> {
    let original = DataId::of(message.original_svg_data.as_ref());
    let candidates = [
        (&message.adjusted_image_data, false),
        (&message.data, false),
        (&message.original_svg_data, false),
        (&message.thumbnail_data, true),
    ];
    let mut decoded = None;
    for (candidate, preview) in candidates {
        let data = DataId::of(candidate.as_ref());
        if decoded.is_none() && !preview && ctx.low.media.host_decodes(data) {
            decoded = Some(data);
            continue;
        }
        if ctx.low.media.is_image(data) {
            if preview && let Some(refusal) = ctx.low.media.svg_refusal(original) {
                let original = original.map_or_else(|| "none".to_owned(), |data| data.to_string());
                ctx.low.report(
                    codes::KN1113,
                    DiagnosticSite::new()
                        .node(id.clone())
                        .data("data", original)
                        .data("reason", refusal.to_string()),
                );
            }
            let shown = ctx.low.media.media(data);
            return match decoded.and_then(|data| ctx.low.media.media(data)) {
                Some(media) => Some(PictureMedia {
                    media,
                    fallback: shown,
                }),
                None => shown.map(PictureMedia::alone),
            };
        }
    }
    let data = decoded.unwrap_or_else(|| DataId::of(message.data.as_ref()));
    ctx.low.media.media(data).map(PictureMedia::alone)
}

impl PictureMedia {
    fn alone(media: Sha256) -> Self {
        PictureMedia {
            media,
            fallback: None,
        }
    }
}

fn picture_of(
    ctx: &mut DrawableContext<'_>,
    object: &Resolved<'_>,
    id: &NodeId,
    message: &tsd::ImageArchive,
) -> Option<Element> {
    let b = base(ctx, object, id, message.super_.as_ref());
    let mask = ctx.low.graph.get::<tsd::MaskArchive>(message.mask.as_ref());
    let paths = mask.map(|mask| {
        mask_paths(
            mask.pathsource.as_ref(),
            mask.super_.as_ref().and_then(|d| d.geometry.as_ref()),
        )
    });
    let mask_paths = match paths {
        Some(None) => {
            let data = payload(&[
                ("type", "TSD.MaskArchive".into()),
                ("reason", "outline".into()),
            ]);
            return Some(foreign(b, ForeignType::Unknown, data, ctx));
        }
        Some(Some(paths)) => Some(paths),
        None => None,
    };
    let Some(PictureMedia { media, fallback }) = picture_media(ctx, id, message) else {
        let data = DataId::of(message.data.as_ref())
            .map_or_else(|| "none".to_owned(), |data| data.to_string());
        ctx.low.report(
            codes::KN1110,
            DiagnosticSite::new().node(id.clone()).data("data", data),
        );
        return None;
    };
    let clip = instant_alpha(ctx, id, message);
    Some(picture(
        ctx,
        b,
        PictureMedia { media, fallback },
        message.style.as_ref(),
        Outlines {
            clip,
            mask: mask_paths,
        },
    ))
}

/// The outline an image's Instant Alpha cuts it to, as the picture's shape. An outline that is not read leaves the
/// image whole, and is reported (KN1114, with the reason).
fn instant_alpha(
    ctx: &mut DrawableContext<'_>,
    id: &NodeId,
    message: &tsd::ImageArchive,
) -> Option<Geometry> {
    let path = message.instant_alpha_path.as_ref()?;
    match instant_alpha_geometry(
        path,
        message.natural_size.as_ref(),
        ctx.low.limits.path_segments,
    ) {
        Ok(geometry) => Some(geometry),
        Err(reason) => {
            ctx.low.report(
                codes::KN1114,
                DiagnosticSite::new()
                    .node(id.clone())
                    .data("reason", reason),
            );
            None
        }
    }
}

/// What a picture is cut to: its shape (an image's Instant Alpha outline) and its mask, both in its frame.
#[derive(Default)]
struct Outlines {
    clip: Option<Geometry>,
    mask: Option<Vec<Path>>,
}

fn picture(
    ctx: &mut DrawableContext<'_>,
    b: Base,
    PictureMedia { media, fallback }: PictureMedia,
    style: Option<&tsp::Reference>,
    Outlines { clip, mask }: Outlines,
) -> Element {
    let props = style_chain::<MediaStyle>(ctx.low.graph, style);
    let picture_stroke = stroke(
        inherited(&props, |p| (p.stroke.as_ref(), None)),
        Ends::default(),
        &mut ctx.low,
    )
    .flatten();
    let opacity = inherited(&props, |p| (p.opacity.as_ref(), None)).map(|x| f64::from(*x));
    let picture_shadow =
        element_shadow(ctx, inherited(&props, |p| (p.shadow.as_ref(), None)), &b.id);
    ctx.low.see(FeatureId::ElementPicture);
    let mask = mask.filter(|mask| !mask.is_empty());
    if mask.is_some() {
        ctx.low.see(FeatureId::ImageMask);
    }
    if clip.is_some() {
        ctx.low.see(FeatureId::ImageClip);
    }
    let opacity = translucent(opacity);
    if opacity.is_some() {
        ctx.low.see(FeatureId::ImageOpacity);
    }
    Element::Picture(Box::new(Picture {
        id: b.id,
        origin: b.origin,
        name: None,
        placeholder: None,
        inherits: None,
        match_key: None,
        hidden: None,
        opacity,
        link: b.link,
        alt: b.alt,
        ext: None,
        frame: Some(b.frame),
        media,
        fallback,
        crop: None,
        clip,
        mask,
        stroke: picture_stroke.map(Some),
        shadow: picture_shadow,
    }))
}

fn movie_of(
    ctx: &mut DrawableContext<'_>,
    object: &Resolved<'_>,
    id: &NodeId,
    message: &tsd::MovieArchive,
) -> Option<Element> {
    let b = base(ctx, object, id, message.super_.as_ref());
    let movie_data = DataId::of(message.movie_data.as_ref());
    // Keynote places an animated GIF as a movie.
    if ctx.low.media.is_image(movie_data)
        && let Some(media) = ctx.low.media.media(movie_data)
    {
        return Some(picture(
            ctx,
            b,
            PictureMedia::alone(media),
            message.style.as_ref(),
            Outlines::default(),
        ));
    }
    if message.audio_only == Some(true) {
        ctx.low.see(FeatureId::ElementAudio);
        ctx.low
            .report(codes::KN1107, DiagnosticSite::new().node(id.clone()));
        return None;
    }
    let media = ctx.low.media.media(movie_data);
    let poster = ctx
        .low
        .media
        .media(DataId::of(message.poster_image_data.as_ref()));
    if media.is_none() {
        let data = movie_data.map_or_else(|| "none".to_owned(), |data| data.to_string());
        ctx.low.report(
            codes::KN1105,
            DiagnosticSite::new().node(id.clone()).data("data", data),
        );
    }
    let props = style_chain::<MediaStyle>(ctx.low.graph, message.style.as_ref());
    let opacity = inherited(&props, |p| (p.opacity.as_ref(), None)).map(|x| f64::from(*x));
    let movie_shadow = element_shadow(ctx, inherited(&props, |p| (p.shadow.as_ref(), None)), id);
    ctx.low.see(FeatureId::ElementVideo);
    let start_ms = ms(wide(message.start_time));
    let end_ms = wide(message.end_time)
        .filter(|end| *end != 0.0 && !end.is_nan())
        .map(|end| ms(Some(end)));
    let trim = (start_ms.0 > 0 || end_ms.is_some()).then_some(Trim { start_ms, end_ms });
    Some(Element::Media(Box::new(MediaElement {
        id: b.id,
        origin: b.origin,
        name: None,
        placeholder: None,
        inherits: None,
        match_key: None,
        hidden: None,
        opacity: translucent(opacity),
        link: b.link,
        alt: b.alt,
        ext: None,
        frame: Some(b.frame),
        media_kind: MediaKind::Video,
        media,
        poster,
        trim,
        looped: LOOPING.contains(&message.loop_option.unwrap_or(0)),
        muted: message.volume == Some(0.0),
        autoplay: message.auto_play == Some(true),
        across_slides: message.plays_across_slides == Some(true),
        shadow: movie_shadow,
    })))
}

fn group_of(
    ctx: &mut DrawableContext<'_>,
    object: &Resolved<'_>,
    id: &NodeId,
    message: &tsd::GroupArchive,
    depth: usize,
) -> Result<Element, LowerError> {
    let limit = ctx.low.limits.group_depth;
    if depth + 1 > limit {
        return Err(LowerError::Limit(format!(
            "Groups nest deeper than {limit}"
        )));
    }
    let b = base(ctx, object, id, message.super_.as_ref());
    ctx.low.see(FeatureId::ElementGroup);
    let outer = ctx.to_slide;
    ctx.to_slide = outer.within(&b.frame);
    let children = lower_drawables(ctx, &message.children, depth + 1);
    ctx.to_slide = outer;
    let children = children?;
    Ok(Element::Group(Box::new(Group {
        id: b.id,
        origin: b.origin,
        name: None,
        placeholder: None,
        inherits: None,
        match_key: None,
        hidden: None,
        opacity: None,
        link: b.link,
        alt: b.alt,
        ext: None,
        frame: Some(b.frame),
        child_space: None,
        children,
        shadow: None,
    })))
}

fn table_of(
    ctx: &mut DrawableContext<'_>,
    object: &Resolved<'_>,
    id: &NodeId,
    message: &tst::TableInfoArchive,
) -> Result<Element, LowerError> {
    let b = base(ctx, object, id, message.super_.as_ref());
    let mut fonts = FontUses {
        requested: &mut *ctx.requested,
        drawn: Some(&mut ctx.drawn),
    };
    match read_table(&mut ctx.low, &mut fonts, message, id) {
        Ok(grid) => {
            let (w, h) = grid_size(&grid.columns, &grid.rows);
            let frame = Frame {
                w: Size::from_steps(w.steps()),
                h: Size::from_steps(h.steps()),
                ..b.frame
            };
            Ok(Element::Table(Box::new(Table {
                id: b.id,
                origin: b.origin,
                name: None,
                placeholder: None,
                inherits: None,
                match_key: None,
                hidden: None,
                opacity: None,
                link: b.link,
                alt: b.alt,
                ext: None,
                frame: Some(frame),
                columns: grid.columns,
                rows: grid.rows,
            })))
        }
        Err(TableError::Lower(error)) => Err(error),
        Err(TableError::Unreadable(reason)) => {
            let model = ctx
                .low
                .graph
                .get::<tst::TableModelArchive>(message.table_model.as_ref());
            let data = payload(&[
                (
                    "name",
                    model
                        .and_then(|m| m.table_name.clone())
                        .unwrap_or_default()
                        .into(),
                ),
                (
                    "rows",
                    JsonValue::number(f64::from(model.and_then(|m| m.number_of_rows).unwrap_or(0))),
                ),
                (
                    "columns",
                    JsonValue::number(f64::from(
                        model.and_then(|m| m.number_of_columns).unwrap_or(0),
                    )),
                ),
                ("reason", reason.into()),
            ]);
            Ok(foreign(b, ForeignType::Table, data, ctx))
        }
    }
}

/// The drawable a message's `super` is, for a type lowered as unknown.
fn drawable_of(archive: &Archive) -> Option<&tsd::DrawableArchive> {
    match archive {
        Archive::TsdMaskArchive(mask) => mask.super_.as_ref(),
        _ => None,
    }
}

/// One object as an element. Groups recurse; every other kind lowers in [`lower_leaf`], whose frame stays off the
/// recursion's stack.
fn lower_object(
    ctx: &mut DrawableContext<'_>,
    object: &Resolved<'_>,
    id: &NodeId,
    depth: usize,
) -> Result<Option<Element>, LowerError> {
    match object.archive {
        Archive::TsdGroupArchive(message) => group_of(ctx, object, id, message, depth).map(Some),
        _ => lower_leaf(ctx, object, id),
    }
}

#[inline(never)]
fn lower_leaf(
    ctx: &mut DrawableContext<'_>,
    object: &Resolved<'_>,
    id: &NodeId,
) -> Result<Option<Element>, LowerError> {
    Ok(match object.archive {
        Archive::KnPlaceholderArchive(message) => Some(placeholder_of(ctx, object, id, message)?),
        Archive::TswpShapeInfoArchive(message) => {
            Some(text_shape_of(ctx, object, id, message, true)?)
        }
        Archive::TsdShapeArchive(message) => Some(shape_of(ctx, object, id, message, None)?),
        Archive::TsdConnectionLineArchive(message) => {
            let default = tsd::ShapeArchive::default();
            let shape = message.super_.as_ref().unwrap_or(&default);
            Some(shape_of(ctx, object, id, shape, None)?)
        }
        Archive::TsdImageArchive(message) => picture_of(ctx, object, id, message),
        Archive::TsdMovieArchive(message) => movie_of(ctx, object, id, message),
        Archive::TstTableInfoArchive(message) => Some(table_of(ctx, object, id, message)?),
        Archive::TschChartDrawableArchive(message) => {
            let message: &tsch::ChartDrawableArchive = message;
            let b = base(ctx, object, id, message.super_.as_ref());
            Some(foreign(b, ForeignType::Chart, payload(&[]), ctx))
        }
        // Any other decoded message: placed, in its frame when it has one.
        other => {
            let b = base(ctx, object, id, drawable_of(other));
            let data = payload(&[("type", other.type_name().into())]);
            Some(foreign(b, ForeignType::Unknown, data, ctx))
        }
    })
}

/// An object of a type the schema does not decode: an unknown Foreign at the origin, its frame unread.
#[inline(never)]
fn undecoded(
    ctx: &mut DrawableContext<'_>,
    id: NodeId,
    object: ObjectId,
    type_name: Option<&str>,
) -> Element {
    let file = ctx.low.graph.file_of(object).unwrap_or(DOCUMENT_PATH);
    let b = Base {
        id,
        origin: object_origin(file, object),
        frame: frame_of(None),
        alt: None,
        link: None,
    };
    let data = payload(&[("type", type_name.unwrap_or("unknown").into())]);
    foreign(b, ForeignType::Unknown, data, ctx)
}

/// The grid slots of the tables a list of references names, through groups as deep as [`lower_drawables`] reaches,
/// each object once: counted from each model's row and column counts, whether the table lowers or not.
pub fn table_slots(graph: &ObjectGraph, refs: &[tsp::Reference], group_depth: usize) -> u64 {
    fn visit(
        graph: &ObjectGraph,
        list: &[tsp::Reference],
        depth: usize,
        group_depth: usize,
        counted: &mut BTreeSet<ObjectId>,
        total: &mut u64,
    ) {
        for reference in list {
            let Some(id) = referenced(Some(reference)) else {
                continue;
            };
            if !counted.insert(id) {
                continue;
            }
            match graph.resolve_id(id).map(|object| object.archive) {
                Some(Archive::TstTableInfoArchive(info)) => {
                    let model = graph.get::<tst::TableModelArchive>(info.table_model.as_ref());
                    let rows = u64::from(model.and_then(|m| m.number_of_rows).unwrap_or(0));
                    let columns = u64::from(model.and_then(|m| m.number_of_columns).unwrap_or(0));
                    *total = add_slots(*total, rows.saturating_mul(columns));
                }
                Some(Archive::TsdGroupArchive(group)) if depth < group_depth => {
                    visit(
                        graph,
                        &group.children,
                        depth + 1,
                        group_depth,
                        counted,
                        total,
                    );
                }
                _ => {}
            }
        }
    }
    let mut counted = BTreeSet::new();
    let mut total = 0;
    visit(graph, refs, 0, group_depth, &mut counted, &mut total);
    total
}

fn taken_id(ctx: &mut DrawableContext<'_>, object: ObjectId) -> Result<(NodeId, bool), LowerError> {
    let part = object.to_string();
    let taken = ctx
        .ids
        .take(&[IdPart::Str(&part)])
        .map_err(|error| LowerError::Other(error.to_string()))?;
    Ok((taken.id, taken.duplicate))
}

/// The drawables a list of references names, each placed once in this container. A dangling reference is reported
/// (KN1008); an undecoded object becomes an unknown Foreign at the origin, its frame unread.
pub fn lower_drawables(
    ctx: &mut DrawableContext<'_>,
    refs: &[tsp::Reference],
    depth: usize,
) -> Result<Vec<Element>, LowerError> {
    let mut out = Vec::new();
    // A list that names an object again is reported once per object, however often it repeats.
    let mut repeated: BTreeSet<ObjectId> = BTreeSet::new();
    let graph = ctx.low.graph;
    for reference in refs {
        let Some(object_id) = referenced(Some(reference)) else {
            continue;
        };
        if ctx.placed.contains(&object_id) {
            if repeated.insert(object_id) {
                ctx.low.report(
                    codes::KN1010,
                    DiagnosticSite::new().data("id", object_id.to_string()),
                );
            }
            continue;
        }
        let limit = ctx.low.limits.elements_per_container;
        if ctx.placed.len() >= limit {
            return Err(LowerError::Limit(format!(
                "A container holds more than {limit} elements"
            )));
        }
        let object = graph.resolve(Some(reference));
        let type_name = graph.type_name(Some(reference));
        if object.is_none() && type_name.is_none() {
            ctx.low.report(
                codes::KN1008,
                DiagnosticSite::new().data("id", object_id.to_string()),
            );
            continue;
        }
        ctx.placed.insert(object_id);
        let (id, duplicate) = taken_id(ctx, object_id)?;
        if duplicate {
            ctx.low.report(
                codes::KN1010,
                DiagnosticSite::new()
                    .node(id.clone())
                    .data("id", object_id.to_string()),
            );
        }
        let Some(object) = object else {
            out.push(undecoded(ctx, id, object_id, type_name));
            continue;
        };
        if let Some(element) = lower_object(ctx, &object, &id, depth)? {
            out.push(element);
        }
    }
    Ok(out)
}
