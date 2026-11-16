//! Display P3 and sRGB, and the blending of gradient stops. Both spaces share the sRGB transfer function; between
//! them the linear components go through CIE XYZ (D65) with CSS Color 4's matrices. A gradient blends in Display P3
//! when any of its stops is P3, in sRGB otherwise, colour and alpha separately (unpremultiplied).

use slidock_ir::color::{ColorSpace, RgbColor};

use crate::model::{srgb_from_linear, srgb_to_linear};

type Matrix = [[f64; 3]; 3];

const SRGB_TO_XYZ: Matrix = [
    [
        506_752.0 / 1_228_815.0,
        87_881.0 / 245_763.0,
        12_673.0 / 70_218.0,
    ],
    [
        87_098.0 / 409_605.0,
        175_762.0 / 245_763.0,
        12_673.0 / 175_545.0,
    ],
    [
        7_918.0 / 409_605.0,
        87_881.0 / 737_289.0,
        1_001_167.0 / 1_053_270.0,
    ],
];

const XYZ_TO_SRGB: Matrix = [
    [12_831.0 / 3_959.0, -329.0 / 214.0, -1_974.0 / 3_959.0],
    [
        -851_781.0 / 878_810.0,
        1_648_619.0 / 878_810.0,
        36_519.0 / 878_810.0,
    ],
    [705.0 / 12_673.0, -2_585.0 / 12_673.0, 705.0 / 667.0],
];

const P3_TO_XYZ: Matrix = [
    [
        608_311.0 / 1_250_200.0,
        189_793.0 / 714_400.0,
        198_249.0 / 1_000_160.0,
    ],
    [
        35_783.0 / 156_275.0,
        247_089.0 / 357_200.0,
        198_249.0 / 2_500_400.0,
    ],
    [0.0, 32_229.0 / 714_400.0, 5_220_557.0 / 5_000_800.0],
];

const XYZ_TO_P3: Matrix = [
    [
        446_124.0 / 178_915.0,
        -333_277.0 / 357_830.0,
        -72_051.0 / 178_915.0,
    ],
    [-14_852.0 / 17_905.0, 63_121.0 / 35_810.0, 423.0 / 17_905.0],
    [
        11_844.0 / 330_415.0,
        -50_337.0 / 660_830.0,
        316_169.0 / 330_415.0,
    ],
];

fn apply(m: &Matrix, v: [f64; 3]) -> [f64; 3] {
    m.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2])
}

/// The sRGB transfer function extended to negative components, by symmetry.
fn decode(c: f64) -> f64 {
    if c < 0.0 {
        -srgb_to_linear(-c)
    } else {
        srgb_to_linear(c)
    }
}

fn encode(c: f64) -> f64 {
    if c < 0.0 {
        -srgb_from_linear(-c)
    } else {
        srgb_from_linear(c)
    }
}

fn convert(rgb: [f64; 3], to_xyz: &Matrix, from_xyz: &Matrix) -> [f64; 3] {
    apply(from_xyz, apply(to_xyz, rgb.map(decode))).map(encode)
}

/// Encoded Display P3 components as encoded sRGB, unclamped: outside [0, 1] when outside the sRGB gamut.
pub fn p3_to_srgb(rgb: [f64; 3]) -> [f64; 3] {
    convert(rgb, &P3_TO_XYZ, &XYZ_TO_SRGB)
}

/// Encoded sRGB components as encoded Display P3.
pub fn srgb_to_p3(rgb: [f64; 3]) -> [f64; 3] {
    convert(rgb, &SRGB_TO_XYZ, &XYZ_TO_P3)
}

/// The space a gradient over these colours blends in: Display P3 when any is P3.
pub fn blending_space<'a>(colors: impl IntoIterator<Item = &'a RgbColor>) -> ColorSpace {
    if colors
        .into_iter()
        .any(|color| color.space == ColorSpace::P3)
    {
        ColorSpace::P3
    } else {
        ColorSpace::Srgb
    }
}

/// A colour's encoded components and alpha in `space`.
pub fn components_in(color: &RgbColor, space: ColorSpace) -> [f64; 4] {
    let rgb = [color.r.to_f64(), color.g.to_f64(), color.b.to_f64()];
    let [r, g, b] = match (color.space, space) {
        (ColorSpace::Srgb, ColorSpace::P3) => srgb_to_p3(rgb),
        (ColorSpace::P3, ColorSpace::Srgb) => p3_to_srgb(rgb),
        _ => rgb,
    };
    [r, g, b, color.a.to_f64()]
}

/// The colour `t` of the way from `a` to `b`, each component and alpha linear (unpremultiplied).
pub fn lerp(a: [f64; 4], b: [f64; 4], t: f64) -> [f64; 4] {
    let [ar, ag, ab, aa] = a;
    let [br, bg, bb, ba] = b;
    [
        ar + (br - ar) * t,
        ag + (bg - ag) * t,
        ab + (bb - ab) * t,
        aa + (ba - aa) * t,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_between_display_p3_and_srgb_with_css_colour_4s_matrices() {
        // P3 red is outside sRGB: (1.0930, −0.2267, −0.1501) (CSS Color 4 sample).
        let red = p3_to_srgb([1.0, 0.0, 0.0]);
        assert!((red[0] - 1.093_1).abs() < 5e-4, "{red:?}");
        assert!((red[1] + 0.226_7).abs() < 5e-4, "{red:?}");
        assert!((red[2] + 0.150_1).abs() < 5e-4, "{red:?}");
        // White and grey are the same in both; a round trip returns the colour.
        for c in [[1.0, 1.0, 1.0], [0.5, 0.5, 0.5]] {
            for (x, y) in p3_to_srgb(c).iter().zip(c) {
                assert!((x - y).abs() < 1e-6);
            }
        }
        let c = [0.0823, 0.2228, 0.5373];
        for (x, y) in srgb_to_p3(p3_to_srgb(c)).iter().zip(c) {
            assert!((x - y).abs() < 1e-9);
        }
        // The retro deck's P3 stop in sRGB: (0.0004, 0.2267, 0.5578).
        let s = p3_to_srgb(c);
        assert!(
            (s[1] - 0.226_7).abs() < 5e-4 && (s[2] - 0.557_8).abs() < 5e-4,
            "{s:?}"
        );
    }

    #[test]
    fn blends_in_p3_when_any_stop_is_p3() {
        let srgb = RgbColor::new(
            ColorSpace::Srgb,
            slidock_ir::units::UnitRatio::ZERO,
            slidock_ir::units::UnitRatio::ZERO,
            slidock_ir::units::UnitRatio::ZERO,
            slidock_ir::units::UnitRatio::from_steps(100_000),
        );
        let mut p3 = srgb.clone();
        p3.space = ColorSpace::P3;
        assert_eq!(blending_space([&srgb, &srgb]), ColorSpace::Srgb);
        assert_eq!(blending_space([&srgb, &p3]), ColorSpace::P3);
        assert_eq!(
            lerp([0.0, 0.0, 0.0, 0.25], [1.0, 0.5, 0.0, 1.0], 0.5),
            [0.5, 0.25, 0.0, 0.625]
        );
    }
}
