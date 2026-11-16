//! Emoji fonts never ship: a family that is one binds to the viewer's emoji font, through the stack of the platforms'
//! emoji families and the CSS `emoji` generic ([`GenericFamily::Emoji`]). Their files are huge (Apple Color Emoji is
//! 183 MiB of bitmaps) and every viewer has one.
//!
//! The text is measured with one face standing for every platform's: the metrics of Noto Color Emoji 2.047 (OFL,
//! Google Fonts' v50 file; numbers only), whose glyphs all advance by the same width. A cluster draws one glyph of
//! that width: a zero-width joiner joins the next character into its cluster, and variation selectors, skin-tone
//! modifiers after a character, the combining keycap, tag characters and the second regional indicator of a flag add
//! none.

use std::sync::OnceLock;

use slidock_ir::fonts::{FaceStyle, FontMetrics, GenericFamily, GlyphBounds, VMetrics, WinMetrics};

use super::{CatalogFace, CatalogFamily, FamilyKey};
use crate::bind::NORMAL_WIDTH;

/// Keys of the colour emoji families: Apple's, Microsoft's, Google's, Android's older one, EmojiOne's and Firefox's
/// Twemoji.
pub const EMOJI_FAMILIES: &[&str] = &[
    "applecoloremoji",
    "segoeuiemoji",
    "notocoloremoji",
    "androidemoji",
    "emojionecolor",
    "twemojimozilla",
];

/// The family an emoji family binds to, as the IR's binding names it.
pub const PLATFORM_EMOJI: &str = "emoji";

const UNITS_PER_EM: u32 = 1024;
/// Every emoji glyph's advance, in font units.
const ADVANCE: u32 = 1275;

/// Whether a family is one of the [`EMOJI_FAMILIES`].
pub fn is_emoji_family(key: &FamilyKey) -> bool {
    EMOJI_FAMILIES.contains(&key.as_str())
}

/// The metrics-only family every emoji family binds to: one upright face at 400, which CSS matching picks for every
/// weight and style.
pub fn platform_emoji() -> &'static CatalogFamily {
    static FAMILY: OnceLock<CatalogFamily> = OnceLock::new();
    FAMILY.get_or_init(|| CatalogFamily {
        family: PLATFORM_EMOJI.to_owned(),
        generic: GenericFamily::Emoji,
        faces: vec![CatalogFace {
            weight: 400,
            width: NORMAL_WIDTH,
            style: FaceStyle::Normal,
            postscript_name: PLATFORM_EMOJI.to_owned(),
            metrics: FontMetrics {
                units_per_em: UNITS_PER_EM,
                hhea: VMetrics {
                    ascender: 950,
                    descender: -250,
                    line_gap: 0,
                },
                typo: VMetrics {
                    ascender: 950,
                    descender: -250,
                    line_gap: 0,
                },
                win: WinMetrics {
                    ascent: 950,
                    descent: 250,
                },
                bounds: Some(GlyphBounds {
                    y_min: -250,
                    y_max: 952,
                }),
                grave_top: None,
            },
            variable: false,
            monospace: false,
            sha256: None,
            variation: None,
            file: None,
        }],
    })
}

/// Adds nothing to the cluster before it.
fn extends_cluster(ch: char) -> bool {
    matches!(u32::from(ch),
        0xfe00..=0xfe0f | 0x20e3 | 0x1f3fb..=0x1f3ff | 0xe0020..=0xe007f)
}

fn is_regional_indicator(ch: char) -> bool {
    matches!(u32::from(ch), 0x1f1e6..=0x1f1ff)
}

/// The advance of each character of `text` in ems as the platform emoji face draws it: a cluster's first character
/// takes the face's advance, the rest none.
pub fn emoji_advances(text: &str) -> Vec<f64> {
    let advance = f64::from(ADVANCE) / f64::from(UNITS_PER_EM);
    let mut out = Vec::new();
    let mut joined = false;
    let mut open_flag = false;
    let mut started = false;
    for ch in text.chars() {
        let width = if ch == '\u{200d}' {
            joined = true;
            0.0
        } else if joined {
            joined = false;
            0.0
        } else if started && extends_cluster(ch) {
            0.0
        } else if is_regional_indicator(ch) && open_flag {
            open_flag = false;
            0.0
        } else {
            open_flag = is_regional_indicator(ch);
            advance
        };
        started = true;
        out.push(width);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_families_by_font_key() {
        for family in [
            "Apple Color Emoji",
            "AppleColorEmoji",
            "Segoe UI Emoji",
            "Noto Color Emoji",
            "Android Emoji",
            "EmojiOne Color",
            "Twemoji Mozilla",
        ] {
            assert!(is_emoji_family(&FamilyKey::of(family)), "{family}");
        }
        // Negative controls: a symbol font and monochrome text fonts are no colour emoji fonts.
        for family in [
            "Segoe UI Symbol",
            "Apple Symbols",
            "Noto Sans Symbols 2",
            "Arial",
        ] {
            assert!(!is_emoji_family(&FamilyKey::of(family)), "{family}");
        }
        for &key in EMOJI_FAMILIES {
            assert_eq!(FamilyKey::of(key).as_str(), key);
        }
    }

    #[test]
    fn a_cluster_advances_once() {
        let em = f64::from(ADVANCE) / f64::from(UNITS_PER_EM);
        assert_eq!(emoji_advances("😀"), [em]);
        // ❤️: the heart and its emoji presentation selector.
        assert_eq!(emoji_advances("\u{2764}\u{fe0f}"), [em, 0.0]);
        // 👍🏽: a skin tone modifies the thumb.
        assert_eq!(emoji_advances("\u{1f44d}\u{1f3fd}"), [em, 0.0]);
        // 👩‍💻: joined by a zero-width joiner.
        assert_eq!(emoji_advances("\u{1f469}\u{200d}\u{1f4bb}"), [em, 0.0, 0.0]);
        // 🇧🇷🇺🇸: two flags of two regional indicators each.
        assert_eq!(
            emoji_advances("\u{1f1e7}\u{1f1f7}\u{1f1fa}\u{1f1f8}"),
            [em, 0.0, em, 0.0]
        );
        // 1️⃣: a keycap.
        assert_eq!(emoji_advances("1\u{fe0f}\u{20e3}"), [em, 0.0, 0.0]);
        // Negative control: emoji side by side advance each.
        assert_eq!(emoji_advances("😀😀"), [em, em]);
        // A modifier opening the text draws its own swatch.
        assert_eq!(emoji_advances("\u{1f3fd}"), [em]);
    }

    #[test]
    fn the_platform_face_is_the_emoji_generic() {
        let family = platform_emoji();
        assert_eq!(family.generic, GenericFamily::Emoji);
        assert_eq!(
            family.css_match(700, FaceStyle::Italic).map(|f| f.weight),
            Some(400)
        );
    }
}
