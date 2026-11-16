//! Shadows as the carrier's own CSS shadow, one declaration per carrier and always the last of its style, so a document
//! without shadows prints as it would without this module.
//!
//! - `filter: drop-shadow(x y σ colour)` shadows the drawn alpha of everything the element paints (geometry layers,
//!   strokes, line ends, glyphs, a picture after its crop, clip and mask, overflowing text). Filter Effects 1 takes
//!   its third length as the Gaussian's standard deviation. Engines draw its offset at whole CSS px of the element's
//!   local axes (Chromium floors, WebKit truncates toward zero; measured at device scales 1, 2 and 4), so it is
//!   printed at the nearest whole px (0.75 pt), which both draw as printed.
//! - `box-shadow` and `text-shadow` take a blur length of twice the standard deviation (CSS Backgrounds 3 §7.1): a
//!   plain opaque rectangle, a video's poster rectangle, a text run's glyphs.
//!
//! The offset lives in the carrier's local axes, under every rotation and flip printed above it ([`Axes`]): the page
//! offset is taken into them, or, for a shadow that turns with its shape, turned first.

use slidock_ir::shadow::{Axes, ResolvedShadow};

use crate::color::css_color;
use crate::css::{MaybeDecl, pt};
use crate::error::Result;

/// The CSS forms of a shadow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowForm {
    /// `filter: drop-shadow(…)`: the alpha of everything the element paints.
    Drop,
    /// `box-shadow`: the element's border box, which must be opaque.
    Box,
    /// `text-shadow`: the glyphs of the text the element holds.
    Text,
}

/// One CSS px in points.
const PX: f64 = 0.75;

/// `x` at the nearest whole CSS px.
fn on_px_grid(x: f64) -> f64 {
    (x / PX).round() * PX + 0.0
}

/// `x y blur colour`, the blur length as the form takes it.
fn shadow_value(shadow: &ResolvedShadow, axes: &Axes, form: ShadowForm) -> Result<String> {
    let (x, y) = shadow.offset_in(axes);
    let (x, y, blur_per_sigma) = match form {
        ShadowForm::Drop => (on_px_grid(x), on_px_grid(y), 1.0),
        ShadowForm::Box | ShadowForm::Text => (x, y, 2.0),
    };
    Ok(format!(
        "{} {} {} {}",
        pt(x)?,
        pt(y)?,
        pt(shadow.sigma.to_f64() * blur_per_sigma)?,
        css_color(&shadow.color)?
    ))
}

/// The declaration that draws `shadow` in `form`, `axes` being the carrier's local axes.
pub fn shadow_decl(
    shadow: Option<&ResolvedShadow>,
    axes: &Axes,
    form: ShadowForm,
) -> Result<MaybeDecl> {
    let Some(shadow) = shadow else {
        return Ok(None);
    };
    let value = shadow_value(shadow, axes, form)?;
    Ok(Some(match form {
        ShadowForm::Drop => ("filter", format!("drop-shadow({value})")),
        ShadowForm::Box => ("box-shadow", value),
        ShadowForm::Text => ("text-shadow", value),
    }))
}
