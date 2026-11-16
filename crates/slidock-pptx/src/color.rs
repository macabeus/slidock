//! DrawingML colours (ECMA-376 Part 1, §20.1.2.3): a colour choice with its modifiers. Literal colours (sRGB, scRGB,
//! HSL, system, preset) arrive with their modifiers applied; scheme references stay symbolic as theme colours, so
//! re-theming still works. `phClr` is the placeholder a theme's format scheme fills in from a style reference.

use slidock_color::{Hsl, Rgb, apply_color_mods, normal_hue};
use slidock_ir::color::{
    Color, ColorMod, ColorModOp, ColorSpace, MappedSlot, RgbColor, ThemeColor, ThemeTag,
};
use slidock_math::{exp, log};

use crate::report::Report;
use crate::units::{ratio, unit_ratio};
use crate::xml::{El, Element, Ns};

/// A colour as written: a literal or theme colour, or the format scheme's placeholder with its own modifiers.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SourceColor {
    Color(Color),
    PhClr(Vec<ColorMod>),
}

const COLOR_ELEMENTS: [&str; 6] = [
    "srgbClr",
    "schemeClr",
    "sysClr",
    "scrgbClr",
    "hslClr",
    "prstClr",
];

fn component(x: f64) -> slidock_ir::units::UnitRatio {
    unit_ratio(x.clamp(0.0, 1.0))
}

pub(crate) fn rgb(r: f64, g: f64, b: f64, a: f64) -> RgbColor {
    RgbColor::new(
        ColorSpace::Srgb,
        component(r),
        component(g),
        component(b),
        component(a),
    )
}

/// Six hex digits as an opaque sRGB colour.
pub(crate) fn hex_color(hex: Option<&str>) -> Option<RgbColor> {
    let hex = hex?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |at: usize| {
        hex.get(at..at + 2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            .map_or(0.0, |value| f64::from(value) / 255.0)
    };
    Some(rgb(channel(0), channel(2), channel(4), 1.0))
}

fn black() -> RgbColor {
    rgb(0.0, 0.0, 0.0, 1.0)
}

pub(crate) fn mapped_slot(name: &str) -> Option<MappedSlot> {
    Some(match name {
        "dk1" => MappedSlot::Dk1,
        "lt1" => MappedSlot::Lt1,
        "dk2" => MappedSlot::Dk2,
        "lt2" => MappedSlot::Lt2,
        "accent1" => MappedSlot::Accent1,
        "accent2" => MappedSlot::Accent2,
        "accent3" => MappedSlot::Accent3,
        "accent4" => MappedSlot::Accent4,
        "accent5" => MappedSlot::Accent5,
        "accent6" => MappedSlot::Accent6,
        "hlink" => MappedSlot::Hlink,
        "folHlink" => MappedSlot::FolHlink,
        "bg1" => MappedSlot::Bg1,
        "tx1" => MappedSlot::Tx1,
        "bg2" => MappedSlot::Bg2,
        "tx2" => MappedSlot::Tx2,
        _ => return None,
    })
}

pub(crate) fn theme_color(slot: MappedSlot, mods: Vec<ColorMod>) -> Color {
    Color::Theme(ThemeColor {
        kind: ThemeTag,
        slot,
        mods,
        resolved: None,
    })
}

fn mod_op(name: &str) -> Option<ColorModOp> {
    Some(match name {
        "lumMod" => ColorModOp::LumMod,
        "lumOff" => ColorModOp::LumOff,
        "tint" => ColorModOp::Tint,
        "shade" => ColorModOp::Shade,
        "alpha" => ColorModOp::Alpha,
        "alphaMod" => ColorModOp::AlphaMod,
        "alphaOff" => ColorModOp::AlphaOff,
        "satMod" => ColorModOp::SatMod,
        "satOff" => ColorModOp::SatOff,
        "hueMod" => ColorModOp::HueMod,
        "hueOff" => ColorModOp::HueOff,
        "inv" => ColorModOp::Inv,
        "gray" => ColorModOp::Gray,
        "comp" => ColorModOp::Comp,
        _ => return None,
    })
}

/// Modifier values are thousandths of a percent, except `hueOff`'s angle and the valueless `inv`, `gray`, `comp`.
fn read_mods(element: &Element, report: &mut dyn Report) -> Vec<ColorMod> {
    let mut mods = Vec::new();
    for modifier in element.kids() {
        if modifier.ns != Ns::A {
            continue;
        }
        let Some(op) = mod_op(&modifier.local) else {
            report.dropped(&modifier.local);
            continue;
        };
        let raw = modifier.int_attr("val").unwrap_or(0) as f64;
        let value = match op {
            ColorModOp::Inv | ColorModOp::Gray | ColorModOp::Comp => 0.0,
            ColorModOp::HueOff => raw / 60_000.0,
            _ => raw / 100_000.0,
        };
        mods.push(ColorMod {
            op,
            value: ratio(value),
        });
    }
    mods
}

/// scRGB components are linear light; the IR holds sRGB.
fn encode_srgb(c: f64) -> f64 {
    let x = c.clamp(0.0, 1.0);
    if x <= 0.003_130_8 {
        x * 12.92
    } else {
        1.055 * exp(log(x) / 2.4) - 0.055
    }
}

fn hsl(hue: f64, saturation: f64, lightness: f64) -> RgbColor {
    let Rgb { r, g, b } = Hsl {
        h: normal_hue(hue),
        s: saturation,
        l: lightness,
    }
    .to_rgb();
    rgb(r, g, b, 1.0)
}

/// Windows system colours, for a `sysClr` that omits `lastClr`; any other name reads as black.
fn system_color(name: &str) -> Option<&'static str> {
    match name {
        "windowText" => Some("000000"),
        "window" => Some("FFFFFF"),
        "btnFace" => Some("F0F0F0"),
        _ => None,
    }
}

fn literal(element: &Element) -> RgbColor {
    let attr = |name: &str| element.attr(name);
    match &*element.local {
        "srgbClr" => hex_color(attr("val")).unwrap_or_else(black),
        "sysClr" => hex_color(attr("lastClr"))
            .or_else(|| hex_color(system_color(attr("val").unwrap_or(""))))
            .unwrap_or_else(black),
        "scrgbClr" => {
            let component =
                |name: &str| encode_srgb(element.int_attr(name).unwrap_or(0) as f64 / 100_000.0);
            rgb(component("r"), component("g"), component("b"), 1.0)
        }
        "hslClr" => hsl(
            element.int_attr("hue").unwrap_or(0) as f64 / 60_000.0,
            element.int_attr("sat").unwrap_or(0) as f64 / 100_000.0,
            element.int_attr("lum").unwrap_or(0) as f64 / 100_000.0,
        ),
        _ => hex_color(preset_color(&preset_key(attr("val").unwrap_or("")))).unwrap_or_else(black),
    }
}

/// The colour choice among an element's children (`a:solidFill`, `a:buClr`, a gradient stop, …).
pub(crate) fn color_element<'a>(parent: impl El<'a>) -> Option<&'a Element> {
    parent
        .kids()
        .into_iter()
        .find(|kid| kid.ns == Ns::A && COLOR_ELEMENTS.contains(&&*kid.local))
}

/// A literal with its modifiers applied.
fn modified(color: &RgbColor, mods: &[ColorMod]) -> RgbColor {
    if mods.is_empty() {
        return color.clone();
    }
    // Only a NaN component fails, which finite modifier values cannot produce.
    apply_color_mods(color, mods).unwrap_or_else(|_| color.clone())
}

/// One colour element as written.
pub(crate) fn read_color(element: &Element, report: &mut dyn Report) -> SourceColor {
    let mods = read_mods(element, report);
    if &*element.local == "schemeClr" {
        let slot = element.attr("val").unwrap_or("");
        if slot == "phClr" {
            return SourceColor::PhClr(mods);
        }
        return SourceColor::Color(theme_color(
            mapped_slot(slot).unwrap_or(MappedSlot::Tx1),
            mods,
        ));
    }
    SourceColor::Color(Color::Rgb(modified(&literal(element), &mods)))
}

/// The colour of `phClr` filled in by a style reference's colour: its slot, its modifiers, then the entry's.
pub(crate) fn fill_placeholder(color: SourceColor, by: Option<&Color>) -> Color {
    let mods = match color {
        SourceColor::Color(color) => return color,
        SourceColor::PhClr(mods) => mods,
    };
    match by {
        Some(Color::Rgb(base)) => Color::Rgb(modified(base, &mods)),
        Some(Color::Theme(base)) => {
            let mut all = base.mods.clone();
            all.extend(mods);
            theme_color(base.slot, all)
        }
        None => theme_color(MappedSlot::Tx1, mods),
    }
}

/// A colour outside a format scheme: `phClr` has no style colour to stand for, so it reads as the text colour.
pub(crate) fn plain_color(color: SourceColor) -> Color {
    fill_placeholder(color, None)
}

/// ST_PresetColorVal by CSS name (`dkBlue` is `darkblue`).
fn preset_key(name: &str) -> String {
    let name = if let Some(rest) = name.strip_prefix("dk") {
        format!("dark{rest}")
    } else if let Some(rest) = name.strip_prefix("lt") {
        format!("light{rest}")
    } else if let Some(rest) = name.strip_prefix("med") {
        format!("medium{rest}")
    } else {
        name.to_owned()
    };
    name.to_lowercase()
}

fn preset_color(key: &str) -> Option<&'static str> {
    PRESET_COLORS
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, hex)| *hex)
}

/// The standard's preset colours, which are the X11 colours.
const PRESET_COLORS: [(&str, &str); 147] = [
    ("aliceblue", "F0F8FF"),
    ("antiquewhite", "FAEBD7"),
    ("aqua", "00FFFF"),
    ("aquamarine", "7FFFD4"),
    ("azure", "F0FFFF"),
    ("beige", "F5F5DC"),
    ("bisque", "FFE4C4"),
    ("black", "000000"),
    ("blanchedalmond", "FFEBCD"),
    ("blue", "0000FF"),
    ("blueviolet", "8A2BE2"),
    ("brown", "A52A2A"),
    ("burlywood", "DEB887"),
    ("cadetblue", "5F9EA0"),
    ("chartreuse", "7FFF00"),
    ("chocolate", "D2691E"),
    ("coral", "FF7F50"),
    ("cornflowerblue", "6495ED"),
    ("cornsilk", "FFF8DC"),
    ("crimson", "DC143C"),
    ("cyan", "00FFFF"),
    ("darkblue", "00008B"),
    ("darkcyan", "008B8B"),
    ("darkgoldenrod", "B8860B"),
    ("darkgray", "A9A9A9"),
    ("darkgrey", "A9A9A9"),
    ("darkgreen", "006400"),
    ("darkkhaki", "BDB76B"),
    ("darkmagenta", "8B008B"),
    ("darkolivegreen", "556B2F"),
    ("darkorange", "FF8C00"),
    ("darkorchid", "9932CC"),
    ("darkred", "8B0000"),
    ("darksalmon", "E9967A"),
    ("darkseagreen", "8FBC8F"),
    ("darkslateblue", "483D8B"),
    ("darkslategray", "2F4F4F"),
    ("darkslategrey", "2F4F4F"),
    ("darkturquoise", "00CED1"),
    ("darkviolet", "9400D3"),
    ("deeppink", "FF1493"),
    ("deepskyblue", "00BFFF"),
    ("dimgray", "696969"),
    ("dimgrey", "696969"),
    ("dodgerblue", "1E90FF"),
    ("firebrick", "B22222"),
    ("floralwhite", "FFFAF0"),
    ("forestgreen", "228B22"),
    ("fuchsia", "FF00FF"),
    ("gainsboro", "DCDCDC"),
    ("ghostwhite", "F8F8FF"),
    ("gold", "FFD700"),
    ("goldenrod", "DAA520"),
    ("gray", "808080"),
    ("grey", "808080"),
    ("green", "008000"),
    ("greenyellow", "ADFF2F"),
    ("honeydew", "F0FFF0"),
    ("hotpink", "FF69B4"),
    ("indianred", "CD5C5C"),
    ("indigo", "4B0082"),
    ("ivory", "FFFFF0"),
    ("khaki", "F0E68C"),
    ("lavender", "E6E6FA"),
    ("lavenderblush", "FFF0F5"),
    ("lawngreen", "7CFC00"),
    ("lemonchiffon", "FFFACD"),
    ("lightblue", "ADD8E6"),
    ("lightcoral", "F08080"),
    ("lightcyan", "E0FFFF"),
    ("lightgoldenrodyellow", "FAFAD2"),
    ("lightgray", "D3D3D3"),
    ("lightgrey", "D3D3D3"),
    ("lightgreen", "90EE90"),
    ("lightpink", "FFB6C1"),
    ("lightsalmon", "FFA07A"),
    ("lightseagreen", "20B2AA"),
    ("lightskyblue", "87CEFA"),
    ("lightslategray", "778899"),
    ("lightslategrey", "778899"),
    ("lightsteelblue", "B0C4DE"),
    ("lightyellow", "FFFFE0"),
    ("lime", "00FF00"),
    ("limegreen", "32CD32"),
    ("linen", "FAF0E6"),
    ("magenta", "FF00FF"),
    ("maroon", "800000"),
    ("mediumaquamarine", "66CDAA"),
    ("mediumblue", "0000CD"),
    ("mediumorchid", "BA55D3"),
    ("mediumpurple", "9370DB"),
    ("mediumseagreen", "3CB371"),
    ("mediumslateblue", "7B68EE"),
    ("mediumspringgreen", "00FA9A"),
    ("mediumturquoise", "48D1CC"),
    ("mediumvioletred", "C71585"),
    ("midnightblue", "191970"),
    ("mintcream", "F5FFFA"),
    ("mistyrose", "FFE4E1"),
    ("moccasin", "FFE4B5"),
    ("navajowhite", "FFDEAD"),
    ("navy", "000080"),
    ("oldlace", "FDF5E6"),
    ("olive", "808000"),
    ("olivedrab", "6B8E23"),
    ("orange", "FFA500"),
    ("orangered", "FF4500"),
    ("orchid", "DA70D6"),
    ("palegoldenrod", "EEE8AA"),
    ("palegreen", "98FB98"),
    ("paleturquoise", "AFEEEE"),
    ("palevioletred", "DB7093"),
    ("papayawhip", "FFEFD5"),
    ("peachpuff", "FFDAB9"),
    ("peru", "CD853F"),
    ("pink", "FFC0CB"),
    ("plum", "DDA0DD"),
    ("powderblue", "B0E0E6"),
    ("purple", "800080"),
    ("red", "FF0000"),
    ("rosybrown", "BC8F8F"),
    ("royalblue", "4169E1"),
    ("saddlebrown", "8B4513"),
    ("salmon", "FA8072"),
    ("sandybrown", "F4A460"),
    ("seagreen", "2E8B57"),
    ("seashell", "FFF5EE"),
    ("sienna", "A0522D"),
    ("silver", "C0C0C0"),
    ("skyblue", "87CEEB"),
    ("slateblue", "6A5ACD"),
    ("slategray", "708090"),
    ("slategrey", "708090"),
    ("snow", "FFFAFA"),
    ("springgreen", "00FF7F"),
    ("steelblue", "4682B4"),
    ("tan", "D2B48C"),
    ("teal", "008080"),
    ("thistle", "D8BFD8"),
    ("tomato", "FF6347"),
    ("turquoise", "40E0D0"),
    ("violet", "EE82EE"),
    ("wheat", "F5DEB3"),
    ("white", "FFFFFF"),
    ("whitesmoke", "F5F5F5"),
    ("yellow", "FFFF00"),
    ("yellowgreen", "9ACD32"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_preset_names_by_their_css_kin() {
        assert_eq!(preset_key("dkBlue"), "darkblue");
        assert_eq!(preset_key("ltGoldenrodYellow"), "lightgoldenrodyellow");
        assert_eq!(preset_key("medSeaGreen"), "mediumseagreen");
        assert_eq!(preset_color("darkblue"), Some("00008B"));
        assert_eq!(preset_color("nope"), None);
        assert_eq!(hex_color(Some("FF000")), None);
        assert_eq!(hex_color(Some("+F0000")), None);
    }

    #[test]
    fn converts_hsl_and_linear_light() {
        let red = hsl(0.0, 1.0, 0.5);
        assert_eq!(
            (red.r.to_f64(), red.g.to_f64(), red.b.to_f64()),
            (1.0, 0.0, 0.0)
        );
        let blue = hsl(-120.0, 1.0, 0.5);
        assert_eq!(blue.b.to_f64(), 1.0);
        // Linear 1 is sRGB 1 on the ratio grid.
        assert_eq!(rgb(encode_srgb(1.0), 0.0, 0.0, 1.0).r.to_f64(), 1.0);
        assert_eq!(encode_srgb(-1.0), 0.0);
    }
}
