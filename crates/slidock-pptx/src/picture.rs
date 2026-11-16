//! Pictures (`p:pic`): the image as media, its crop as fractions (negative values pad), a non-rectangular shape as a
//! clip, `alphaModFix` as the picture's opacity, and its outline. A picture that holds a movie is a video with the
//! picture as its poster; one that holds sound keeps showing its picture, and the sound is dropped.

use slidock_canon::{js_number, to_js_string};
use slidock_core_api::lowering::LowerError;
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::elements::{Crop, MediaKind, Trim};
use slidock_ir::features::FeatureId;
use slidock_ir::geometry::Geometry;
use slidock_ir::ids::{NodeId, Sha256};
use slidock_ir::source::{Element as IrElement, MediaElement, Picture};
use slidock_ir::units::{Ms, Ratio, UnitRatio};

use crate::clip;
use crate::context::Lowering;
use crate::element::{Base, blip_media, element_base, frame_of};
use crate::fill::PaintCx;
use crate::geometry::{ReadGeometry, read_geometry};
use crate::line::read_line;
use crate::shadow::element_shadow;
use crate::style_refs::{read_effect_ref, read_style};
use crate::units::ratio;
use crate::xml::{El, Element, Ns};

/// A crop leaving less than this share of an axis is declined rather than printed as a sliver.
const MIN_VISIBLE: f64 = 0.005;

/// Vector picture types, which plan-media declines.
const VECTOR_TYPES: [&str; 4] = ["image/svg+xml", "image/emf", "image/wmf", "application/pdf"];

/// Blip children that keep the picture's colours: `alphaModFix` (read as opacity) and `extLst`. Others recolour it.
const READ_EFFECTS: [&str; 2] = ["alphaModFix", "extLst"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum MovieKind {
    Video,
    Audio,
}

struct Movie<'e> {
    kind: MovieKind,
    rel: Option<&'e str>,
}

/// The movie or sound a picture holds, from its DrawingML video or audio file element.
fn movie_of(nv_pr: Option<&Element>) -> Option<Movie<'_>> {
    for element in nv_pr.kids() {
        if element.ns != Ns::A {
            continue;
        }
        match &*element.local {
            "videoFile" | "quickTimeFile" => {
                return Some(Movie {
                    kind: MovieKind::Video,
                    rel: element.rel_attr("link"),
                });
            }
            "audioFile" | "wavAudioFile" | "audioCd" => {
                return Some(Movie {
                    kind: MovieKind::Audio,
                    rel: element
                        .rel_attr("link")
                        .or_else(|| element.rel_attr("embed")),
                });
            }
            _ => {}
        }
    }
    None
}

/// The embedded bytes of a movie: `p14:media` in the non-visual properties' extensions.
fn embedded_media(nv_pr: Option<&Element>) -> Option<&Element> {
    nv_pr
        .child(Ns::P, "extLst")
        .kids()
        .into_iter()
        .find_map(|ext| {
            ext.kids()
                .find(|kid| kid.is(Ns::P14, "media"))
                .map(|rc| &**rc)
        })
}

fn movie_bytes(
    movie: &Movie<'_>,
    nv_pr: Option<&Element>,
    base: &Base,
    ctx: &mut Lowering<'_>,
) -> Result<Option<Sha256>, LowerError> {
    let embedded = embedded_media(nv_pr);
    let rel = ctx
        .rels
        .get(embedded.rel_attr("embed").unwrap_or(""))
        .or_else(|| ctx.rels.get(movie.rel.unwrap_or("")))
        .cloned();
    let rel = match rel {
        Some(rel) if !rel.external => rel,
        rel => {
            let (target, reason) = match &rel {
                Some(rel) => (clip(&rel.target, 256), "linked"),
                None => ("(none)".to_owned(), "missing"),
            };
            ctx.report_on(
                &base.id,
                codes::PX2306,
                DiagnosticSite::new()
                    .data("target", target)
                    .data("reason", reason),
            );
            return Ok(None);
        }
    };
    match ctx.media.add(ctx.pkg, &rel.target) {
        Ok(sha256) => Ok(Some(sha256)),
        Err(LowerError::Part { part, reason }) => {
            ctx.report_on(
                &base.id,
                codes::PX2306,
                DiagnosticSite::new()
                    .data("target", part)
                    .data("reason", reason),
            );
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn ms(value: f64) -> Ms {
    Ms(value.clamp(0.0, f64::from(u32::MAX)) as u32)
}

/// `p14:trim`: milliseconds cut from the start and from the end. The IR's end is a time in the movie, so the cut
/// from the end needs the movie's length; when the length is unknown the end is dropped (PX2309).
fn trim_of(
    nv_pr: Option<&Element>,
    media: Option<&Sha256>,
    node: &NodeId,
    ctx: &mut Lowering<'_>,
) -> Option<Trim> {
    let trim = embedded_media(nv_pr).child(Ns::P14, "trim")?;
    let cut_ms = |name: &str| {
        let value = trim.attr(name).map_or(0.0, js_number);
        if value.is_finite() && value > 0.0 {
            slidock_canon::js_round(value).min(3_600_000.0)
        } else {
            0.0
        }
    };
    let (start, cut) = (cut_ms("st"), cut_ms("end"));
    let length = media.and_then(|media| ctx.media.duration(media));
    let end = length
        .filter(|length| cut != 0.0 && length - cut > start)
        .map(|length| length - cut);
    if cut != 0.0 && end.is_none() {
        ctx.report_on(node, codes::PX2309, DiagnosticSite::new().data("end", cut));
    }
    (start != 0.0 || end.is_some()).then(|| Trim {
        start_ms: ms(start),
        end_ms: end.map(ms),
    })
}

/// An `a:srcRect` as fractions of the image per side.
fn crop_of(blip_fill: &Element) -> Option<Crop> {
    let rect = blip_fill.child(Ns::A, "srcRect");
    let side = |name: &str| ratio(rect.int_attr(name).unwrap_or(0) as f64 / 100_000.0);
    let crop = Crop {
        l: side("l"),
        t: side("t"),
        r: side("r"),
        b: side("b"),
    };
    (rect.is_some() && crop.is_set()).then_some(crop)
}

pub(crate) fn lower_picture(
    element: &Element,
    at: &str,
    ctx: &mut Lowering<'_>,
) -> Result<Option<IrElement>, LowerError> {
    let base = element_base(element, at, ctx)?;
    let nv_pr = element.path(&[(Ns::P, "nvPicPr"), (Ns::P, "nvPr")]);
    let sp_pr = element.child(Ns::P, "spPr").map(|rc| &**rc);
    let frame = frame_of(sp_pr.child(Ns::A, "xfrm"), &base, ctx);
    let blip_fill = element.child(Ns::P, "blipFill").map(|rc| &**rc);
    let movie = movie_of(nv_pr);

    if let Some(movie) = movie
        .as_ref()
        .filter(|movie| movie.kind == MovieKind::Video)
    {
        ctx.seen(FeatureId::ElementVideo);
        let poster = match blip_fill {
            Some(blip_fill) => blip_media(blip_fill, ctx, Some(&base.id), FeatureId::ElementVideo)?,
            None => None,
        };
        let media = movie_bytes(movie, nv_pr, &base, ctx)?;
        let trim = trim_of(nv_pr, media.as_ref(), &base.id, ctx);
        let style = element.child(Ns::P, "style").map(|rc| &**rc);
        let effect = read_effect_ref(style, ctx.formats, ctx);
        let shadow = element_shadow(sp_pr, effect.as_ref(), &base.id, ctx);
        return Ok(Some(IrElement::Media(Box::new(MediaElement {
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
            media_kind: MediaKind::Video,
            media,
            poster,
            trim,
            looped: false,
            muted: false,
            autoplay: false,
            // A media node's `numSld` keeps one element playing past its slide (reported by the timing reader); no
            // PresentationML field carries a clip on into another slide's element.
            across_slides: false,
            shadow,
        }))));
    }
    if movie.is_some() {
        ctx.seen(FeatureId::ElementAudio);
        ctx.report_on(&base.id, codes::PX2304, DiagnosticSite::new());
    }

    ctx.seen(FeatureId::ElementPicture);
    let Some(blip_fill) = blip_fill else {
        ctx.report_on(
            &base.id,
            codes::PX2307,
            DiagnosticSite::new()
                .data("part", "(none)")
                .data("reason", "no picture is referenced"),
        );
        return Ok(None);
    };
    let Some(media) = blip_media(blip_fill, ctx, Some(&base.id), FeatureId::ElementPicture)? else {
        return Ok(None);
    };
    if let Some(entry) = ctx.media.entry(&media) {
        if entry.frames.unwrap_or(1) > 1 {
            ctx.seen(FeatureId::ImageAnimated);
        }
        if VECTOR_TYPES.contains(&entry.mime.as_str()) {
            ctx.seen(FeatureId::ImageVector);
        }
    }

    let crop = crop_of(blip_fill);
    if let Some(crop) = &crop {
        let [l, t, r, b] = [crop.l, crop.t, crop.r, crop.b].map(Ratio::to_f64);
        if 1.0 - l - r < MIN_VISIBLE || 1.0 - t - b < MIN_VISIBLE {
            let written = [l, t, r, b].map(to_js_string).join(" ");
            ctx.report_on(
                &base.id,
                codes::PX2305,
                DiagnosticSite::new().data("crop", written),
            );
            return Ok(None);
        }
        ctx.seen(FeatureId::ImageCrop);
    }
    let fill_rect = blip_fill.path(&[(Ns::A, "stretch"), (Ns::A, "fillRect")]);
    if fill_rect.is_some()
        && ["l", "t", "r", "b"]
            .iter()
            .any(|name| fill_rect.int_attr(name).unwrap_or(0) != 0)
    {
        ctx.report_on(&base.id, codes::PX2303, DiagnosticSite::new());
    }
    if blip_fill.child(Ns::A, "tile").is_some() {
        ctx.seen(FeatureId::FillTile);
        ctx.report_on(
            &base.id,
            codes::PX2104,
            DiagnosticSite::new().feature(FeatureId::FillTile),
        );
    }

    let blip = blip_fill.child(Ns::A, "blip");
    let amount = match blip.child(Ns::A, "alphaModFix") {
        Some(alpha) => alpha.int_attr("amt").unwrap_or(100_000).clamp(0, 100_000),
        None => 100_000,
    };
    let opacity = UnitRatio::from_steps(ratio(amount as f64 / 100_000.0).steps());
    if opacity.to_f64() < 1.0 {
        ctx.seen(FeatureId::ImageOpacity);
    }
    let recolor: Vec<&str> = blip
        .kids()
        .into_iter()
        .filter(|effect| effect.ns == Ns::A && !READ_EFFECTS.contains(&&*effect.local))
        .map(|effect| &*effect.local)
        .collect();
    if !recolor.is_empty() {
        ctx.seen(FeatureId::ImageRecolor);
        ctx.report_on(
            &base.id,
            codes::PX2302,
            DiagnosticSite::new().data("effects", recolor.join(", ")),
        );
    }

    let ext = sp_pr.child(Ns::A, "xfrm").child(Ns::A, "ext");
    let size = (
        ext.int_attr("cx").unwrap_or(0),
        ext.int_attr("cy").unwrap_or(0),
    );
    let mut geometry = read_geometry(sp_pr, size, ctx.limits.path_segments)?;
    let clip_geometry = match &mut geometry {
        ReadGeometry::Geometry(Geometry::Preset(preset), _) if preset.name.as_str() == "rect" => {
            None
        }
        ReadGeometry::Geometry(geometry, nan) => {
            ctx.not_a_number(&base.id, "clip", std::mem::take(nan));
            Some(geometry.clone())
        }
        _ => None,
    };
    if clip_geometry.is_some() {
        ctx.seen(FeatureId::ImageClip);
    }
    if let ReadGeometry::Unknown(detail) = &geometry {
        ctx.report_on(
            &base.id,
            codes::PX2308,
            DiagnosticSite::new().data("geometry", clip(detail, 64)),
        );
    }

    let mut paint = PaintCx::new(ctx, Some(base.id.clone()));
    let style = read_style(element.child(Ns::P, "style").map(|rc| &**rc), &mut paint)?;
    let stroke = read_line(
        sp_pr.child(Ns::A, "ln"),
        style.as_ref().and_then(|style| style.line.as_ref()),
        &mut paint,
        true,
    );
    let shadow = element_shadow(
        sp_pr,
        style.and_then(|style| style.effect).as_ref(),
        &base.id,
        ctx,
    );

    Ok(Some(IrElement::Picture(Box::new(Picture {
        id: base.id,
        origin: base.origin,
        name: base.name,
        placeholder: base.placeholder,
        inherits: None,
        match_key: None,
        hidden: base.hidden,
        opacity: (opacity.to_f64() < 1.0).then_some(opacity),
        link: base.link,
        alt: base.alt,
        ext: None,
        frame,
        media,
        fallback: None,
        crop,
        clip: clip_geometry,
        mask: None,
        stroke,
        shadow,
    }))))
}
