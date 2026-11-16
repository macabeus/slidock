//! Keynote names fonts by PostScript name only (`HelveticaNeue-Medium`) and embeds none. A name becomes a font
//! request: the family its name spells, the face's weight and style, and the PostScript name itself, which binding
//! and the bench's oracle check match exactly.

use slidock_fonts::names::postscript_face;

/// Keynote's text default, when no style in a run's chain names a font.
pub const DEFAULT_POSTSCRIPT_NAME: &str = "HelveticaNeue";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeynoteFace {
    pub family: String,
    pub weight: u32,
    pub italic: bool,
    pub postscript_name: String,
}

/// The family, weight and style a PostScript name spells; a name without a weight word is the regular face.
pub fn font_face(postscript_name: &str) -> KeynoteFace {
    let face = postscript_face(postscript_name);
    KeynoteFace {
        family: if face.family.is_empty() {
            postscript_name.to_owned()
        } else {
            face.family
        },
        weight: face.weight.unwrap_or(400),
        italic: face.italic,
        postscript_name: postscript_name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_family_weight_and_style_a_name_spells() {
        for (name, family, weight, italic) in [
            ("HelveticaNeue-Bold", "Helvetica Neue", 700, false),
            ("CourierNewPS-BoldItalicMT", "Courier New", 700, true),
            ("CourierNewPSMT", "Courier New", 400, false),
            ("TrumpGothicEast-Bold", "Trump Gothic East", 700, false),
            ("8-Bit-Madness", "8 Bit Madness", 400, false),
            ("Monoid-Regular", "Monoid", 400, false),
            (".SFNS-Semibold", "SFNS", 600, false),
        ] {
            assert_eq!(
                font_face(name),
                KeynoteFace {
                    family: family.to_owned(),
                    weight,
                    italic,
                    postscript_name: name.to_owned(),
                },
                "{name}"
            );
        }
    }
}
