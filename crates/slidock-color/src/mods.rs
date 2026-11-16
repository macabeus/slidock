//! DrawingML's colour modifiers. Luminance, saturation and hue work in HSL over the colour's own components; tint and
//! shade blend in linear light, as Office and LibreOffice do; the result is clamped and quantised to the ratio grid,
//! and the colour space is kept.

use slidock_ir::color::{ColorMod, ColorModOp, RgbColor};
use slidock_ir::units::UnitRatio;

use crate::model::{Hsl, Rgb, clamp01, normal_hue, srgb_from_linear, srgb_to_linear};

/// A component the modifiers left without a value on the ratio grid.
#[derive(Clone, Copy, Debug, PartialEq, thiserror::Error)]
#[error("colour component {component} is {value}, not a number in [0, 1]")]
pub struct ColorError {
    pub component: char,
    pub value: f64,
}

/// A colour with alpha, as numbers, while modifiers apply.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Rgba {
    fn rgb(self) -> Rgb {
        Rgb {
            r: self.r,
            g: self.g,
            b: self.b,
        }
    }

    fn with_rgb(self, Rgb { r, g, b }: Rgb) -> Rgba {
        Rgba { r, g, b, a: self.a }
    }

    fn with_alpha(self, a: f64) -> Rgba {
        Rgba { a, ..self }
    }
}

impl From<&RgbColor> for Rgba {
    fn from(color: &RgbColor) -> Self {
        Rgba {
            r: color.r.to_f64(),
            g: color.g.to_f64(),
            b: color.b.to_f64(),
            a: color.a.to_f64(),
        }
    }
}

fn in_hsl(rgb: Rgb, change: impl FnOnce(Hsl) -> Hsl) -> Rgb {
    let next = change(rgb.to_hsl());
    Hsl {
        h: normal_hue(next.h),
        s: clamp01(next.s),
        l: clamp01(next.l),
    }
    .to_rgb()
}

fn in_linear(rgb: Rgb, change: impl Fn(f64) -> f64) -> Rgb {
    let channel = |c: f64| srgb_from_linear(clamp01(change(srgb_to_linear(c))));
    Rgb {
        r: channel(rgb.r),
        g: channel(rgb.g),
        b: channel(rgb.b),
    }
}

/// One modifier applied to a colour. `value` is a fraction (1 = 100%), or degrees for `hueOff`; `inv`, `gray` and
/// `comp` ignore it.
pub fn apply_mod(color: Rgba, op: ColorModOp, value: f64) -> Rgba {
    let rgb = color.rgb();
    match op {
        ColorModOp::Alpha => color.with_alpha(clamp01(value)),
        ColorModOp::AlphaMod => color.with_alpha(clamp01(color.a * value)),
        ColorModOp::AlphaOff => color.with_alpha(clamp01(color.a + value)),
        ColorModOp::LumMod => color.with_rgb(in_hsl(rgb, |hsl| Hsl {
            l: hsl.l * value,
            ..hsl
        })),
        ColorModOp::LumOff => color.with_rgb(in_hsl(rgb, |hsl| Hsl {
            l: hsl.l + value,
            ..hsl
        })),
        ColorModOp::SatMod => color.with_rgb(in_hsl(rgb, |hsl| Hsl {
            s: hsl.s * value,
            ..hsl
        })),
        ColorModOp::SatOff => color.with_rgb(in_hsl(rgb, |hsl| Hsl {
            s: hsl.s + value,
            ..hsl
        })),
        ColorModOp::HueMod => color.with_rgb(in_hsl(rgb, |hsl| Hsl {
            h: hsl.h * value,
            ..hsl
        })),
        ColorModOp::HueOff => color.with_rgb(in_hsl(rgb, |hsl| Hsl {
            h: hsl.h + value,
            ..hsl
        })),
        ColorModOp::Comp => color.with_rgb(in_hsl(rgb, |hsl| Hsl {
            h: hsl.h + 180.0,
            ..hsl
        })),
        // A 10% tint keeps 10% of the colour and adds 90% white.
        ColorModOp::Tint => color.with_rgb(in_linear(rgb, |c| c * value + (1.0 - value))),
        ColorModOp::Shade => color.with_rgb(in_linear(rgb, |c| c * value)),
        ColorModOp::Inv => color.with_rgb(Rgb {
            r: 1.0 - rgb.r,
            g: 1.0 - rgb.g,
            b: 1.0 - rgb.b,
        }),
        ColorModOp::Gray => {
            let y = 0.2126 * rgb.r + 0.7152 * rgb.g + 0.0722 * rgb.b;
            color.with_rgb(Rgb { r: y, g: y, b: y })
        }
    }
}

fn component(name: char, x: f64) -> Result<UnitRatio, ColorError> {
    UnitRatio::quantise(clamp01(x)).map_err(|_| ColorError {
        component: name,
        value: x,
    })
}

/// A colour with its modifiers applied in order, clamped and quantised; the colour space is kept. Only components
/// outside [0, 1] can leave a NaN behind (an infinite saturation scaled by 0), which is an error.
pub fn apply_color_mods(color: &RgbColor, mods: &[ColorMod]) -> Result<RgbColor, ColorError> {
    let out = mods.iter().fold(Rgba::from(color), |acc, m| {
        apply_mod(acc, m.op, m.value.to_f64())
    });
    Ok(RgbColor::new(
        color.space,
        component('r', out.r)?,
        component('g', out.g)?,
        component('b', out.b)?,
        component('a', out.a)?,
    ))
}
