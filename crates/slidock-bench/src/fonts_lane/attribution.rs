//! `fonts@1`: which face drew each glyph of a mounted slide, from `CSS.getPlatformFontsForNode` per text node.
//!
//! The lane measures the deck's text: slidock's own labels (a foreign object's marker, a failed container's code, set
//! in a generic font) are left out.
//!
//! A glyph drawn by a font that is not a web font is the renderer's fallback, and so is one drawn by the shadow face
//! (the `.notdef` of a family no shipped face has, for a character no font has). A web font is one of the deck's shipped
//! faces (the page has no other), found by its PostScript name (the subset keeps the file's names), else by the family
//! its file names (a variable face reports its instance's PostScript name, `CaveatRoman-Bold`), else as the face of
//! the first family of the stack that ships one; when several shipped faces fit, the one whose family comes first in
//! the text's font-family stack. The face draws the requested design (`exact`) unless
//! it is a bundled substitute standing in for another family (`substitute`). A glyph not drawn by the first family of
//! the stack that ships a face is a coverage gap: the bound face lacks its code point.
//!
//! The colour-emoji exception: a renderer counts glyphs, and an emoji sequence (a base with its VS16, skin-tone
//! modifier, tags or keycap, and `ZWJ`-joined pictographs) is one glyph from a face that shapes it but one box per
//! code point from a face that shapes nothing (Unicode's Last Resort, the container's only face, and the shadow face).
//! A sequence was boxed when every other face that drew the node is a shipped face whose file lacks the sequence's
//! base; its extra code points then come off the boxing face's count ([`boxed_extra`]). Any other face that drew the
//! node (a colour emoji font, any renderer font) may have shaped it, so the counts stay as drawn.

use std::collections::{BTreeMap, BTreeSet};

use slidock_bench_schema::lanes::{FontSource, FontsRowMetrics, GlyphCounts};
use slidock_fonts::CodepointSet;

use super::deck::font_families;
use crate::renderer::session::{PlatformFont, TextNodeFonts};
use crate::renderer::shadow_font::SHADOW_POSTSCRIPT_NAME;

/// A shipped face as the lane attributes glyphs to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneFace {
    /// The `@font-face` family.
    pub family: String,
    pub postscript_name: String,
    /// The family the file's own `name` table gives.
    pub file_family: String,
    pub source: FontSource,
    /// The emoji bases ([`PICTOGRAPHIC`]) the file maps.
    pub pictographs: CodepointSet,
}

/// Families compare as CSS matches them: case-insensitively, with runs of white space as one space.
pub fn family_key(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// `Math.round(v * 1e4) / 1e4 + 0`: the results grid.
fn quantise(value: f64) -> f64 {
    slidock_bench_metrics::quantise(value)
}

/// `part / whole` on the results grid, `None` over nothing.
pub fn share(part: u32, whole: u32) -> Option<f64> {
    (whole > 0).then(|| quantise(f64::from(part) / f64::from(whole)))
}

fn add_to(
    counts: &mut GlyphCounts,
    glyphs: u32,
    class: Class,
    gap: bool,
    source: Option<FontSource>,
) {
    counts.glyphs = counts.glyphs.saturating_add(glyphs);
    let bucket = match class {
        Class::Exact => &mut counts.exact,
        Class::Substitute => &mut counts.substitute,
        Class::Fallback => &mut counts.fallback,
    };
    *bucket = bucket.saturating_add(glyphs);
    if gap {
        counts.coverage_gaps = counts.coverage_gaps.saturating_add(glyphs);
    }
    if let Some(source) = source {
        let entry = counts.sources.entry(source).or_default();
        *entry = entry.saturating_add(glyphs);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Exact,
    Substitute,
    Fallback,
}

/// Code points that join the emoji before them: variation selectors, skin-tone modifiers, tags, the keycap.
fn joins_emoji(c: char) -> bool {
    matches!(c, '\u{FE0E}' | '\u{FE0F}' | '\u{20E3}' | '\u{1F3FB}'..='\u{1F3FF}' | '\u{E0020}'..='\u{E007F}')
}

fn regional_indicator(c: char) -> bool {
    matches!(c, '\u{1F1E6}'..='\u{1F1FF}')
}

/// Emoji bases: the blocks of Unicode's `Extended_Pictographic` property, less the regional indicators and skin-tone
/// modifiers (a superset is harmless here: a base alone is one code point either way).
pub const PICTOGRAPHIC: [(u32, u32); 20] = [
    (0xA9, 0xA9),
    (0xAE, 0xAE),
    (0x203C, 0x203C),
    (0x2049, 0x2049),
    (0x2122, 0x2122),
    (0x2139, 0x2139),
    (0x2194, 0x21AA),
    (0x231A, 0x23FF),
    (0x24C2, 0x24C2),
    (0x25AA, 0x25FE),
    (0x2600, 0x27BF),
    (0x2934, 0x2935),
    (0x2B05, 0x2B55),
    (0x3030, 0x3030),
    (0x303D, 0x303D),
    (0x3297, 0x3297),
    (0x3299, 0x3299),
    (0x1F000, 0x1F1E5),
    (0x1F200, 0x1F3FA),
    (0x1F400, 0x1FAFF),
];

fn pictographic(c: char) -> bool {
    let c = u32::from(c);
    PICTOGRAPHIC
        .iter()
        .any(|&(first, last)| (first..=last).contains(&c))
}

/// The emoji bases as a set, to ask a face's file which of them it maps.
pub fn pictographic_set() -> CodepointSet {
    let mut set = CodepointSet::new();
    for (first, last) in PICTOGRAPHIC {
        set.insert_range(first, last);
    }
    set
}

/// An emoji sequence: its base code point and its length in code points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sequence {
    base: u32,
    len: u32,
}

/// The emoji sequences of `text`.
fn emoji_sequences(text: &str) -> Vec<Sequence> {
    let chars: Vec<char> = text.chars().collect();
    let mut sequences = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        let keycap_base = (c.is_ascii_digit() || c == '#' || c == '*')
            && chars
                .get(i + 1)
                .is_some_and(|&n| n == '\u{FE0F}' || n == '\u{20E3}');
        let pair =
            regional_indicator(c) && chars.get(i + 1).copied().is_some_and(regional_indicator);
        if !(pictographic(c) || keycap_base || pair) {
            i += 1;
            continue;
        }
        let mut end = i + if pair { 2 } else { 1 };
        loop {
            match (chars.get(end), chars.get(end + 1)) {
                (Some(&n), _) if joins_emoji(n) => end += 1,
                (Some('\u{200D}'), Some(&n)) if pictographic(n) => end += 2,
                _ => break,
            }
        }
        sequences.push(Sequence {
            base: u32::from(c),
            len: u32::try_from(end - i).unwrap_or(u32::MAX),
        });
        i = end;
    }
    sequences
}

/// A face that draws one box per code point and shapes nothing: Unicode's Last Resort (`LastResort`,
/// `LastResortHE-Regular`) and the shadow face.
fn boxes(font: &PlatformFont) -> bool {
    font.postscript_name.starts_with("LastResort") || font.postscript_name == SHADOW_POSTSCRIPT_NAME
}

/// How a font that drew a node relates to its emoji sequences.
#[derive(Clone)]
enum Drew<'a> {
    /// A face that boxes each code point ([`boxes`]).
    Boxes,
    /// A shipped face: it shaped the sequences whose base its file maps.
    Shipped(&'a LaneFace),
    /// Any other face, which may have shaped any sequence.
    Other,
}

/// The glyphs to take off a node's boxing faces under the colour-emoji exception: each sequence they boxed counts once.
/// A sequence was boxed when no face that drew the node could shape it; nothing is taken when any face could have
/// shaped any sequence, or when the boxing faces drew fewer glyphs than the boxed sequences have code points.
fn boxed_extra(text: &str, drew: &[(u32, Drew<'_>)]) -> u32 {
    if drew.iter().any(|(_, kind)| matches!(kind, Drew::Other)) {
        return 0;
    }
    let boxed: Vec<Sequence> = emoji_sequences(text)
        .into_iter()
        .filter(|sequence| {
            !drew.iter().any(|(_, kind)| {
                matches!(kind, Drew::Shipped(face) if face.pictographs.contains(sequence.base))
            })
        })
        .collect();
    let code_points: u32 = boxed.iter().map(|s| s.len).fold(0, u32::saturating_add);
    let boxing: u32 = drew
        .iter()
        .filter(|(_, kind)| matches!(kind, Drew::Boxes))
        .map(|(glyphs, _)| *glyphs)
        .fold(0, u32::saturating_add);
    if boxing < code_points {
        return 0;
    }
    boxed
        .iter()
        .map(|s| s.len.saturating_sub(1))
        .fold(0, u32::saturating_add)
}

/// The row's `fonts@1` metrics from its text nodes and the deck's shipped faces.
pub fn attribute(nodes: &[TextNodeFonts], faces: &[LaneFace]) -> FontsRowMetrics {
    let shipped: BTreeSet<String> = faces.iter().map(|f| family_key(&f.family)).collect();
    let mut total = GlyphCounts::default();
    let mut families: BTreeMap<String, GlyphCounts> = BTreeMap::new();
    let mut fallback_fonts = BTreeSet::new();
    // slidock's own labels draw in a generic font of their own: the lane measures the deck's text.
    for node in nodes.iter().filter(|node| !node.own_label) {
        let stack = font_families(&node.family);
        let keys: Vec<String> = stack.iter().map(|f| family_key(f)).collect();
        let requested = stack.first().cloned().unwrap_or_default();
        let bound = keys.iter().find(|key| shipped.contains(*key));
        let mut drawn = Vec::new();
        for font in node.fonts.iter().filter(|font| font.glyphs > 0) {
            let rank = |face: &&LaneFace| {
                let key = family_key(&face.family);
                keys.iter().position(|k| *k == key).unwrap_or(usize::MAX)
            };
            let face = if font.custom && font.postscript_name != SHADOW_POSTSCRIPT_NAME {
                let by_name = |matches: &dyn Fn(&LaneFace) -> bool| {
                    faces.iter().filter(|face| matches(face)).min_by_key(rank)
                };
                by_name(&|face| face.postscript_name == font.postscript_name)
                    .or_else(|| {
                        by_name(&|face| family_key(&face.file_family) == family_key(&font.family))
                    })
                    .or_else(|| {
                        by_name(&|face| bound.is_some_and(|b| *b == family_key(&face.family)))
                    })
            } else {
                None
            };
            let (class, gap, source) = match face {
                None => {
                    fallback_fonts.insert(font.family.clone());
                    (Class::Fallback, bound.is_some(), None)
                }
                Some(face) => {
                    let key = family_key(&face.family);
                    let class = if face.source == FontSource::Substitute
                        && keys.first().is_some_and(|first| *first != key)
                    {
                        Class::Substitute
                    } else {
                        Class::Exact
                    };
                    (class, bound.is_some_and(|b| *b != key), Some(face.source))
                }
            };
            let kind = match face {
                Some(face) => Drew::Shipped(face),
                None if boxes(font) => Drew::Boxes,
                None => Drew::Other,
            };
            drawn.push((font.glyphs, kind, class, gap, source));
        }
        let kinds: Vec<(u32, Drew<'_>)> = drawn
            .iter()
            .map(|(glyphs, kind, ..)| (*glyphs, kind.clone()))
            .collect();
        let mut extra = boxed_extra(&node.text, &kinds);
        for (glyphs, kind, class, gap, source) in drawn {
            let glyphs = if matches!(kind, Drew::Boxes) {
                let less = extra.min(glyphs);
                extra -= less;
                glyphs - less
            } else {
                glyphs
            };
            add_to(&mut total, glyphs, class, gap, source);
            add_to(
                families.entry(requested.clone()).or_default(),
                glyphs,
                class,
                gap,
                source,
            );
        }
    }
    FontsRowMetrics {
        exact_share: share(total.exact, total.glyphs),
        substitute_share: share(total.substitute, total.glyphs),
        fallback_share: share(total.fallback, total.glyphs),
        total,
        fallback_fonts: fallback_fonts.into_iter().collect(),
        families,
        equivalence: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::session::PlatformFont;

    fn face(family: &str, postscript: &str, source: FontSource) -> LaneFace {
        LaneFace {
            family: family.into(),
            postscript_name: postscript.into(),
            file_family: family.into(),
            source,
            pictographs: CodepointSet::new(),
        }
    }

    fn node(family: &str, fonts: &[(&str, bool, u32)]) -> TextNodeFonts {
        TextNodeFonts {
            own_label: false,
            text: "text".into(),
            family: family.into(),
            fonts: fonts
                .iter()
                .map(|(postscript, custom, glyphs)| PlatformFont {
                    family: format!("{postscript} family"),
                    postscript_name: (*postscript).into(),
                    custom: *custom,
                    glyphs: *glyphs,
                })
                .collect(),
        }
    }

    fn faces() -> Vec<LaneFace> {
        vec![
            face("Roboto", "Roboto-Regular", FontSource::Embedded),
            face("Oswald", "Oswald-Regular", FontSource::Google),
            face("Liberation Sans", "LiberationSans", FontSource::Substitute),
        ]
    }

    #[test]
    fn the_requested_design_is_exact_and_a_stand_in_is_a_substitute() {
        let metrics = attribute(
            &[
                node("Roboto, sans-serif", &[("Roboto-Regular", true, 10)]),
                // Oswald Medium binds to Oswald: the same design, from Google.
                node(
                    "\"Oswald Medium\", Oswald, sans-serif",
                    &[("Oswald-Regular", true, 4)],
                ),
                // Arial's substitute.
                node(
                    "Arial, \"Liberation Sans\", sans-serif",
                    &[("LiberationSans", true, 6)],
                ),
                // Liberation Sans asked for by name is the requested design.
                node(
                    "\"Liberation Sans\", sans-serif",
                    &[("LiberationSans", true, 3)],
                ),
            ],
            &faces(),
        );
        let t = &metrics.total;
        assert_eq!(
            (t.glyphs, t.exact, t.substitute, t.fallback, t.coverage_gaps),
            (23, 17, 6, 0, 0)
        );
        assert_eq!(t.sources.get(&FontSource::Substitute), Some(&9));
        assert_eq!(t.sources.get(&FontSource::Google), Some(&4));
        assert_eq!(metrics.exact_share, Some(0.7391));
        assert_eq!(metrics.families["Arial"].substitute, 6);
        assert_eq!(metrics.families["Oswald Medium"].exact, 4);
        assert!(metrics.fallback_fonts.is_empty());
    }

    #[test]
    fn slidocks_own_labels_are_not_the_decks_text() {
        // "Image not included" in the marker's `9pt sans-serif`, drawn by the renderer's Last Resort.
        let marker = TextNodeFonts {
            own_label: true,
            ..node("sans-serif", &[("LastResortHE-Regular", false, 18)])
        };
        let metrics = attribute(
            &[
                node("Roboto, sans-serif", &[("Roboto-Regular", true, 8)]),
                marker,
            ],
            &faces(),
        );
        let t = &metrics.total;
        assert_eq!((t.glyphs, t.exact, t.fallback), (8, 8, 0));
        assert!(metrics.fallback_fonts.is_empty());
        assert!(!metrics.families.contains_key("sans-serif"));
    }

    #[test]
    fn a_renderer_font_is_fallback_and_a_gap_only_under_a_shipped_family() {
        let metrics = attribute(
            &[
                node(
                    "Roboto, sans-serif",
                    &[
                        ("Roboto-Regular", true, 8),
                        ("LastResortHE-Regular", false, 2),
                    ],
                ),
                // No family of the stack ships a face: everything is the renderer's, but no face lacks a code point.
                node(
                    "Wingdings, sans-serif",
                    &[("LastResortHE-Regular", false, 5)],
                ),
                // A shipped substitute drawing what the bound face lacks.
                node(
                    "Roboto, \"Liberation Sans\", sans-serif",
                    &[("Roboto-Regular", true, 3), ("LiberationSans", true, 1)],
                ),
            ],
            &faces(),
        );
        let t = &metrics.total;
        assert_eq!(
            (t.glyphs, t.exact, t.substitute, t.fallback, t.coverage_gaps),
            (19, 11, 1, 7, 3)
        );
        assert_eq!(metrics.fallback_fonts, ["LastResortHE-Regular family"]);
        assert_eq!(metrics.families["Wingdings"].coverage_gaps, 0);
        assert_eq!(metrics.families["Roboto"].coverage_gaps, 3);
    }

    #[test]
    fn a_shared_postscript_name_goes_to_the_family_first_in_the_stack() {
        let faces = vec![
            face("Roboto", "Roboto-Regular", FontSource::Google),
            face("Roboto Medium", "Roboto-Regular", FontSource::Embedded),
        ];
        let metrics = attribute(
            &[node(
                "\"Roboto Medium\", Roboto, sans-serif",
                &[("Roboto-Regular", true, 5)],
            )],
            &faces,
        );
        assert_eq!(metrics.total.sources.get(&FontSource::Embedded), Some(&5));
        assert_eq!(metrics.total.coverage_gaps, 0);
    }

    #[test]
    fn the_shadow_faces_notdef_is_fallback() {
        let metrics = attribute(
            &[node(
                "Arial, \"Liberation Sans\", sans-serif",
                &[(SHADOW_POSTSCRIPT_NAME, true, 2)],
            )],
            &faces(),
        );
        assert_eq!(
            (metrics.total.fallback, metrics.total.coverage_gaps),
            (2, 2)
        );
    }

    #[test]
    fn a_variable_instance_is_its_files_face() {
        let faces = vec![face("Caveat", "Caveat-Regular", FontSource::Google)];
        let mut bold = node("Caveat, cursive", &[("CaveatRoman-Bold", true, 7)]);
        bold.fonts[0].family = "Caveat".into();
        let metrics = attribute(&[bold], &faces);
        assert_eq!((metrics.total.exact, metrics.total.fallback), (7, 0));
        // An instance is its file's face even past the bound family.
        let faces = vec![
            face("Liberation Sans", "LiberationSans", FontSource::Substitute),
            face("Caveat", "Caveat-Regular", FontSource::Google),
        ];
        let mut past = node(
            "Arial, \"Liberation Sans\", Caveat",
            &[("CaveatRoman-Bold", true, 3)],
        );
        past.fonts[0].family = "Caveat".into();
        let metrics = attribute(&[past], &faces);
        assert_eq!(metrics.total.sources.get(&FontSource::Google), Some(&3));
        assert_eq!(metrics.total.coverage_gaps, 3);
        // A web font no shipped face names is still the deck's: the bound family's.
        let unnamed = node("Caveat, cursive", &[("Unknown-Instance", true, 2)]);
        assert_eq!(attribute(&[unnamed], &faces).total.exact, 2);
    }

    #[test]
    fn an_emoji_sequence_is_one_fallback_glyph_whoever_draws_it() {
        // 🙋 🙋‍♀️ 🙋‍♂️: three sequences, nine code points.
        let text =
            "Hello? \u{1F64B}\u{1F64B}\u{200D}\u{2640}\u{FE0F}\u{1F64B}\u{200D}\u{2642}\u{FE0F}";
        assert_eq!(lens(text), [1, 4, 4]);
        let mut last_resort = node(
            "Arial, \"Liberation Sans\", sans-serif",
            &[
                ("LiberationSans", true, 6),
                ("LastResortHE-Regular", false, 9),
            ],
        );
        last_resort.text = text.into();
        let mut colour = last_resort.clone();
        colour.fonts[1] = colour_emoji(3);
        for row in [&last_resort, &colour] {
            let t = attribute(std::slice::from_ref(row), &faces()).total;
            assert_eq!(
                (t.glyphs, t.substitute, t.fallback, t.coverage_gaps),
                (9, 6, 3, 3)
            );
        }
        // Last Resort's other boxes still count one by one.
        last_resort.fonts[1].glyphs = 11;
        assert_eq!(attribute(&[last_resort], &faces()).total.fallback, 5);
    }

    fn colour_emoji(glyphs: u32) -> PlatformFont {
        PlatformFont {
            family: "Apple Color Emoji".into(),
            postscript_name: "AppleColorEmoji".into(),
            custom: false,
            glyphs,
        }
    }

    fn lens(text: &str) -> Vec<u32> {
        emoji_sequences(text).iter().map(|s| s.len).collect()
    }

    #[test]
    fn only_sequences_no_face_could_shape_are_counted_once() {
        // 🙋‍♀️ and five Tibetan letters no font has.
        let text = "\u{1F64B}\u{200D}\u{2640}\u{FE0F} \u{0F00}\u{0F01}\u{0F02}\u{0F03}\u{0F04}";
        let fallback = |fonts: Vec<PlatformFont>, faces: &[LaneFace]| {
            let row = TextNodeFonts {
                own_label: false,
                text: text.into(),
                family: "Roboto, sans-serif".into(),
                fonts,
            };
            attribute(&[row], faces).total.fallback
        };
        let last_resort = |glyphs| PlatformFont {
            family: "Last Resort".into(),
            postscript_name: "LastResort".into(),
            custom: false,
            glyphs,
        };
        let roboto = |glyphs| PlatformFont {
            family: "Roboto".into(),
            postscript_name: "Roboto-Regular".into(),
            custom: true,
            glyphs,
        };
        // A colour emoji font shaped the sequence: the five boxes are real gaps, and its glyph counts.
        assert_eq!(fallback(vec![colour_emoji(1), last_resort(5)], &faces()), 6);
        // A shipped face that maps the base shaped it.
        let mut covering = faces();
        covering[0].pictographs.insert(0x1F64B);
        assert_eq!(fallback(vec![roboto(2), last_resort(5)], &covering), 5);
        // No face could shape it: Last Resort boxed its four code points.
        assert_eq!(fallback(vec![roboto(1), last_resort(9)], &faces()), 6);
        // Fewer boxes than the sequence has code points: Last Resort did not box it.
        assert_eq!(fallback(vec![roboto(1), last_resort(3)], &faces()), 3);
        // Any renderer font may have shaped it.
        let symbols = PlatformFont {
            family: "Noto Sans Symbols".into(),
            postscript_name: "NotoSansSymbols-Regular".into(),
            custom: false,
            glyphs: 1,
        };
        assert_eq!(fallback(vec![symbols, last_resort(8)], &faces()), 9);
    }

    #[test]
    fn emoji_sequences_join_only_what_belongs_to_them() {
        // A flag, a keycap, a skin tone, a tag sequence (England), a text-style heart.
        assert_eq!(lens("\u{1F1E7}\u{1F1F7}"), [2]);
        assert_eq!(lens("1\u{FE0F}\u{20E3} 2"), [3]);
        assert_eq!(lens("\u{1F44D}\u{1F3FD}"), [2]);
        assert_eq!(
            lens("\u{1F3F4}\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}"),
            [7]
        );
        assert_eq!(lens("\u{2764}\u{FE0E}"), [2]);
        assert_eq!(emoji_sequences("\u{1F1E7}\u{1F1F7}")[0].base, 0x1F1E7);
        // A ZWJ before a letter joins nothing; plain text has no sequence.
        assert_eq!(lens("\u{1F600}\u{200D}a"), [1]);
        assert_eq!(lens("plain \u{2756} text"), [1]);
        assert!(lens("plain text 123").is_empty());
        // A lone skin tone or regional indicator is no base.
        assert!(lens("\u{1F3FD}").is_empty() && lens("\u{1F1E7}").is_empty());
        assert!(!pictographic_set().contains(0x1F1E7) && pictographic_set().contains(0x1F64B));
    }

    #[test]
    fn a_row_without_glyphs_has_no_shares() {
        let metrics = attribute(&[node("Roboto", &[])], &faces());
        assert_eq!(metrics.total, GlyphCounts::default());
        assert_eq!(metrics.exact_share, None);
    }
}
