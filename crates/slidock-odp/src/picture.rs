//! Pictures: a package file an `xlink:href` names, recorded as media, and the picture frames that show one. A reference
//! outside the package and a missing or unreadable file are declined (OD9303, OD9304). A frame's crop (`fo:clip`) is in
//! lengths against the picture's intrinsic size, so it becomes a fraction through the picture's pixels and stated
//! resolution; its opacity, mirroring and outline carry over, colour adjustments are dropped (OD9305).

use slidock_canon::utf16;
use slidock_core_api::lowering::LowerError;
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::elements::Crop;
use slidock_ir::features::{FeatureId, VECTOR_MEDIA};
use slidock_ir::ids::{NodeId, Sha256};
use slidock_ir::source::{Element as IrElement, Picture};
use slidock_ir::units::{Ratio, UnitRatio};
use slidock_math::{Grid, quantise};

use crate::context::Lowering;
use crate::element::{Base, Flips, frame_of};
use crate::graphic::{Paint, opacity_of, read_shadow, read_stroke, shadow_field};
use crate::media::MediaError;
use crate::names::qn;
use crate::odf::Element;
use crate::package::{Reference, resolve_reference};
use crate::styles::{PropertySet, Props, get};
use crate::units::{is_space, percent, raw_length};

/// The media an `xlink:href` names; `None` after declining it, or quietly when `optional` (a fallback image).
pub(crate) fn picture_media<'a>(
    href: &str,
    ctx: &Lowering<'a, '_>,
    feature: FeatureId,
    node: Option<&NodeId>,
    optional: bool,
) -> Result<Option<Sha256>, LowerError> {
    let site = || DiagnosticSite::new().node(node.cloned()).feature(feature);
    let path = match resolve_reference(href) {
        Reference::External(target) => {
            if !optional {
                ctx.report(codes::OD9303, site().data("target", target));
            }
            return Ok(None);
        }
        Reference::Internal(path) => path,
    };
    match ctx.media.borrow_mut().add(ctx.pkg, &path) {
        Ok(sha256) => Ok(Some(sha256)),
        Err(MediaError::Part(error)) => {
            if !optional {
                ctx.report(
                    codes::OD9304,
                    site()
                        .data("part", utf16::clip(&error.part, 256).to_owned())
                        .data("reason", error.reason),
                );
            }
            Ok(None)
        }
        Err(MediaError::Read(reason)) => Err(LowerError::Other(reason)),
    }
}

/// A crop leaving less than this share of an axis is declined rather than printed as a sliver.
const MIN_VISIBLE: f64 = 0.005;

/// `fo:clip="rect(top, right, bottom, left)"` as lengths in points; `auto` reads as 0.
fn clip_lengths(value: Option<&str>) -> Option<[f64; 4]> {
    let value = value?.trim_matches(is_space);
    let inner = value.strip_prefix("rect(")?.strip_suffix(')')?;
    if inner.contains(')') {
        return None;
    }
    let sides: Vec<Option<f64>> = split_sides(slidock_canon::js_trim(inner))
        .into_iter()
        .map(|side| {
            if side == "auto" {
                Some(0.0)
            } else {
                raw_length(Some(side))
            }
        })
        .collect();
    let sides: Vec<f64> = sides.into_iter().collect::<Option<Vec<f64>>>()?;
    let [t, r, b, l] = <[f64; 4]>::try_from(sides).ok()?;
    (t != 0.0 || r != 0.0 || b != 0.0 || l != 0.0).then_some([t, r, b, l])
}

/// `text.split(/\s*,\s*|\s+/)`.
fn split_sides(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    loop {
        // The first separator: spaces around a comma, or a run of spaces.
        let Some(at) = rest.find(|c: char| c == ',' || is_space(c)) else {
            out.push(rest);
            return out;
        };
        out.push(rest.get(..at).unwrap_or_default());
        let after = rest.get(at..).unwrap_or_default();
        let spaces = after.trim_start_matches(is_space);
        rest = match spaces.strip_prefix(',') {
            Some(after_comma) => after_comma.trim_start_matches(is_space),
            None => spaces,
        };
    }
}

/// A picture style's non-neutral colour adjustments.
fn adjustments(graphic: &PropertySet) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(mode) = get(graphic, qn!("draw:color-mode"))
        && !mode.is_empty()
        && mode != "standard"
    {
        out.push(mode.to_owned());
    }
    for name in [
        qn!("draw:luminance"),
        qn!("draw:contrast"),
        qn!("draw:red"),
        qn!("draw:green"),
        qn!("draw:blue"),
    ] {
        if percent(get(graphic, name)).unwrap_or(0.0) != 0.0 {
            out.push(name.as_str().trim_start_matches("draw:").to_owned());
        }
    }
    if percent(get(graphic, qn!("draw:gamma"))).unwrap_or(1.0) != 1.0 {
        out.push("gamma".to_owned());
    }
    if get(graphic, qn!("draw:color-inversion")) == Some("true") {
        out.push("inversion".to_owned());
    }
    out
}

/// The picture a `draw:image` puts in a frame; `None` after declining it.
pub(crate) fn lower_picture<'a>(
    frame_element: &'a Element,
    image: &'a Element,
    base: Base,
    props: &Props<'a>,
    ctx: &Lowering<'a, '_>,
) -> Result<Option<IrElement>, LowerError> {
    let graphic = &props.graphic;
    let mirror = get(graphic, qn!("style:mirror")).unwrap_or("none");
    let frame = frame_of(
        frame_element,
        &base,
        ctx,
        Flips {
            flip_h: mirror.contains("horizontal"),
            flip_v: mirror.contains("vertical"),
        },
    );
    ctx.see(FeatureId::ElementPicture);
    let href = image.attr(qn!("xlink:href")).unwrap_or("");
    let Some(media) = picture_media(href, ctx, FeatureId::ElementPicture, Some(&base.id), false)?
    else {
        return Ok(None);
    };
    let (frames, vector) = {
        let registry = ctx.media.borrow();
        let entry = registry.entry(&media);
        (
            entry.and_then(|entry| entry.frames).unwrap_or(1),
            entry.is_some_and(|entry| VECTOR_MEDIA.contains(&entry.mime.as_str())),
        )
    };
    if frames > 1 {
        ctx.see(FeatureId::ImageAnimated);
    }
    if vector {
        ctx.see(FeatureId::ImageVector);
    }

    let mut crop = None;
    let clip = clip_lengths(get(graphic, qn!("fo:clip")));
    let extent = ctx.media.borrow().extent(&media);
    match (clip, extent) {
        (Some(_), None) => ctx.report(
            codes::OD9301,
            DiagnosticSite::new()
                .node(base.id.clone())
                .feature(FeatureId::ImageCrop),
        ),
        (Some([t, r, b, l]), Some(extent)) => {
            let side = |length: f64, of: f64| {
                quantise(
                    if of > 0.0 { length / of } else { 0.0 },
                    Grid::HundredThousandths,
                )
            };
            let (cl, ct, cr, cb) = (
                side(l, extent.w),
                side(t, extent.h),
                side(r, extent.w),
                side(b, extent.h),
            );
            if 1.0 - cl - cr < MIN_VISIBLE || 1.0 - ct - cb < MIN_VISIBLE {
                let js = slidock_canon::to_js_string;
                ctx.report(
                    codes::OD9302,
                    DiagnosticSite::new().node(base.id.clone()).data(
                        "crop",
                        format!("{} {} {} {}", js(cl), js(ct), js(cr), js(cb)),
                    ),
                );
                return Ok(None);
            }
            if cl != 0.0 || ct != 0.0 || cr != 0.0 || cb != 0.0 {
                ctx.see(FeatureId::ImageCrop);
                let ratio = |x: f64| Ratio::quantise(x).unwrap_or(Ratio::ZERO);
                crop = Some(Crop {
                    l: ratio(cl),
                    t: ratio(ct),
                    r: ratio(cr),
                    b: ratio(cb),
                });
            }
        }
        (None, _) => {}
    }

    let opacity = quantise(
        opacity_of(get(graphic, qn!("draw:image-opacity"))),
        Grid::HundredThousandths,
    );
    if opacity < 1.0 {
        ctx.see(FeatureId::ImageOpacity);
    }
    let recolor = adjustments(graphic);
    if !recolor.is_empty() {
        ctx.see(FeatureId::ImageRecolor);
        ctx.report(
            codes::OD9305,
            DiagnosticSite::new()
                .node(base.id.clone())
                .data("adjustments", recolor.join(", ")),
        );
    }
    let paint = Paint::of(*ctx, Some(&base.id));
    let line = read_stroke(graphic, &paint);
    for feature in line.features.iter().copied() {
        ctx.see(feature);
    }
    let shadow = shadow_field(read_shadow(graphic), base.placeholder.is_some(), ctx);
    Ok(Some(IrElement::Picture(Box::new(Picture {
        id: base.id,
        origin: base.origin,
        name: base.name,
        placeholder: base.placeholder,
        inherits: None,
        match_key: None,
        hidden: base.hidden,
        opacity: (opacity < 1.0).then(|| UnitRatio::quantise(opacity).unwrap_or(UnitRatio::ZERO)),
        link: base.link,
        alt: base.alt,
        ext: None,
        frame,
        media,
        fallback: None,
        crop,
        clip: None,
        mask: None,
        stroke: line.stroke,
        shadow,
    }))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_clip_rectangles() {
        assert_eq!(
            clip_lengths(Some("rect(0in, 0.25in, 0in, 0.25in)")),
            Some([0.0, 18.0, 0.0, 18.0])
        );
        assert_eq!(
            clip_lengths(Some(" rect(1pt 2pt auto 4pt) ")),
            Some([1.0, 2.0, 0.0, 4.0])
        );
        assert_eq!(clip_lengths(Some("rect(0in, 0in, 0in, 0in)")), None);
        assert_eq!(clip_lengths(Some("rect(1pt, 2pt, 3pt)")), None);
        assert_eq!(clip_lengths(Some("rect(1pt, x, 3pt, 4pt)")), None);
        assert_eq!(clip_lengths(Some("rect()")), None);
    }

    #[test]
    fn splits_clip_sides() {
        assert_eq!(split_sides("1, 2 ,3 4"), ["1", "2", "3", "4"]);
        assert_eq!(split_sides("1,,2"), ["1", "", "2"]);
    }
}
