//! DrawingML colour (ECMA-376 Part 1, §20.1.2.3): the colour modifiers (tint, shade, lumMod, satMod, alpha, …)
//! as pure maths, and the HSL and sRGB transfer conversions they work in. The frontends apply the modifiers to
//! literal colours at their boundary, and resolve-theme applies them to theme references. Display P3 and sRGB
//! convert through CIE XYZ for gradients, which blend in P3 when any stop is P3.

mod gamut;
mod model;
mod mods;

pub use gamut::{blending_space, components_in, lerp, p3_to_srgb, srgb_to_p3};
pub use model::{Hsl, Rgb, clamp01, normal_hue, pow, srgb_from_linear, srgb_to_linear};
pub use mods::{ColorError, Rgba, apply_color_mods, apply_mod};
