//! The colour spaces the modifiers work in: HSL over a colour's own components, and linear light through the sRGB
//! transfer function (IEC 61966-2-1). Every operation is IEEE arithmetic or fdlibm, so results are the same bits
//! on every target.

use slidock_math::{exp, log};

/// Colour components as numbers, before quantisation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

/// Hue in degrees, saturation and lightness as fractions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsl {
    pub h: f64,
    pub s: f64,
    pub l: f64,
}

/// `x` within [0, 1]; NaN stays NaN.
pub fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}

/// `x^y` for `x > 0` as `exp(y·log x)`, and 0 otherwise.
pub fn pow(x: f64, y: f64) -> f64 {
    if x <= 0.0 { 0.0 } else { exp(y * log(x)) }
}

/// An sRGB component in linear light.
pub fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        pow((c + 0.055) / 1.055, 2.4)
    }
}

/// A linear-light component in sRGB.
pub fn srgb_from_linear(c: f64) -> f64 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * pow(c, 1.0 / 2.4) - 0.055
    }
}

/// A hue in [0, 360).
pub fn normal_hue(h: f64) -> f64 {
    ((h % 360.0) + 360.0) % 360.0
}

impl Rgb {
    pub fn to_hsl(self) -> Hsl {
        let Rgb { r, g, b } = self;
        let max = if r > g {
            if r > b { r } else { b }
        } else if g > b {
            g
        } else {
            b
        };
        let min = if r < g {
            if r < b { r } else { b }
        } else if g < b {
            g
        } else {
            b
        };
        let l = (max + min) / 2.0;
        let d = max - min;
        if d == 0.0 {
            return Hsl { h: 0.0, s: 0.0, l };
        }
        let s = if l > 0.5 {
            d / (2.0 - max - min)
        } else {
            d / (max + min)
        };
        let h = if max == r {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        Hsl { h: h * 60.0, s, l }
    }
}

fn hue_channel(p: f64, q: f64, t: f64) -> f64 {
    let u = if t < 0.0 {
        t + 1.0
    } else if t > 1.0 {
        t - 1.0
    } else {
        t
    };
    if u < 1.0 / 6.0 {
        p + (q - p) * 6.0 * u
    } else if u < 1.0 / 2.0 {
        q
    } else if u < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - u) * 6.0
    } else {
        p
    }
}

impl Hsl {
    /// The colour of a hue in [0, 360); saturation and lightness are taken as they are.
    pub fn to_rgb(self) -> Rgb {
        let Hsl { h, s, l } = self;
        if s == 0.0 {
            return Rgb { r: l, g: l, b: l };
        }
        let q = if l < 0.5 {
            l * (1.0 + s)
        } else {
            l + s - l * s
        };
        let p = 2.0 * l - q;
        let t = h / 360.0;
        Rgb {
            r: hue_channel(p, q, t + 1.0 / 3.0),
            g: hue_channel(p, q, t),
            b: hue_channel(p, q, t - 1.0 / 3.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsl_round_trips_the_primaries_and_greys() {
        let red = Rgb {
            r: 1.0,
            g: 0.0,
            b: 0.0,
        };
        assert_eq!(
            red.to_hsl(),
            Hsl {
                h: 0.0,
                s: 1.0,
                l: 0.5
            }
        );
        assert_eq!(red.to_hsl().to_rgb(), red);
        let grey = Rgb {
            r: 0.25,
            g: 0.25,
            b: 0.25,
        };
        assert_eq!(grey.to_hsl().s, 0.0);
        assert_eq!(grey.to_hsl().to_rgb(), grey);
        // Blue is the last branch: (r − g) / d + 4.
        let blue = Rgb {
            r: 0.0,
            g: 0.0,
            b: 1.0,
        };
        assert_eq!(blue.to_hsl().h, 240.0);
        // Magenta takes the +6 wrap of a hue below red.
        let magenta = Rgb {
            r: 1.0,
            g: 0.0,
            b: 0.5,
        };
        assert_eq!(magenta.to_hsl().h, 330.0);
    }

    #[test]
    fn normal_hue_wraps_both_ways() {
        assert_eq!(normal_hue(370.0), 10.0);
        assert_eq!(normal_hue(-30.0), 330.0);
        assert_eq!(normal_hue(-720.0), 0.0);
    }

    #[test]
    fn transfer_functions_meet_at_their_knees() {
        assert_eq!(srgb_to_linear(0.04045), 0.04045 / 12.92);
        assert_eq!(srgb_from_linear(0.003_130_8), 0.003_130_8 * 12.92);
        assert_eq!(srgb_to_linear(1.0), 1.0);
        assert_eq!(pow(0.0, 2.4), 0.0);
        assert_eq!(pow(-1.0, 2.0), 0.0);
        assert_eq!(clamp01(-0.5), 0.0);
        assert_eq!(clamp01(1.5), 1.0);
        assert!(clamp01(f64::NAN).is_nan());
    }
}
