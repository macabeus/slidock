//! A Foreign node prints its fallback, else a visible marker. Its dialect stays out of the document.

use slidock_format::markers::FOREIGN_MARKER;
use slidock_ir::resolved::Foreign;

use crate::attributes::{Kind, Root, common, element_attributes};
use crate::context::PrintCtx;
use crate::css::{decl, style};
use crate::error::Result;
use crate::frame::{box_decls, opacity_decl};
use crate::tree::{Attrs, Element, lines, text};

/// A full-box marker that says what is missing, its label at the box's centre: a box that runs off the slide keeps
/// its centre on it far more often than its top-left corner (a full-bleed picture).
pub fn marker(label: &str) -> Element {
    let css = style([
        decl("position", "absolute"),
        decl("inset", "0"),
        decl("display", "flex"),
        decl("align-items", "center"),
        decl("justify-content", "center"),
        decl("text-align", "center"),
        decl("outline", "1pt dashed #c00"),
        decl("outline-offset", "-1pt"),
        decl("font", "9pt sans-serif"),
        decl("color", "#c00"),
    ]);
    Element::new(
        "div",
        Attrs::new().flag(FOREIGN_MARKER, true).val("style", css),
        vec![text(label)],
    )
}

/// A media item drawn over the whole box, as Foreign fallbacks and video posters are.
pub fn full_image(src: String, alt: Option<&str>) -> Element {
    let css = style([
        decl("position", "absolute"),
        decl("left", "0pt"),
        decl("top", "0pt"),
        decl("width", "100%"),
        decl("height", "100%"),
        decl("object-fit", "fill"),
    ]);
    Element::new(
        "img",
        Attrs::new()
            .val("src", src)
            .val("alt", alt.unwrap_or("").to_owned())
            .val("style", css),
        Vec::new(),
    )
}

pub fn print(element: &Foreign, ctx: &mut PrintCtx<'_>) -> Result<Element> {
    let mut children = Vec::new();
    let fallback = element.fallback.as_ref();
    if let Some(media) = fallback.and_then(|fallback| fallback.media.as_ref())
        && let Some(src) = ctx.media_ref(media)?
    {
        children.push(full_image(src, element.alt.as_deref()));
    }
    if let Some(elements) = fallback.and_then(|fallback| fallback.elements.as_ref()) {
        let axes = ctx.axes.then_frame(&element.frame);
        children.extend(ctx.print_in(axes, elements)?);
    }
    let foreign_type = element.foreign_type.as_str();
    if children.is_empty() {
        children.push(marker(&format!("Unsupported {foreign_type}")));
    }
    let mut decls = box_decls(&element.frame, None)?;
    decls.push(opacity_decl(element.opacity)?);
    let attrs = element_attributes(common!(element, Kind::Foreign), ctx, Root::Div)?
        .val("data-foreign", foreign_type.to_owned())
        .val("style", style(decls));
    Ok(Element::new("div", attrs, lines(children)))
}
