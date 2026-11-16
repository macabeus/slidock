//! PostScript names. Keynote names its fonts by PostScript name only, and a PDF names each font it draws with one
//! (`ABCDEF+Roboto-MediumItalic`, `TimesNewRomanPS-BoldMT`): an optional subset tag, the family with no spaces,
//! and a style suffix. Every frontend reads them here, so the same face asks for the same family and weight
//! whichever format carried it.

use slidock_canon::js_trim;

use super::weights::weight_of_name;

/// What a PostScript name spells; `weight` is `None` when the name holds no weight word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostscriptFace {
    pub family: String,
    pub weight: Option<u32>,
    pub italic: bool,
}

/// The family names of macOS fonts whose PostScript stem differs from the family with its spaces removed.
const APPLE_FAMILIES: &[(&str, &str)] = &[
    ("HelveticaNeue", "Helvetica Neue"),
    ("ArialNarrow", "Arial Narrow"),
    ("ArialRoundedMTBold", "Arial Rounded MT Bold"),
    ("AvenirNext", "Avenir Next"),
    ("AvenirNextCondensed", "Avenir Next Condensed"),
    ("GillSans", "Gill Sans"),
    ("TrebuchetMS", "Trebuchet MS"),
    ("TimesNewRoman", "Times New Roman"),
    ("CourierNew", "Courier New"),
    ("AmericanTypewriter", "American Typewriter"),
    ("MarkerFelt", "Marker Felt"),
    ("ChalkboardSE", "Chalkboard SE"),
    ("SFPro", "SF Pro"),
    ("SFProDisplay", "SF Pro Display"),
    ("SFProText", "SF Pro Text"),
    ("SFMono", "SF Mono"),
];

/// The words a style suffix is made of, longest first so `ExtraBold` reads as one word. Width words belong to the
/// face too: `Futura-CondensedExtraBold` is Futura's condensed extra-bold face.
const STYLE_WORDS: &[&str] = &[
    "extrablack",
    "ultrablack",
    "extrabold",
    "ultrabold",
    "extralight",
    "ultralight",
    "semilight",
    "condensed",
    "semibold",
    "demibold",
    "hairline",
    "regular",
    "oblique",
    "nominal",
    "italic",
    "medium",
    "normal",
    "narrow",
    "black",
    "heavy",
    "light",
    "plain",
    "roman",
    "bold",
    "book",
    "demi",
    "thin",
];

/// The name without the tag a PDF puts before the name of an embedded subset (six capital letters and `+`).
pub fn strip_subset_tag(name: &str) -> &str {
    match name.as_bytes().split_at_checked(7) {
        Some((tag, _))
            if tag.iter().take(6).all(u8::is_ascii_uppercase) && tag.get(6) == Some(&b'+') =>
        {
            name.get(7..).unwrap_or(name)
        }
        _ => name,
    }
}

/// The name without one of the vendor suffixes Adobe and Monotype append to stems and faces (`PSMT`, `PS`, `MT`).
fn strip_vendor_suffix(name: &str) -> &str {
    ["PSMT", "PS", "MT"]
        .into_iter()
        .find_map(|suffix| name.strip_suffix(suffix))
        .unwrap_or(name)
}

/// The style words a suffix is made of, or `None` when it holds anything else (`PGothic` in `MS-PGothic`).
fn style_words(suffix: &str) -> Option<Vec<&'static str>> {
    let lower = strip_vendor_suffix(suffix).to_ascii_lowercase();
    let mut rest = lower.as_str();
    let mut words = Vec::new();
    while !rest.is_empty() {
        let word = STYLE_WORDS
            .iter()
            .copied()
            .find(|word| rest.starts_with(word))?;
        words.push(word);
        rest = rest.get(word.len()..)?;
    }
    (!words.is_empty()).then_some(words)
}

/// A stem as a family name: Apple's own name for it, else `TimesNewRoman` → `Times New Roman`, `MS-PGothic` →
/// `MS PGothic`.
fn family_of(stem: &str) -> String {
    let bare = strip_vendor_suffix(stem);
    if let Some(&(_, family)) = APPLE_FAMILIES.iter().find(|(apple, _)| *apple == bare) {
        return family.to_owned();
    }
    let mut out = String::with_capacity(bare.len() + 4);
    let mut previous: Option<char> = None;
    for c in bare.chars() {
        if c == '-' || c == '_' {
            if !matches!(previous, Some('-' | '_')) {
                out.push(' ');
            }
        } else {
            let camel = previous.is_some_and(|p| p.is_ascii_lowercase()) && c.is_ascii_uppercase();
            let numbered = previous.is_some_and(|p| p.is_ascii_alphabetic()) && c.is_ascii_digit();
            if camel || numbered {
                out.push(' ');
            }
            out.push(c);
        }
        previous = Some(c);
    }
    js_trim(&out).to_owned()
}

/// The family, weight and slant a PostScript name spells. Only style words after the last dash (or a PDF's
/// comma) are a face: `8-Bit-Madness` is one family. Apple's system font names start with a dot.
pub fn postscript_face(name: &str) -> PostscriptFace {
    let bare = strip_subset_tag(name);
    let bare = bare.strip_prefix('.').unwrap_or(bare);
    let bare = js_trim(bare.trim_end_matches('-'));
    let cut = bare.rfind('-').max(bare.rfind(',')).filter(|&cut| cut > 0);
    let words = cut
        .and_then(|cut| bare.get(cut + 1..))
        .and_then(style_words)
        .unwrap_or_default();
    let stem = match cut {
        Some(cut) if !words.is_empty() => bare.get(..cut).unwrap_or(bare),
        _ => bare,
    };
    PostscriptFace {
        family: family_of(stem),
        weight: words.iter().find_map(|word| weight_of_name(word)),
        italic: words
            .iter()
            .any(|word| matches!(*word, "italic" | "oblique")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: &[(&str, &str, Option<u32>, bool)] = &[
        // Keynote
        ("HelveticaNeue", "Helvetica Neue", None, false),
        ("HelveticaNeue-Bold", "Helvetica Neue", Some(700), false),
        ("HelveticaNeue-Medium", "Helvetica Neue", Some(500), false),
        ("CourierNewPS-BoldItalicMT", "Courier New", Some(700), true),
        ("CourierNewPSMT", "Courier New", None, false),
        (
            "TrumpGothicEast-Bold",
            "Trump Gothic East",
            Some(700),
            false,
        ),
        ("8-Bit-Madness", "8 Bit Madness", None, false),
        ("Monoid-Regular", "Monoid", Some(400), false),
        ("UpheavalTT-BRK-", "Upheaval TT BRK", None, false),
        (".SFNS-Semibold", "SFNS", Some(600), false),
        ("SFProDisplay-Bold", "SF Pro Display", Some(700), false),
        ("Futura-CondensedExtraBold", "Futura", Some(800), false),
        ("Futura-CondensedMedium", "Futura", Some(500), false),
        ("AvenirNext-DemiBold", "Avenir Next", Some(600), false),
        ("GillSans-SemiBold", "Gill Sans", Some(600), false),
        ("Avenir-Heavy", "Avenir", Some(900), false),
        ("Menlo-Regular", "Menlo", Some(400), false),
        ("ArialRoundedMTBold", "Arial Rounded MT Bold", None, false),
        // PDF
        ("ArialMT", "Arial", None, false),
        ("Arial-BoldMT", "Arial", Some(700), false),
        ("Arial-BoldItalicMT", "Arial", Some(700), true),
        (
            "TimesNewRomanPS-BoldMT",
            "Times New Roman",
            Some(700),
            false,
        ),
        (
            "BarlowCondensed-Regular",
            "Barlow Condensed",
            Some(400),
            false,
        ),
        ("Roboto-MediumItalic", "Roboto", Some(500), true),
        ("NotoSans-SemiBold", "Noto Sans", Some(600), false),
        ("MS-PGothic", "MS PGothic", None, false),
        (
            "NotoSansSymbols2-Regular",
            "Noto Sans Symbols 2",
            Some(400),
            false,
        ),
        ("Helvetica,Bold", "Helvetica", Some(700), false),
        ("Times-Roman", "Times", None, false),
        ("A-Bold", "A", Some(700), false),
    ];

    #[test]
    fn reads_family_weight_and_slant() {
        for &(name, family, weight, italic) in NAMES {
            let expected = PostscriptFace {
                family: family.to_owned(),
                weight,
                italic,
            };
            assert_eq!(postscript_face(name), expected, "{name}");
            assert_eq!(
                postscript_face(&format!("ABCDEF+{name}")),
                expected,
                "ABCDEF+{name}"
            );
        }
    }

    #[test]
    fn strips_only_a_six_capital_subset_tag() {
        assert_eq!(strip_subset_tag("ABCDEF+Roboto"), "Roboto");
        assert_eq!(strip_subset_tag("ABCDE+Roboto"), "ABCDE+Roboto");
        assert_eq!(strip_subset_tag("ABCDEf+Roboto"), "ABCDEf+Roboto");
        assert_eq!(strip_subset_tag("ABCDEF+"), "");
        assert_eq!(strip_subset_tag("ABCDEF"), "ABCDEF");
    }

    #[test]
    fn strips_the_longest_vendor_suffix_once() {
        assert_eq!(strip_vendor_suffix("ArialPSMT"), "Arial");
        assert_eq!(strip_vendor_suffix("ArialMTMT"), "ArialMT");
        assert_eq!(strip_vendor_suffix("PSPS"), "PS");
        assert_eq!(strip_vendor_suffix("Arialmt"), "Arialmt");
    }

    #[test]
    fn splits_stems_into_words() {
        assert_eq!(family_of("Open__Sans--Pro"), "Open Sans Pro");
        assert_eq!(family_of("NotoSansCJKjp"), "Noto Sans CJKjp");
        assert_eq!(family_of("Font2Go"), "Font 2Go");
        assert_eq!(family_of("_Lead"), "Lead");
    }

    #[test]
    fn a_suffix_of_other_words_is_part_of_the_family() {
        assert_eq!(
            postscript_face("Foo-BoldX"),
            PostscriptFace {
                family: "Foo Bold X".to_owned(),
                weight: None,
                italic: false
            }
        );
        assert_eq!(postscript_face("-Bold").family, "Bold");
    }
}
