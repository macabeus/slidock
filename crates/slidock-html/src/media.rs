//! Video and audio: a `<video>` with its poster, paused in a static document (the player plays it). The poster is what
//! thumbnails and the static render show, so a video without one is a visible loss (HTML6005). Audio has nothing to
//! show; it prints an empty box and the backend names the loss (HTML6003). A video's shadow is its poster rectangle's
//! box shadow; audio and a video without a poster cast none. `data-across-slides` marks a clip the
//! player carries on into the next slide's same clip (`docs/format.md` §3).

use slidock_ir::diagnostics::codes;
use slidock_ir::elements::MediaKind;
use slidock_ir::resolved::MediaElement;

use crate::attributes::{Kind, Root, common, element_attributes};
use crate::context::PrintCtx;
use crate::css::{decl, number, style};
use crate::error::Result;
use crate::foreign::{full_image, marker};
use crate::frame::{box_decls, is_empty_box, opacity_decl};
use crate::shadow::{ShadowForm, shadow_decl};
use crate::tree::{Attrs, Element, lines};

/// A video's content, and whether it shows the poster (a marker stands in for a video without one).
fn video_content(element: &MediaElement, ctx: &mut PrintCtx<'_>) -> Result<(Element, bool)> {
    let poster = match &element.poster {
        Some(poster) => ctx.media_ref(poster)?,
        None => None,
    };
    let Some(poster) = poster else {
        let site = ctx
            .site()
            .node(element.id.clone())
            .origin(element.origin.clone());
        ctx.diag(codes::HTML6005, site);
        return Ok((marker("Video without a poster"), false));
    };
    let src = match &element.media {
        Some(media) => ctx.media_ref(media)?,
        None => None,
    };
    let Some(src) = src else {
        return Ok((full_image(poster, element.alt.as_deref()), true));
    };
    let trim = match &element.trim {
        Some(trim) => Some(format!(
            "{}-{}",
            number(f64::from(trim.start_ms.0))?,
            match trim.end_ms {
                Some(end) => number(f64::from(end.0))?,
                None => String::new(),
            }
        )),
        None => None,
    };
    let css = style([
        decl("position", "absolute"),
        decl("left", "0pt"),
        decl("top", "0pt"),
        decl("width", "100%"),
        decl("height", "100%"),
        decl("object-fit", "fill"),
    ]);
    let video = Element::new(
        "video",
        Attrs::new()
            .val("src", src)
            .val("poster", poster)
            .val("preload", "none".to_owned())
            .flag("playsinline", true)
            .flag("muted", element.muted)
            .flag("loop", element.looped)
            .val("style", css)
            .flag("data-loop", element.looped)
            .flag("data-muted", element.muted)
            .flag("data-autoplay", element.autoplay)
            .val("data-trim", trim)
            .flag("data-across-slides", element.across_slides),
        Vec::new(),
    );
    Ok((video, true))
}

pub fn print(element: &MediaElement, ctx: &mut PrintCtx<'_>) -> Result<Element> {
    let (children, poster) = match element.media_kind {
        MediaKind::Video => {
            let (content, poster) = video_content(element, ctx)?;
            (vec![content], poster)
        }
        MediaKind::Audio => (Vec::new(), false),
    };
    let mut decls = box_decls(&element.frame, None)?;
    decls.push(opacity_decl(element.opacity)?);
    // The poster's rectangle casts the shadow; a filter never sits over a playing video.
    if poster && !is_empty_box(&element.frame, None) {
        decls.push(shadow_decl(
            element.shadow.as_ref(),
            &ctx.axes.then_frame(&element.frame),
            ShadowForm::Box,
        )?);
    }
    let attrs = element_attributes(common!(element, Kind::Media), ctx, Root::Div)?
        .val("style", style(decls));
    Ok(Element::new("div", attrs, lines(children)))
}
