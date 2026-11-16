//! Geometry as inline SVG in the element's own points: the viewBox is the frame, so strokes keep their width.

use slidock_format::markers::GEOMETRY;
use slidock_ir::geometry::{Seg, Segment};
use slidock_ir::units::Pt;

use crate::css::{decl, decl_if, fixed, style};
use crate::error::Result;
use crate::tree::{Attrs, Element, lines};

/// Path data from IR segments, in points times `scale`.
pub fn path_data(segments: &[Segment], scale: f64) -> Result<String> {
    let mut out = String::new();
    for segment in segments {
        out.push_str(segment.command());
        for (i, value) in segment.coords().iter().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            out.push_str(&fixed(value.to_f64() * scale)?);
        }
    }
    Ok(out)
}

/// The rectangle from the origin to `(w, h)`, as lower-geometry writes a `rect`.
pub fn rect(w: Pt, h: Pt) -> [Segment; 5] {
    [
        Seg::M(Pt::ZERO, Pt::ZERO),
        Seg::L(w, Pt::ZERO),
        Seg::L(w, h),
        Seg::L(Pt::ZERO, h),
        Seg::Z,
    ]
}

/// The SVG layer of an element whose box is `width` × `height` points. A zero-width or zero-height box (a straight
/// connector) gets a 1 pt viewport on that axis, since a zero viewBox renders nothing; its paths overflow it visibly.
pub fn svg_layer(width: f64, height: f64, children: Vec<Element>) -> Result<Element> {
    let degenerate = width <= 0.0 || height <= 0.0;
    let vw = fixed(if width > 0.0 { width } else { 1.0 })?;
    let vh = fixed(if height > 0.0 { height } else { 1.0 })?;
    let css = style([
        decl("position", "absolute"),
        decl("left", "0pt"),
        decl("top", "0pt"),
        decl("overflow", "visible"),
        decl_if(degenerate, "width", || format!("{vw}pt")),
        decl_if(degenerate, "height", || format!("{vh}pt")),
    ]);
    Ok(Element::new(
        "svg",
        Attrs::new()
            .flag(GEOMETRY, true)
            .val("viewBox", format!("0 0 {vw} {vh}"))
            .val("preserveAspectRatio", "none".to_owned())
            .val("width", "100%".to_owned())
            .val("height", "100%".to_owned())
            .val("style", css),
        lines(children),
    ))
}
