//! Colours as CSS. An unmodified theme reference prints a variable with its resolved value as the fallback, so a
//! presentation can be re-themed and an editor recovers the reference; everything else prints the literal.

use slidock_canon::js_round;
use slidock_format::markers::COLOR_VAR_PREFIX;
use slidock_ir::color::{ColorSpace, ResolvedColor, RgbColor};
use slidock_ir::units::UnitRatio;

use crate::css::ratio;
use crate::error::Result;

fn hex2(component: UnitRatio, out: &mut String) {
    let byte = js_round(component.to_f64() * 255.0).clamp(0.0, 255.0) as u8;
    out.push_str(&format!("{byte:02x}"));
}

fn opaque(color: &RgbColor) -> bool {
    color.a.to_f64() >= 1.0
}

/// `#rrggbb`, `#rrggbbaa` when not opaque, or `color(display-p3 r g b / a)`.
pub fn literal_color(color: &RgbColor) -> Result<String> {
    if color.space == ColorSpace::P3 {
        let alpha = if opaque(color) {
            String::new()
        } else {
            format!(" / {}", ratio(color.a.to_f64())?)
        };
        return Ok(format!(
            "color(display-p3 {} {} {}{alpha})",
            ratio(color.r.to_f64())?,
            ratio(color.g.to_f64())?,
            ratio(color.b.to_f64())?
        ));
    }
    let mut out = String::from("#");
    hex2(color.r, &mut out);
    hex2(color.g, &mut out);
    hex2(color.b, &mut out);
    if !opaque(color) {
        hex2(color.a, &mut out);
    }
    Ok(out)
}

pub fn css_color(color: &ResolvedColor) -> Result<String> {
    match color {
        ResolvedColor::Rgb(rgb) => literal_color(rgb),
        ResolvedColor::Theme(theme) => {
            let literal = literal_color(&theme.resolved)?;
            if theme.mods.is_empty() {
                Ok(format!(
                    "var({COLOR_VAR_PREFIX}{},{literal})",
                    theme.slot.as_str()
                ))
            } else {
                Ok(literal)
            }
        }
    }
}
