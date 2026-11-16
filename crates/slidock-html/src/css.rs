//! Numbers and declarations as the printer writes them: every number on the IR's grid through the canonical formatter,
//! so the same presentation prints the same bytes on every engine.

use slidock_canon::{Grid, format_grid};

use crate::error::Result;

/// A CSS declaration.
pub type Decl = (&'static str, String);
/// A declaration that may be absent; absent ones are skipped.
pub type MaybeDecl = Option<Decl>;

/// Up to 3 decimals, trailing zeros trimmed (`formatFixed`).
pub fn fixed(x: f64) -> Result<String> {
    Ok(format_grid(x, Grid::Thousandths)?)
}

/// Points: up to 3 decimals, always with the unit (`0pt`).
pub fn pt(x: f64) -> Result<String> {
    Ok(format!("{}pt", fixed(x)?))
}

pub fn deg(x: f64) -> Result<String> {
    Ok(format!("{}deg", fixed(x)?))
}

pub fn percent(x: f64) -> Result<String> {
    Ok(format!("{}%", fixed(x)?))
}

/// Unitless ratios: up to 5 decimals (`formatRatio`).
pub fn ratio(x: f64) -> Result<String> {
    Ok(format_grid(x, Grid::HundredThousandths)?)
}

/// A number that is not quantised (milliseconds, indices, IR values already on their grid): `String(n)`.
pub fn number(x: f64) -> Result<String> {
    Ok(slidock_canon::format_number(x)?)
}

/// CSS pixels per point (clip paths are in the box's px).
pub use slidock_ir::units::PX_PER_PT;

/// Four sides in points as the shortest `padding` or `margin` value: one value where all match, else two or three.
/// It compares the printed values.
pub fn sides(top: f64, right: f64, bottom: f64, left: f64) -> Result<String> {
    let (t, r, b, l) = (pt(top)?, pt(right)?, pt(bottom)?, pt(left)?);
    Ok(if l != r {
        format!("{t} {r} {b} {l}")
    } else if b != t {
        format!("{t} {r} {b}")
    } else if r == t {
        t
    } else {
        format!("{t} {r}")
    })
}

/// `property:value;…` in the given order, without a trailing semicolon.
pub fn style(decls: impl IntoIterator<Item = MaybeDecl>) -> String {
    let mut out = String::new();
    for (property, value) in decls.into_iter().flatten() {
        if !out.is_empty() {
            out.push(';');
        }
        out.push_str(property);
        out.push(':');
        out.push_str(&value);
    }
    out
}

/// A declaration that is always present.
pub fn decl(property: &'static str, value: impl Into<String>) -> MaybeDecl {
    Some((property, value.into()))
}

/// A declaration present when `on`.
pub fn decl_if(on: bool, property: &'static str, value: impl FnOnce() -> String) -> MaybeDecl {
    on.then(|| (property, value()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_four_sides_as_the_shortest_value() {
        assert_eq!(sides(1.0, 1.0, 1.0, 1.0).unwrap(), "1pt");
        assert_eq!(sides(1.0, 2.0, 1.0, 2.0).unwrap(), "1pt 2pt");
        assert_eq!(sides(1.0, 2.0, 3.0, 2.0).unwrap(), "1pt 2pt 3pt");
        assert_eq!(sides(1.0, 2.0, 1.0, 3.0).unwrap(), "1pt 2pt 1pt 3pt");
        assert_eq!(sides(0.0, 0.0, 0.0, 0.0).unwrap(), "0pt");
        // Compared as printed: below the grid, sides match.
        assert_eq!(sides(1.0, 1.0001, 1.0, 0.9999).unwrap(), "1pt");
    }

    #[test]
    fn prints_units_on_the_grid() {
        assert_eq!(pt(7.1995).unwrap(), "7.199pt");
        assert_eq!(pt(-0.0005).unwrap(), "0pt");
        assert_eq!(deg(30.0).unwrap(), "30deg");
        assert_eq!(percent(-12.5).unwrap(), "-12.5%");
        assert_eq!(ratio(1.0 / 3.0).unwrap(), "0.33333");
        assert_eq!(number(500.0).unwrap(), "500");
        assert!(pt(f64::NAN).is_err());
    }

    #[test]
    fn joins_present_declarations() {
        assert_eq!(
            style([
                decl("a", "1"),
                None,
                decl_if(true, "b", || "2".into()),
                decl_if(false, "c", String::new)
            ]),
            "a:1;b:2"
        );
        assert_eq!(style([None]), "");
    }
}
