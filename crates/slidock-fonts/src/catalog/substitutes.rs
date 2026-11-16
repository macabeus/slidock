//! Metric-compatible stand-ins for families a presentation asks for and the catalog lacks, and the family each CSS
//! generic renders with. Every target is in the built-in table, or for a deck that ships its fonts in the floor
//! table (Caladea), so a substitution always has metrics; a catalog without the target binds the generic instead.

use slidock_ir::fonts::GenericFamily;

use super::FamilyKey;
use crate::names::generic_of;

/// Requested family keys and their substitutes.
pub const SUBSTITUTES: &[(&str, &str)] = &[
    ("arial", "Liberation Sans"),
    ("helvetica", "Liberation Sans"),
    ("helveticaneue", "Liberation Sans"),
    ("timesnewroman", "Liberation Serif"),
    ("times", "Liberation Serif"),
    ("couriernew", "Liberation Mono"),
    ("courier", "Liberation Mono"),
    ("calibri", "Carlito"),
    ("cambria", "Caladea"),
    ("starsymbol", "DejaVu Sans"),
    ("opensymbol", "DejaVu Sans"),
    ("symbol", "DejaVu Sans"),
];

/// Keys of the symbol fonts PowerPoint and Word draw bullets and pictographs with: their code points are their own
/// pictures (a symbol `cmap`), so a face of another family draws other characters and none stands in for them.
pub const SYMBOL_FAMILIES: &[&str] = &[
    "symbol",
    "wingdings",
    "wingdings2",
    "wingdings3",
    "webdings",
    "marlett",
    "mtextra",
];

/// The generics of bundled families whose names do not suggest theirs ([`generic_of`] reads names).
const BUNDLED_GENERICS: &[(&str, GenericFamily)] = &[("caladea", GenericFamily::Serif)];

/// The bundled face that draws the code points no face a deck ships has, when that deck ships its fonts: a family the
/// bundled substitutes hold, and its PostScript name.
pub const LAST_RESORT: (&str, &str) = ("DejaVu Sans", "DejaVuSans");

/// The metric-compatible substitute for a family, if it has one.
pub fn substitute(key: &FamilyKey) -> Option<&'static str> {
    SUBSTITUTES
        .iter()
        .find(|(requested, _)| *requested == key.as_str())
        .map(|&(_, target)| target)
}

/// Whether a family is one of the [`SYMBOL_FAMILIES`].
pub fn is_symbol_family(key: &FamilyKey) -> bool {
    SYMBOL_FAMILIES.contains(&key.as_str())
}

/// The CSS generic of a catalog family: a bundled family's own, else the one its name suggests.
pub fn family_generic(family: &str) -> GenericFamily {
    let key = FamilyKey::of(family);
    BUNDLED_GENERICS
        .iter()
        .find(|(bundled, _)| *bundled == key.as_str())
        .map_or_else(|| generic_of(family), |&(_, generic)| generic)
}

/// The family a CSS generic renders with, as the bench renderer's font configuration aliases them.
pub fn generic_family(generic: GenericFamily) -> &'static str {
    match generic {
        GenericFamily::SansSerif | GenericFamily::Cursive | GenericFamily::Fantasy => {
            "Liberation Sans"
        }
        GenericFamily::Serif => "Liberation Serif",
        GenericFamily::Monospace => "Liberation Mono",
        GenericFamily::Emoji => super::emoji::PLATFORM_EMOJI,
    }
}

#[cfg(test)]
mod tests {
    use super::super::builtin::{builtin_catalog, floor_catalog};
    use super::*;

    #[test]
    fn substitutes_by_font_key() {
        assert_eq!(
            substitute(&FamilyKey::of("Helvetica Neue")),
            Some("Liberation Sans")
        );
        assert_eq!(
            substitute(&FamilyKey::of("Times New Roman")),
            Some("Liberation Serif")
        );
        assert_eq!(
            substitute(&FamilyKey::of("COURIER-NEW")),
            Some("Liberation Mono")
        );
        assert_eq!(substitute(&FamilyKey::of("Calibri")), Some("Carlito"));
        assert_eq!(substitute(&FamilyKey::of("Cambria")), Some("Caladea"));
        assert_eq!(
            substitute(&FamilyKey::of("OpenSymbol")),
            Some("DejaVu Sans")
        );
        assert_eq!(substitute(&FamilyKey::of("Calibri Light")), None);
        assert_eq!(substitute(&FamilyKey::of("Roboto")), None);
    }

    #[test]
    fn every_target_has_built_in_metrics() {
        let generics = [
            GenericFamily::SansSerif,
            GenericFamily::Serif,
            GenericFamily::Monospace,
            GenericFamily::Cursive,
            GenericFamily::Fantasy,
        ];
        let targets = SUBSTITUTES
            .iter()
            .map(|&(_, target)| target)
            .chain(generics.map(generic_family));
        for target in targets {
            let key = FamilyKey::of(target);
            let family = builtin_catalog()
                .get(&key)
                .or_else(|| floor_catalog().get(&key));
            assert!(family.is_some_and(|f| !f.faces.is_empty()), "{target}");
        }
    }

    #[test]
    fn the_last_resort_is_a_bundled_face() {
        let (family, postscript_name) = LAST_RESORT;
        let family = builtin_catalog().get(&FamilyKey::of(family)).unwrap();
        assert!(
            family
                .faces
                .iter()
                .any(|face| face.postscript_name == postscript_name)
        );
    }

    #[test]
    fn caladea_is_serif_and_other_families_take_the_generic_their_names_suggest() {
        assert_eq!(family_generic("Caladea"), GenericFamily::Serif);
        assert_eq!(family_generic("CALA-DEA"), GenericFamily::Serif);
        // Negative controls: the name alone says sans-serif for Caladea; other families read their names.
        assert_eq!(generic_of("Caladea"), GenericFamily::SansSerif);
        assert_eq!(family_generic("Carlito"), GenericFamily::SansSerif);
        assert_eq!(family_generic("Liberation Serif"), GenericFamily::Serif);
        assert_eq!(family_generic("Liberation Mono"), GenericFamily::Monospace);
    }

    #[test]
    fn a_monospace_family_without_a_substitute_falls_back_to_liberation_mono() {
        for family in ["Monaco", "Menlo", "Cousine"] {
            assert_eq!(substitute(&FamilyKey::of(family)), None, "{family}");
            assert_eq!(
                generic_family(generic_of(family)),
                "Liberation Mono",
                "{family}"
            );
            assert_eq!(family_generic(family), GenericFamily::Monospace, "{family}");
        }
        // Negative control: a sans-serif family without a substitute falls back to Liberation Sans.
        assert_eq!(generic_family(generic_of("Avenir")), "Liberation Sans");
    }

    #[test]
    fn symbol_families_by_font_key() {
        assert!(is_symbol_family(&FamilyKey::of("Wingdings")));
        assert!(is_symbol_family(&FamilyKey::of("Wingdings 2")));
        assert!(is_symbol_family(&FamilyKey::of("SYMBOL")));
        assert!(!is_symbol_family(&FamilyKey::of("OpenSymbol")));
        assert!(!is_symbol_family(&FamilyKey::of("Arial")));
        for &key in SYMBOL_FAMILIES {
            assert_eq!(FamilyKey::of(key).as_str(), key);
        }
    }

    #[test]
    fn keys_are_font_keys() {
        for &(requested, _) in SUBSTITUTES {
            assert_eq!(FamilyKey::of(requested).as_str(), requested);
        }
    }
}
