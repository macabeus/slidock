//! A picture: an opaque image filling its box uncut is the element itself. Else a clipping box holds the image, placed
//! by percentages so a crop survives any resize. Values the CSS derives non-invertibly (crop, clip and mask paths) are
//! also printed as IR values in data attributes. A shadowed picture keeps its box, which casts the shadow (with its
//! opacity), and clips in a child of its own (`data-sd-clip`): an element's clip cuts its own shadow.

use slidock_format::markers::{MASK, SHADOW_CLIP};
use slidock_ir::geometry::{FillRule, Path};
use slidock_ir::resolved::Picture;
use slidock_ir::units::Pt;

use crate::attributes::{Kind, Root, common, element_attributes};
use crate::context::PrintCtx;
use crate::css::{MaybeDecl, PX_PER_PT, decl, percent, ratio, style};
use crate::error::Result;
use crate::foreign::marker;
use crate::frame::{box_decls, opacity_decl};
use crate::paint::{stroke_attributes, stroke_color, strokes};
use crate::shadow::{ShadowForm, shadow_decl};
use crate::svg::{path_data, rect, svg_layer};
use crate::tree::{Attrs, Element, lines};

/// At least this much of the image stays visible on each axis; frontends decline crops below it.
const MIN_VISIBLE: f64 = 0.005;

fn clip_path(paths: &[Path], scale: f64) -> Result<String> {
    let mut out = String::new();
    for path in paths {
        out.push_str(&path_data(&path.segments, scale)?);
    }
    Ok(out)
}

/// `clip-path: path()` in the box's CSS pixels, honouring an even-odd rule.
fn clip_decl(paths: &[Path]) -> Result<MaybeDecl> {
    let evenodd = if paths.iter().any(|path| path.fill_rule == FillRule::Evenodd) {
        "evenodd, "
    } else {
        ""
    };
    Ok(decl(
        "clip-path",
        format!("path({evenodd}\"{}\")", clip_path(paths, PX_PER_PT)?),
    ))
}

/// The image, its opacity on it unless its box holds that opacity (`opacity_on_box`).
fn image_element(picture: &Picture, src: String, opacity_on_box: bool) -> Result<Element> {
    let (l, t, r, b) = match &picture.crop {
        Some(crop) => (
            crop.l.to_f64(),
            crop.t.to_f64(),
            crop.r.to_f64(),
            crop.b.to_f64(),
        ),
        None => (0.0, 0.0, 0.0, 0.0),
    };
    let sx = 1.0 / f64::max(MIN_VISIBLE, 1.0 - l - r);
    let sy = 1.0 / f64::max(MIN_VISIBLE, 1.0 - t - b);
    let css = style([
        decl("position", "absolute"),
        decl("left", percent(-l * sx * 100.0)?),
        decl("top", percent(-t * sy * 100.0)?),
        decl("width", percent(sx * 100.0)?),
        decl("height", percent(sy * 100.0)?),
        if opacity_on_box {
            None
        } else {
            opacity_decl(picture.opacity)?
        },
    ]);
    Ok(Element::new(
        "img",
        Attrs::new()
            .val("src", src)
            .val("alt", picture.alt.clone().unwrap_or_default())
            .val("style", css),
        Vec::new(),
    ))
}

/// The picture's outline: its clip shape, or its box.
fn stroke_layer(picture: &Picture) -> Result<Option<Element>> {
    let Some(stroke) = strokes(picture.stroke.as_ref().and_then(Option::as_ref)) else {
        return Ok(None);
    };
    let frame = &picture.frame;
    let outline = rect(
        Pt::from_steps(frame.w.steps()),
        Pt::from_steps(frame.h.steps()),
    );
    let segments: Vec<&[_]> = match &picture.clip {
        Some(clip) => clip
            .paths
            .iter()
            .map(|path| path.segments.as_slice())
            .collect(),
        None => vec![outline.as_slice()],
    };
    let color = stroke_color(stroke)?;
    let mut paths = Vec::with_capacity(segments.len());
    for segments in segments {
        paths.push(Element::new(
            "path",
            Attrs::new()
                .val("d", path_data(segments, 1.0)?)
                .with(stroke_attributes(stroke, crate::css::fixed)?)
                .val("style", format!("fill:none;stroke:{color}")),
            Vec::new(),
        ));
    }
    Ok(Some(svg_layer(frame.w.to_f64(), frame.h.to_f64(), paths)?))
}

pub fn print(picture: &Picture, ctx: &mut PrintCtx<'_>) -> Result<Element> {
    let src = ctx.media_ref(&picture.media)?;
    let outline = stroke_layer(picture)?;
    let translucent = opacity_decl(picture.opacity)?.is_some();
    let shadow = shadow_decl(
        picture.shadow.as_ref(),
        &ctx.axes.then_frame(&picture.frame),
        ShadowForm::Drop,
    )?;
    // An opaque image filling its box uncut is the element, clipped at its own box so its edges anti-alias as a
    // clipping box's do. A translucent one keeps the box, where its opacity composites inside the clip. A shadowed
    // one keeps the box too, since the clip would cut its shadow.
    if let Some(src) = &src
        && picture.crop.is_none()
        && picture.clip.is_none()
        && picture.mask.is_none()
        && outline.is_none()
        && !translucent
        && shadow.is_none()
    {
        let mut decls = box_decls(&picture.frame, None)?;
        decls.push(decl("clip", "rect(auto, auto, auto, auto)"));
        let attrs = element_attributes(common!(picture, Kind::Picture), ctx, Root::Div)?
            .val("src", src.clone())
            .val("alt", picture.alt.clone().unwrap_or_default())
            .val("style", style(decls));
        return Ok(Element::new("img", attrs, Vec::new()));
    }
    let has_src = src.is_some();
    let mut content = match src {
        None => marker("Image not included"),
        Some(src) => image_element(picture, src, shadow.is_some())?,
    };
    // A mask inside a clip needs a box of its own, since one element holds one clip path.
    if let (Some(mask), Some(_), true) = (&picture.mask, &picture.clip, has_src) {
        content = Element::new(
            "div",
            Attrs::new().flag(MASK, true).val(
                "style",
                style([
                    decl("position", "absolute"),
                    decl("inset", "0"),
                    clip_decl(mask)?,
                ]),
            ),
            vec![content.into()],
        );
    }
    let outer_clip: Option<&[Path]> = match (&picture.clip, &picture.mask) {
        (Some(clip), _) => Some(&clip.paths),
        (None, Some(mask)) => Some(mask),
        (None, None) => None,
    };
    let crop = match &picture.crop {
        Some(crop) => Some(
            [crop.l, crop.t, crop.r, crop.b]
                .iter()
                .map(|side| ratio(side.to_f64()))
                .collect::<Result<Vec<_>>>()?
                .join(" "),
        ),
        None => None,
    };
    let clip = match &picture.clip {
        Some(clip) => Some(clip_path(&clip.paths, 1.0)?),
        None => None,
    };
    let mask = match &picture.mask {
        Some(mask) => Some(clip_path(mask, 1.0)?),
        None => None,
    };
    let mut clipping = vec![decl("overflow", "hidden")];
    if let Some(paths) = outer_clip.filter(|paths| !paths.is_empty()) {
        clipping.push(clip_decl(paths)?);
    }
    let mut children = vec![content];
    children.extend(outline);
    let mut decls = box_decls(&picture.frame, None)?;
    if shadow.is_some() {
        // The box casts the shadow of what its clipping child draws, as one group with its opacity.
        decls.push(opacity_decl(picture.opacity)?);
        decls.push(shadow);
        let mut inner = vec![decl("position", "absolute"), decl("inset", "0")];
        inner.extend(clipping);
        children = vec![Element::new(
            "div",
            Attrs::new()
                .flag(SHADOW_CLIP, true)
                .val("style", style(inner)),
            lines(children),
        )];
    } else {
        decls.extend(clipping);
    }
    let attrs = element_attributes(common!(picture, Kind::Picture), ctx, Root::Div)?
        .val("data-crop", crop)
        .val("data-clip", clip)
        .val("data-mask", mask)
        .val("style", style(decls));
    Ok(Element::new("div", attrs, lines(children)))
}
