//! ODF's value grammar at the boundary: lengths (cm, mm, in, pt, pc, px), percentages and `#rrggbb` colours, each
//! parsed strictly (plain decimals, matching the whole value) and converted once to the IR's units and grid.

use slidock_canon::{is_js_whitespace, js_trim, js_trim_start};
use slidock_ir::color::{ColorSpace, RgbColor};
use slidock_ir::units::{Pt, Ratio, Size, UnitRatio};
use slidock_math::{Grid, quantise};

const UNITS: [(&str, f64); 7] = [
    ("cm", 72.0 / 2.54),
    ("mm", 72.0 / 25.4),
    ("inch", 72.0),
    ("in", 72.0),
    ("pt", 1.0),
    ("pc", 12.0),
    ("px", 0.75),
];

const PT_LIMIT: f64 = 100_000.0;

fn clamp(x: f64, min: f64, max: f64) -> f64 {
    if x < min {
        min
    } else if x > max {
        max
    } else {
        x
    }
}

/// A decimal number as XML Schema writes it, with bounded digit counts (`[-+]?(\d{1,12}(\.\d{0,20})?|\.\d{1,20})`),
/// at the start of `text`: its value and the text after it.
fn leading_number(text: &str) -> Option<(f64, &str)> {
    let bytes = text.as_bytes();
    let mut at = usize::from(matches!(bytes.first(), Some(b'-' | b'+')));
    let digits = |from: usize| {
        bytes.get(from..).map_or(0, |rest| {
            rest.iter().take_while(|b| b.is_ascii_digit()).count()
        })
    };
    let whole = digits(at);
    at += whole;
    let mut fraction = None;
    if bytes.get(at) == Some(&b'.') {
        let count = digits(at + 1);
        fraction = Some(count);
        at += 1 + count;
    }
    let valid = match fraction {
        _ if whole > 12 => false,
        None => whole >= 1,
        Some(count) => count <= 20 && (whole >= 1 || count >= 1),
    };
    if !valid {
        return None;
    }
    let literal = text.get(..at)?;
    let value: f64 = literal.parse().ok()?;
    Some((value, text.get(at..).unwrap_or_default()))
}

/// The number a whole value states, white space around it allowed, and what follows it before trailing space.
fn number_then(value: &str) -> Option<(f64, &str)> {
    let (number, rest) = leading_number(js_trim_start(value))?;
    Some((number, js_trim(rest)))
}

/// A length in points, unquantised; `None` when the value is not a length.
pub(crate) fn raw_length(value: Option<&str>) -> Option<f64> {
    let (number, unit) = number_then(value?)?;
    UNITS
        .iter()
        .find(|(name, _)| *name == unit)
        .map(|(_, scale)| number * scale)
}

/// Points on the IR's grid and within its range.
pub(crate) fn to_pt(pt: f64) -> f64 {
    quantise(clamp(pt, -PT_LIMIT, PT_LIMIT), Grid::Thousandths)
}

/// A length in points on the IR's grid and within its range.
pub(crate) fn length_pt(value: Option<&str>) -> Option<f64> {
    raw_length(value).map(to_pt)
}

/// A length clamped at zero.
pub(crate) fn size_pt(value: Option<&str>) -> Option<f64> {
    length_pt(value).map(|pt| pt.max(0.0))
}

/// `45%` as 0.45; `None` when the value is not a percentage.
pub(crate) fn percent(value: Option<&str>) -> Option<f64> {
    let (number, rest) = number_then(value?)?;
    (rest.strip_prefix('%').map(js_trim) == Some("")).then_some(number / 100.0)
}

pub(crate) fn plain_number(value: Option<&str>) -> Option<f64> {
    let (number, rest) = number_then(value?)?;
    rest.is_empty().then_some(number)
}

/// An angle in degrees: `30deg`, `0.5rad`, `40grad`, or a plain number times `plain_scale` (a gradient's unitless
/// `draw:angle` may be tenths of a degree, gradient.rs).
pub(crate) fn angle_deg(value: Option<&str>, plain_scale: f64) -> Option<f64> {
    let (n, unit) = number_then(value?)?;
    match unit {
        "deg" => Some(n),
        "rad" => Some((n * 180.0) / std::f64::consts::PI),
        "grad" => Some(n * 0.9),
        "" => Some(n * plain_scale),
        _ => None,
    }
}

/// A value on the ratio grid, clamped to 0–1.
pub(crate) fn ratio(x: f64) -> f64 {
    quantise(clamp(x, 0.0, 1.0), Grid::HundredThousandths)
}

/// An sRGB colour from channels in 0–1.
pub(crate) fn rgb(r: f64, g: f64, b: f64, a: f64) -> RgbColor {
    RgbColor::new(
        ColorSpace::Srgb,
        unit_ratio(ratio(r)),
        unit_ratio(ratio(g)),
        unit_ratio(ratio(b)),
        unit_ratio(ratio(a)),
    )
}

/// `#rrggbb` (and `#rgb`, which some writers use).
pub(crate) fn hex_color(value: Option<&str>) -> Option<RgbColor> {
    let hex = js_trim(value?).strip_prefix('#')?;
    if !(hex.len() == 6 || hex.len() == 3) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let full: Vec<u8> = if hex.len() == 3 {
        hex.bytes().flat_map(|b| [b, b]).collect()
    } else {
        hex.bytes().collect()
    };
    let channel = |at: usize| {
        let pair = full.get(at..at + 2).unwrap_or_default();
        let text = std::str::from_utf8(pair).unwrap_or("0");
        f64::from(u8::from_str_radix(text, 16).unwrap_or(0)) / 255.0
    };
    Some(rgb(channel(0), channel(2), channel(4), 1.0))
}

/// A colour with an opacity applied to its alpha.
pub(crate) fn with_alpha(color: &RgbColor, alpha: f64) -> RgbColor {
    let mut out = color.clone();
    out.a = unit_ratio(ratio(color.a.to_f64() * alpha));
    out
}

/// Whether a character is one JavaScript's `\s` matches.
pub(crate) fn is_space(c: char) -> bool {
    is_js_whitespace(c)
}

/// A quantised value as the IR's point type.
pub(crate) fn pt(x: f64) -> Pt {
    Pt::quantise(x).unwrap_or(Pt::ZERO)
}

/// A quantised value as the IR's size type.
pub(crate) fn size(x: f64) -> Size {
    Size::quantise(x).unwrap_or(Size::ZERO)
}

/// A quantised value as the IR's ratio type.
pub(crate) fn ratio_of(x: f64) -> Ratio {
    Ratio::quantise(x).unwrap_or(Ratio::ZERO)
}

/// A quantised value as the IR's unit ratio type.
pub(crate) fn unit_ratio(x: f64) -> UnitRatio {
    UnitRatio::quantise(x).unwrap_or(UnitRatio::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lengths_strictly_into_points_on_the_ir_grid() {
        assert_eq!(length_pt(Some("2.54cm")), Some(72.0));
        assert_eq!(length_pt(Some("-0.5in")), Some(-36.0));
        assert_eq!(length_pt(Some("10mm")), Some(28.346));
        assert_eq!(length_pt(Some("1pc")), Some(12.0));
        assert_eq!(length_pt(Some("4px")), Some(3.0));
        assert_eq!(length_pt(Some(".5pt")), Some(0.5));
        assert_eq!(length_pt(Some(" 2 inch ")), Some(144.0));
        assert_eq!(length_pt(Some("1.pt")), Some(1.0));
        assert_eq!(length_pt(Some("1e3pt")), None);
        assert_eq!(length_pt(Some("12")), None);
        assert_eq!(length_pt(Some("12 pt x")), None);
        assert_eq!(length_pt(Some("1234567890123pt")), None);
        assert_eq!(length_pt(Some("1.123456789012345678901pt")), None);
        assert_eq!(length_pt(Some(".pt")), None);
        assert_eq!(size_pt(Some("-1cm")), Some(0.0));
    }

    #[test]
    fn parses_percentages_colours_and_angles() {
        assert_eq!(percent(Some("45%")), Some(0.45));
        assert_eq!(percent(Some(" 45 % ")), Some(0.45));
        assert_eq!(percent(Some("45")), None);
        let orange = hex_color(Some("#ff8000")).unwrap();
        assert_eq!(
            [orange.r, orange.g, orange.b, orange.a].map(UnitRatio::to_f64),
            [1.0, 0.50196, 0.0, 1.0]
        );
        assert_eq!(hex_color(Some("#f80")), hex_color(Some("#ff8800")));
        assert_eq!(hex_color(Some("red")), None);
        assert_eq!(angle_deg(Some("300"), 0.1), Some(30.0));
        assert_eq!(angle_deg(Some("30deg"), 0.1), Some(30.0));
        assert_eq!(angle_deg(Some("100grad"), 0.1), Some(90.0));
        assert_eq!(angle_deg(Some("1turn"), 0.1), None);
    }
}
