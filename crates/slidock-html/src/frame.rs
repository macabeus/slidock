//! Where an element's box goes: left/top/width/height in points, then the rotation and the flips as the individual
//! `rotate` and `scale` properties. `translate` carries only the percentage anchors of autosized boxes. `transform` is
//! never printed, so the player can animate boxes without undoing a placement.

use slidock_ir::frame::Frame;
use slidock_ir::text::{Anchor, AnchorX, ResolvedTextBody, TextRotation};
use slidock_ir::units::{Size, UnitRatio};

use crate::css::{MaybeDecl, decl, deg, pt, ratio};
use crate::error::Result;

/// A text box that grows with its text: which axes, and the point it grows from.
#[derive(Clone, Debug, PartialEq)]
pub struct Autosize {
    pub width: bool,
    pub height: bool,
    pub anchor_x: AnchorX,
    pub anchor_y: Anchor,
    pub max_width: Option<Size>,
}

fn anchor_x(anchor: AnchorX) -> f64 {
    match anchor {
        AnchorX::Left => 0.0,
        AnchorX::Center => 0.5,
        AnchorX::Right => 1.0,
    }
}

fn anchor_y(anchor: Anchor) -> f64 {
    match anchor {
        Anchor::Top => 0.0,
        Anchor::Middle => 0.5,
        Anchor::Bottom => 1.0,
    }
}

fn anchor_percent(fraction: f64) -> &'static str {
    if fraction == 0.0 {
        "0"
    } else if fraction == 0.5 {
        "-50%"
    } else {
        "-100%"
    }
}

fn flip_scale(x: i8, y: i8) -> MaybeDecl {
    (x != 1 || y != 1).then(|| ("scale", format!("{x} {y}")))
}

fn sign(flipped: bool) -> i8 {
    if flipped { -1 } else { 1 }
}

/// The rotation and flips of a frame.
pub fn turn_decls(frame: &Frame) -> Result<[MaybeDecl; 2]> {
    let rotation = if frame.rotation.steps() != 0 {
        decl("rotate", deg(frame.rotation.to_f64())?)
    } else {
        None
    };
    Ok([rotation, flip_scale(sign(frame.flip_h), sign(frame.flip_v))])
}

pub fn opacity_decl(opacity: Option<UnitRatio>) -> Result<MaybeDecl> {
    match opacity {
        Some(opacity) if opacity.to_f64() < 1.0 => Ok(decl("opacity", ratio(opacity.to_f64())?)),
        _ => Ok(None),
    }
}

/// The paint-free extent a box of zero width or height carries (a straight connector's): WebKit composites an
/// opacity animation of an empty box apart from its overflowing content, which then paints at full opacity (an
/// arrowhead solid through a fade). A transparent 1 pt spread gives the box an extent of its own; it paints nothing.
const EMPTY_BOX_EXTENT: &str = "0 0 0 1pt transparent";

/// The engines' layout unit, 1/64 px, in points: a box side under it lays out as zero.
const LAYOUT_UNIT_PT: f64 = 0.75 / 64.0;

/// Whether an element's box lays out with a zero width or height (a straight connector's), which then carries the
/// paint-free extent of [`box_decls`].
pub fn is_empty_box(frame: &Frame, autosize: Option<&Autosize>) -> bool {
    let (w, h) = (frame.w.to_f64(), frame.h.to_f64());
    match autosize {
        Some(autosize) if autosize.width || autosize.height => {
            (!autosize.width && w < LAYOUT_UNIT_PT) || (!autosize.height && h < LAYOUT_UNIT_PT)
        }
        _ => w < LAYOUT_UNIT_PT || h < LAYOUT_UNIT_PT,
    }
}

/// An element's box. An autosized box is placed at its anchor point and grows from it.
pub fn box_decls(frame: &Frame, autosize: Option<&Autosize>) -> Result<Vec<MaybeDecl>> {
    let (x, y, w, h) = (
        frame.x.to_f64(),
        frame.y.to_f64(),
        frame.w.to_f64(),
        frame.h.to_f64(),
    );
    let mut decls = Vec::with_capacity(9);
    match autosize {
        Some(autosize) if autosize.width || autosize.height => {
            let fx = if autosize.width {
                anchor_x(autosize.anchor_x)
            } else {
                0.0
            };
            let fy = if autosize.height {
                anchor_y(autosize.anchor_y)
            } else {
                0.0
            };
            decls.push(decl("position", "absolute"));
            decls.push(decl("left", pt(x + w * fx)?));
            decls.push(decl("top", pt(y + h * fy)?));
            decls.push(decl(
                "width",
                if autosize.width {
                    "max-content".to_owned()
                } else {
                    pt(w)?
                },
            ));
            if let (true, Some(max)) = (autosize.width, autosize.max_width) {
                decls.push(decl("max-width", pt(max.to_f64())?));
            }
            if !autosize.height {
                decls.push(decl("height", pt(h)?));
            }
            if fx != 0.0 || fy != 0.0 {
                decls.push(decl(
                    "translate",
                    format!("{} {}", anchor_percent(fx), anchor_percent(fy)),
                ));
            }
        }
        _ => {
            decls.push(decl("position", "absolute"));
            decls.push(decl("left", pt(x)?));
            decls.push(decl("top", pt(y)?));
            decls.push(decl("width", pt(w)?));
            decls.push(decl("height", pt(h)?));
        }
    }
    if is_empty_box(frame, autosize) {
        decls.push(decl("box-shadow", EMPTY_BOX_EXTENT));
    }
    decls.extend(turn_decls(frame)?);
    Ok(decls)
}

/// The scale that gives a text body its page orientation inside an element the frame rotates and flips. Text that
/// flips with its frame keeps the element's flips; other text is unmirrored, the text box undoing them. The body's
/// half turn (which commutes with the flips) folds into the same scale.
pub fn text_turn_decl(frame: &Frame, body: &ResolvedTextBody) -> MaybeDecl {
    let (x, y) = text_turn(frame, body);
    flip_scale(x, y)
}

/// The per-axis signs of [`text_turn_decl`]'s scale.
pub fn text_turn(frame: &Frame, body: &ResolvedTextBody) -> (i8, i8) {
    let half = if body.rotation == TextRotation::Half {
        -1
    } else {
        1
    };
    let undo = |flipped: bool| sign(flipped && !body.flips_with_frame);
    (undo(frame.flip_h) * half, undo(frame.flip_v) * half)
}
