//! Embedded faces must prove their identity: the family, weight and slant a file's own `name` and `OS/2` tables
//! state must match the face the presentation says it is. Google Slides embeds Roboto Regular bytes for
//! "Roboto Medium", and a Black file in a bold slot; a face that fails is demoted, used for its slot only when no
//! source has the face.

use serde::Serialize;
use slidock_ir::fonts::FaceStyle;

use crate::bind::WeightRange;
use crate::catalog::FamilyKey;
use crate::names::split_weight_name;

/// The weight that splits regular slots from bold ones.
pub const BOLD_LINE: u32 = 600;

/// The face a presentation says a file holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Declared {
    pub family: String,
    /// The slot's weight (400 regular, 700 bold for PPTX; any weight for ODP).
    pub weight: u32,
    pub style: FaceStyle,
}

/// What a file's own tables say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileIdentity<'a> {
    /// Name id 16, else 1.
    pub family: &'a str,
    /// Name id 1.
    pub legacy_family: Option<&'a str>,
    /// One weight, or the `wght` range of a variable file.
    pub weight: WeightRange,
    pub italic: bool,
}

/// Why a face is demoted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Mismatch {
    /// The file names another family.
    Family { file_family: String },
    /// Italic in an upright slot, or the reverse.
    Style { file_italic: bool },
    /// A weight on the wrong side of the bold line, or not the weight the family name spells.
    Weight { file_weight: u32 },
    /// A subset file (an EOT of only the glyphs its document used) without glyphs for `missing` of the code points
    /// the presentation draws with the face.
    Subset { missing: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum Identity {
    Accepted,
    Demoted(Mismatch),
}

impl Identity {
    pub fn is_accepted(&self) -> bool {
        matches!(self, Identity::Accepted)
    }
}

/// Checks a file against the face a presentation declares it to be. Accepted when:
/// 1. the file's family (or its legacy family) is the declared one, or the declared name splits into a family and a
///    weight name (`Roboto Medium`) and the file is that family at that weight;
/// 2. the file's slant is the slot's;
/// 3. the file's weight is on the slot's side of the bold line, and is the weight the declared name spells when the
///    file's family is not the whole declared name (a variable file: its range reaches the slot's side, or holds the
///    spelled weight). A file whose family is the whole name (`Archivo Black`, `Shadows Into Light`) is that family,
///    whatever its name ends with.
pub fn check_identity(declared: &Declared, file: FileIdentity<'_>) -> Identity {
    let key = FamilyKey::of(&declared.family);
    let named = |family: &str| FamilyKey::of(family) == key;
    let whole = named(file.family);
    let split = split_weight_name(&declared.family).filter(|_| !whole);
    let direct = whole || file.legacy_family.is_some_and(named);
    let via_split = split.is_some_and(|split| {
        let base = FamilyKey::of(split.family);
        FamilyKey::of(file.family) == base
            || file
                .legacy_family
                .is_some_and(|legacy| FamilyKey::of(legacy) == base)
    });
    if !direct && !via_split {
        return Identity::Demoted(Mismatch::Family {
            file_family: file.family.to_owned(),
        });
    }
    if file.italic != (declared.style == FaceStyle::Italic) {
        return Identity::Demoted(Mismatch::Style {
            file_italic: file.italic,
        });
    }
    let range = file.weight;
    let weight_ok = match split {
        Some(split) => (range.min..=range.max).contains(&split.weight),
        None if declared.weight >= BOLD_LINE => range.max >= BOLD_LINE,
        None => range.min < BOLD_LINE,
    };
    if !weight_ok {
        return Identity::Demoted(Mismatch::Weight {
            file_weight: range.min,
        });
    }
    Identity::Accepted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared(family: &str, weight: u32, style: FaceStyle) -> Declared {
        Declared {
            family: family.to_owned(),
            weight,
            style,
        }
    }

    fn file<'a>(
        family: &'a str,
        legacy: Option<&'a str>,
        weight: u32,
        italic: bool,
    ) -> FileIdentity<'a> {
        FileIdentity {
            family,
            legacy_family: legacy,
            weight: WeightRange::point(weight),
            italic,
        }
    }

    #[test]
    fn a_variable_file_proves_any_weight_its_range_holds() {
        let caveat = |min, max| FileIdentity {
            family: "Caveat",
            legacy_family: None,
            weight: WeightRange { min, max },
            italic: false,
        };
        let bold = declared("Caveat", 700, FaceStyle::Normal);
        assert!(check_identity(&bold, caveat(400, 700)).is_accepted());
        assert!(
            check_identity(
                &declared("Caveat", 400, FaceStyle::Normal),
                caveat(400, 700)
            )
            .is_accepted()
        );
        assert!(
            check_identity(
                &declared("Caveat Medium", 400, FaceStyle::Normal),
                caveat(400, 700)
            )
            .is_accepted()
        );
        // Negative control: a range that stays below the bold line does not prove bold.
        assert_eq!(
            check_identity(&bold, caveat(300, 500)),
            Identity::Demoted(Mismatch::Weight { file_weight: 300 })
        );
    }

    #[test]
    fn a_regular_file_in_a_medium_request_is_demoted() {
        let medium = declared("Roboto Medium", 400, FaceStyle::Normal);
        assert_eq!(
            check_identity(&medium, file("Roboto", Some("Roboto"), 400, false)),
            Identity::Demoted(Mismatch::Weight { file_weight: 400 })
        );
        // Negative control: the real Medium file is accepted, by its family and by its legacy name.
        assert!(
            check_identity(&medium, file("Roboto", Some("Roboto Medium"), 500, false))
                .is_accepted()
        );
        assert!(check_identity(&medium, file("Roboto", None, 500, false)).is_accepted());
    }

    #[test]
    fn the_bold_line_splits_regular_and_bold_slots() {
        let bold = declared("Barlow Condensed", 700, FaceStyle::Normal);
        assert!(check_identity(&bold, file("Barlow Condensed", None, 700, false)).is_accepted());
        assert_eq!(
            check_identity(&bold, file("Barlow Condensed", None, 400, false)),
            Identity::Demoted(Mismatch::Weight { file_weight: 400 })
        );
        let regular = declared("Barlow Condensed", 400, FaceStyle::Normal);
        assert_eq!(
            check_identity(&regular, file("Barlow Condensed", None, 600, false)),
            Identity::Demoted(Mismatch::Weight { file_weight: 600 })
        );
        assert!(check_identity(&regular, file("Barlow Condensed", None, 300, false)).is_accepted());
    }

    #[test]
    fn a_family_whose_name_ends_in_a_weight_word_is_that_family() {
        for (family, weight) in [
            ("Archivo Black", 400),
            ("Shadows Into Light", 400),
            ("Love Light", 400),
            ("Bungee Hairline", 400),
        ] {
            let regular = declared(family, 400, FaceStyle::Normal);
            assert!(
                check_identity(&regular, file(family, Some(family), weight, false)).is_accepted(),
                "{family}"
            );
            assert!(check_identity(&regular, file(family, None, weight, false)).is_accepted());
            // Negative control: the family's own bold line still holds.
            assert_eq!(
                check_identity(
                    &declared(family, 700, FaceStyle::Normal),
                    file(family, None, weight, false)
                ),
                Identity::Demoted(Mismatch::Weight {
                    file_weight: weight
                })
            );
        }
        // A file of the split family is still held to the spelled weight.
        let black = declared("Archivo Black", 400, FaceStyle::Normal);
        assert_eq!(
            check_identity(&black, file("Archivo", Some("Archivo"), 400, false)),
            Identity::Demoted(Mismatch::Weight { file_weight: 400 })
        );
        assert!(
            check_identity(&black, file("Archivo", Some("Archivo Black"), 900, false))
                .is_accepted()
        );
    }

    #[test]
    fn another_family_or_slant_is_demoted() {
        let roboto = declared("Roboto", 400, FaceStyle::Italic);
        assert_eq!(
            check_identity(&roboto, file("Oswald", None, 400, true)),
            Identity::Demoted(Mismatch::Family {
                file_family: "Oswald".into()
            })
        );
        assert_eq!(
            check_identity(&roboto, file("Roboto", None, 400, false)),
            Identity::Demoted(Mismatch::Style { file_italic: false })
        );
        assert!(check_identity(&roboto, file("ROBOTO", None, 400, true)).is_accepted());
    }
}
