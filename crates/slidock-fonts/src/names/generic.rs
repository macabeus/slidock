//! The CSS generic family a family name suggests.

use slidock_ir::fonts::{GenericFamily, font_key};

use super::split_weight_name;
use crate::catalog::FamilyKey;
use crate::google::google_catalog;

const MONOSPACE_WORDS: &[&str] = &["mono", "courier", "consol", "typewriter"];

/// Monospace system families whose names hold none of [`MONOSPACE_WORDS`], by font key: macOS's Monaco and Menlo,
/// Windows' Fixedsys, Terminal and OCR A Extended, and Hack, Iosevka and Terminus of Linux systems. The Google Fonts
/// catalog's monospace families ([`google_catalog`]) count as well.
pub const MONOSPACE_FAMILIES: &[&str] = &[
    "monaco",
    "menlo",
    "fixedsys",
    "terminal",
    "ocraextended",
    "hack",
    "iosevka",
    "terminus",
];

const SERIF_NAMES: &[&str] = &[
    "serif",
    "times",
    "georgia",
    "garamond",
    "baskerville",
    "palatino",
    "cambria",
    "didot",
    "bodoni",
    "caslon",
    "minion",
];

/// A regular-expression word character: ASCII letters, digits and `_`.
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Whether `word` occurs in `name`, with a word boundary before it when `before` and after it when `after`.
fn has_word(name: &str, word: &str, before: bool, after: bool) -> bool {
    let bytes = name.as_bytes();
    let Some(last) = bytes.len().checked_sub(word.len()) else {
        return false;
    };
    (0..=last).any(|at| {
        bytes.get(at..at + word.len()) == Some(word.as_bytes())
            && (!before || at == 0 || bytes.get(at - 1).is_some_and(|&b| !is_word_byte(b)))
            && (!after || bytes.get(at + word.len()).is_none_or(|&b| !is_word_byte(b)))
    })
}

/// Whether a family, or the family before a trailing weight name (`Menlo Bold`), is one of the
/// [`MONOSPACE_FAMILIES`] or a monospace family of the Google Fonts catalog.
fn is_monospace_family(family: &str) -> bool {
    let known = |name: &str| {
        MONOSPACE_FAMILIES.contains(&font_key(name).as_str())
            || google_catalog()
                .get(&FamilyKey::of(name))
                .is_some_and(|google| google.generic == GenericFamily::Monospace)
    };
    known(family) || split_weight_name(family).is_some_and(|split| known(split.family))
}

/// Monospace names (mono, courier, consol, a closing "code", typewriter) and known monospace families, then sans
/// names, then whole serif names; sans-serif otherwise. Only ASCII letters fold case.
pub fn generic_of(family: &str) -> GenericFamily {
    let name = family.to_ascii_lowercase();
    if MONOSPACE_WORDS.iter().any(|word| name.contains(word))
        || has_word(&name, "code", false, true)
        || is_monospace_family(family)
    {
        return GenericFamily::Monospace;
    }
    if name.contains("sans") {
        return GenericFamily::SansSerif;
    }
    if SERIF_NAMES
        .iter()
        .any(|word| has_word(&name, word, true, true))
    {
        return GenericFamily::Serif;
    }
    GenericFamily::SansSerif
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_generic_families_from_names() {
        assert_eq!(generic_of("Times New Roman"), GenericFamily::Serif);
        assert_eq!(generic_of("Liberation Serif"), GenericFamily::Serif);
        assert_eq!(generic_of("PT Sans"), GenericFamily::SansSerif);
        assert_eq!(generic_of("Courier New"), GenericFamily::Monospace);
        assert_eq!(generic_of("Helvetica Neue"), GenericFamily::SansSerif);
        assert_eq!(generic_of("DejaVu Sans Mono"), GenericFamily::Monospace);
        assert_eq!(generic_of("Consolas"), GenericFamily::Monospace);
    }

    #[test]
    fn serif_names_are_whole_words_and_code_ends_a_word() {
        assert_eq!(generic_of("Timesless"), GenericFamily::SansSerif);
        assert_eq!(generic_of("Maxtimes"), GenericFamily::SansSerif);
        assert_eq!(generic_of("Max2Times"), GenericFamily::SansSerif);
        assert_eq!(generic_of("New Times"), GenericFamily::Serif);
        assert_eq!(generic_of("Times_New"), GenericFamily::SansSerif);
        assert_eq!(generic_of("\u{e9}Times"), GenericFamily::Serif);
        assert_eq!(generic_of("Source Code Pro"), GenericFamily::Monospace);
        assert_eq!(generic_of("Fira Code"), GenericFamily::Monospace);
        assert_eq!(generic_of("Codex"), GenericFamily::SansSerif);
        assert_eq!(generic_of("Sans Serif"), GenericFamily::SansSerif);
    }

    #[test]
    fn known_monospace_families_are_monospace_whatever_their_names_spell() {
        for family in [
            "Monaco",
            "MONACO",
            "Menlo",
            "Menlo Bold",
            "Hack",
            "OCR A Extended",
        ] {
            assert_eq!(generic_of(family), GenericFamily::Monospace, "{family}");
        }
        // Google lists these as monospace; their names hold no monospace word.
        for family in ["Cousine", "Anonymous Pro", "VT323", "Lekton", "Sono Light"] {
            assert_eq!(generic_of(family), GenericFamily::Monospace, "{family}");
        }
        // Negative controls: other families keep the generic their names suggest, a known name must be the whole
        // family, and Google's other categories are not read.
        assert_eq!(generic_of("Monacoish"), GenericFamily::SansSerif);
        assert_eq!(generic_of("Terminal Grotesque"), GenericFamily::SansSerif);
        assert_eq!(generic_of("Helvetica Neue"), GenericFamily::SansSerif);
        assert_eq!(generic_of("Lora"), GenericFamily::SansSerif);
        assert_eq!(generic_of("Georgia"), GenericFamily::Serif);
    }

    #[test]
    fn every_monospace_family_of_the_tables_is_monospace() {
        for &key in MONOSPACE_FAMILIES {
            assert_eq!(font_key(key), key);
            assert_eq!(generic_of(key), GenericFamily::Monospace, "{key}");
            assert!(
                !MONOSPACE_WORDS.iter().any(|word| key.contains(word)),
                "{key} needs no entry"
            );
        }
        let google: Vec<&str> = google_catalog()
            .families()
            .iter()
            .filter(|family| family.generic == GenericFamily::Monospace)
            .map(|family| family.family.as_str())
            .collect();
        assert_eq!(google.len(), 46);
        for family in google {
            assert_eq!(generic_of(family), GenericFamily::Monospace, "{family}");
        }
    }
}
