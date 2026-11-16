//! The colour modifiers' rules.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

use slidock_color::apply_color_mods;
use slidock_ir::color::{ColorMod, ColorModOp, ColorSpace, RgbColor};
use slidock_ir::units::{Ratio, UnitRatio};

/// A colour from `rrggbb` with its components quantised, as the frontends read `srgbClr`.
fn rgb_in(hex: &str, a: f64, space: ColorSpace) -> RgbColor {
    let channel = |at: usize| {
        let byte = u8::from_str_radix(&hex[at..at + 2], 16).unwrap();
        UnitRatio::quantise(f64::from(byte) / 255.0).unwrap()
    };
    RgbColor::new(
        space,
        channel(0),
        channel(2),
        channel(4),
        UnitRatio::quantise(a).unwrap(),
    )
}

fn rgb(hex: &str) -> RgbColor {
    rgb_in(hex, 1.0, ColorSpace::Srgb)
}

fn hex(color: &RgbColor) -> String {
    [color.r, color.g, color.b]
        .iter()
        .map(|c| format!("{:02x}", (c.to_f64() * 255.0).round() as u8))
        .collect()
}

fn mods(pairs: &[(ColorModOp, f64)]) -> Vec<ColorMod> {
    pairs
        .iter()
        .map(|&(op, value)| ColorMod {
            op,
            value: Ratio::quantise(value).unwrap(),
        })
        .collect()
}

fn apply(color: &RgbColor, pairs: &[(ColorModOp, f64)]) -> RgbColor {
    apply_color_mods(color, &mods(pairs)).unwrap()
}

use ColorModOp::*;

#[test]
fn matches_offices_named_theme_shades() {
    // "Blue, Accent 1, Darker 25%" and "Lighter 40%" of Office's 4472C4.
    assert_eq!(hex(&apply(&rgb("4472c4"), &[(LumMod, 0.75)])), "2f5597");
    assert_eq!(
        hex(&apply(&rgb("4472c4"), &[(LumMod, 0.6), (LumOff, 0.4)])),
        "8faadc"
    );
}

#[test]
fn blends_tint_and_shade_in_linear_light() {
    assert_eq!(hex(&apply(&rgb("000000"), &[(Tint, 0.5)])), "bcbcbc");
    assert_eq!(hex(&apply(&rgb("ffffff"), &[(Shade, 0.5)])), "bcbcbc");
    assert_eq!(
        apply(&rgb("ed7d31"), &[(Tint, 1.0)]),
        apply(&rgb("ed7d31"), &[])
    );
}

#[test]
fn sets_scales_and_offsets_alpha_within_0_1() {
    let white = |a: f64| rgb_in("ffffff", a, ColorSpace::Srgb);
    assert_eq!(apply(&white(0.8), &[(AlphaMod, 0.5)]).a.to_f64(), 0.4);
    assert_eq!(apply(&white(0.8), &[(AlphaOff, 0.5)]).a.to_f64(), 1.0);
    assert_eq!(apply(&white(1.0), &[(Alpha, 0.25)]).a.to_f64(), 0.25);
    assert_eq!(apply(&white(0.5), &[(AlphaOff, -0.75)]).a.to_f64(), 0.0);
}

#[test]
fn turns_hue_and_saturation_complements_inverts_and_greys() {
    assert_eq!(hex(&apply(&rgb("ff0000"), &[(HueOff, 120.0)])), "00ff00");
    assert_eq!(
        hex(&apply(&rgb("ff0000"), &[(HueMod, 2.0), (HueOff, 0.0)])),
        "ff0000"
    );
    assert_eq!(hex(&apply(&rgb("ff0000"), &[(Comp, 0.0)])), "00ffff");
    assert_eq!(hex(&apply(&rgb("ff8080"), &[(SatMod, 0.0)])), "bfbfbf");
    assert_eq!(hex(&apply(&rgb("bf8080"), &[(SatOff, 1.0)])), "ff4040");
    assert_eq!(hex(&apply(&rgb("336699"), &[(Inv, 0.0)])), "cc9966");
    assert_eq!(hex(&apply(&rgb("ff0000"), &[(Gray, 0.0)])), "363636");
}

#[test]
fn keeps_the_colour_space_and_quantises_onto_the_ratio_grid() {
    let out = apply(&rgb_in("4472c4", 1.0, ColorSpace::P3), &[(LumMod, 0.75)]);
    assert_eq!(out.space, ColorSpace::P3);
    for c in [out.r, out.g, out.b] {
        assert!(slidock_math::is_quantised(c.to_f64(), UnitRatio::GRID));
    }
}

#[test]
fn applies_modifiers_in_order() {
    // lumMod then lumOff lightens; the other order saturates at white first.
    let lighter = apply(&rgb("4472c4"), &[(LumMod, 0.6), (LumOff, 0.4)]);
    let other = apply(&rgb("4472c4"), &[(LumOff, 0.4), (LumMod, 0.6)]);
    assert_ne!(lighter, other);
}

#[test]
fn clamps_hsl_results_and_wraps_hue() {
    // A lightness pushed past 1 is white, a hue turned past a whole turn wraps.
    assert_eq!(hex(&apply(&rgb("4472c4"), &[(LumOff, 2.0)])), "ffffff");
    assert_eq!(hex(&apply(&rgb("ff0000"), &[(HueOff, -240.0)])), "00ff00");
}

#[test]
fn a_component_outside_the_grid_range_that_ends_as_nan_is_an_error() {
    // r = 1.5, b = 0.5: lightness 1, so the saturation divides by zero, and satMod 0 turns its infinity into NaN.
    let wide = RgbColor::new(
        ColorSpace::Srgb,
        UnitRatio::from_steps(150_000),
        UnitRatio::from_steps(100_000),
        UnitRatio::from_steps(50_000),
        UnitRatio::from_steps(100_000),
    );
    let error = apply_color_mods(&wide, &mods(&[(SatMod, 0.0)])).unwrap_err();
    assert_eq!(error.component, 'r');
    assert!(error.value.is_nan());
}
