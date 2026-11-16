//! The `@font-face` rules of `<style id="sd-fonts">`: one per shipped file and weight range, with descriptors that
//! state the file's own weight (its `usWeightClass`, or its `wght` axis range) and style, never the weight a run
//! asked for, so the browser picks the faces the binder picked and synthesizes bold exactly where they lack it.
//!
//! The rule text matches what `slidock-bundle` writes: `font-family`, a `data:` `src` with its format, then
//! `font-weight`, `font-style`, `unicode-range` and `font-display`, with no spaces between declarations.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use slidock_canon::utf16;
use slidock_format::escape::css_string;
use slidock_ir::fonts::FaceStyle;

use crate::bind::{FaceDescriptor, WeightRange};
use crate::glyphs::CodepointSet;
use crate::sfnt::Outlines;

/// The format of a shipped file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FontFormat {
    Woff2,
    /// A raw TrueType file, shipped when WOFF2 encoding failed.
    TrueType,
    /// A raw CFF OpenType file, shipped when WOFF2 encoding failed.
    OpenType,
}

impl FontFormat {
    /// The raw format of a face with these outlines.
    pub fn raw(outlines: Outlines) -> FontFormat {
        match outlines {
            Outlines::TrueType => FontFormat::TrueType,
            Outlines::Cff => FontFormat::OpenType,
        }
    }

    pub const fn mime(self) -> &'static str {
        match self {
            FontFormat::Woff2 => "font/woff2",
            FontFormat::TrueType => "font/ttf",
            FontFormat::OpenType => "font/otf",
        }
    }

    /// The `format()` hint of `src`.
    pub const fn css(self) -> &'static str {
        match self {
            FontFormat::Woff2 => "woff2",
            FontFormat::TrueType => "truetype",
            FontFormat::OpenType => "opentype",
        }
    }

    /// The file extension, for packagings that write fonts as files.
    pub const fn extension(self) -> &'static str {
        match self {
            FontFormat::Woff2 => "woff2",
            FontFormat::TrueType => "ttf",
            FontFormat::OpenType => "otf",
        }
    }
}

/// `font-display`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FontDisplay {
    /// Never draw with a fallback of other metrics; the player waits for the faces before it reveals a slide.
    #[default]
    Block,
    Swap,
    Fallback,
    Optional,
    Auto,
}

impl FontDisplay {
    pub const fn as_str(self) -> &'static str {
        match self {
            FontDisplay::Block => "block",
            FontDisplay::Swap => "swap",
            FontDisplay::Fallback => "fallback",
            FontDisplay::Optional => "optional",
            FontDisplay::Auto => "auto",
        }
    }
}

/// One `@font-face` rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFaceRule {
    /// The CSS family: the family the binding ships, as the catalog spells it.
    pub family: String,
    /// The file's own weight (or the part of its axis range this rule claims) and style.
    pub descriptor: FaceDescriptor,
    /// The code points the file maps, printed as `unicode-range`; none prints no descriptor.
    pub unicode_range: Option<CodepointSet>,
    pub format: FontFormat,
    pub display: FontDisplay,
    pub bytes: Vec<u8>,
}

pub(crate) fn style_name(style: FaceStyle) -> &'static str {
    match style {
        FaceStyle::Normal => "normal",
        FaceStyle::Italic => "italic",
    }
}

/// `font-weight`: one number, or `min max` for a range.
pub fn weight_value(weight: WeightRange) -> String {
    if weight.min == weight.max {
        weight.min.to_string()
    } else {
        format!("{} {}", weight.min, weight.max)
    }
}

impl FontFaceRule {
    /// The rule's text.
    pub fn css(&self) -> String {
        let range = self
            .unicode_range
            .as_ref()
            .and_then(CodepointSet::unicode_range)
            .map(|range| format!(";unicode-range:{range}"))
            .unwrap_or_default();
        format!(
            "@font-face{{font-family:{};src:url(data:{};base64,{}) format(\"{}\");font-weight:{};font-style:{}{};\
             font-display:{}}}",
            css_string(&self.family),
            self.format.mime(),
            STANDARD.encode(&self.bytes),
            self.format.css(),
            weight_value(self.descriptor.weight),
            style_name(self.descriptor.style),
            range,
            self.display.as_str(),
        )
    }
}

fn style_rank(style: FaceStyle) -> u8 {
    match style {
        FaceStyle::Normal => 0,
        FaceStyle::Italic => 1,
    }
}

/// The order rules print in: family (by UTF-16 code unit), lightest weight, style (normal first), first code
/// point, then the bytes themselves, so equal inputs print equal stylesheets whatever order they came in.
pub fn rule_order(a: &FontFaceRule, b: &FontFaceRule) -> Ordering {
    utf16::cmp(&a.family, &b.family)
        .then(a.descriptor.weight.min.cmp(&b.descriptor.weight.min))
        .then(a.descriptor.weight.max.cmp(&b.descriptor.weight.max))
        .then(style_rank(a.descriptor.style).cmp(&style_rank(b.descriptor.style)))
        .then(
            a.unicode_range
                .as_ref()
                .and_then(CodepointSet::first)
                .cmp(&b.unicode_range.as_ref().and_then(CodepointSet::first)),
        )
        .then(a.format.cmp(&b.format))
        .then_with(|| a.bytes.cmp(&b.bytes))
}

/// The rules' texts in [`rule_order`]: what `<style id="sd-fonts">` holds, one rule per line.
pub fn font_rules(rules: &[FontFaceRule]) -> Vec<String> {
    let mut sorted: Vec<&FontFaceRule> = rules.iter().collect();
    sorted.sort_by(|a, b| rule_order(a, b));
    sorted.into_iter().map(FontFaceRule::css).collect()
}

/// A face the emitter ships for a family: its true descriptor and the weights it renders (the drawn weights whose
/// CSS match it is).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmittedFace {
    pub descriptor: FaceDescriptor,
    pub wins: BTreeSet<u32>,
}

/// The weight ranges each face's rules claim, so no weight lies in two rules of the same family and style. A
/// static face claims its weight. A variable face claims its axis range minus the weights of the static faces
/// shipped beside it, keeping for each weight it renders the piece nearest that weight (the piece holding it, or
/// for a weight outside its axis the piece the browser's matching reaches first), so its bytes are usually written
/// once; a variable face that renders no weight keeps every piece. A variable face whose whole axis the statics
/// claim keeps none.
pub fn claimed_ranges(faces: &[EmittedFace]) -> Vec<Vec<WeightRange>> {
    faces
        .iter()
        .map(|face| {
            let range = face.descriptor.weight;
            if range.min == range.max {
                return vec![range];
            }
            let mut cuts: Vec<u32> = faces
                .iter()
                .filter(|other| other.descriptor.style == face.descriptor.style)
                .map(|other| other.descriptor.weight)
                .filter(|other| {
                    other.min == other.max && range.min <= other.min && other.min <= range.max
                })
                .map(|other| other.min)
                .collect();
            cuts.sort_unstable();
            cuts.dedup();
            let mut pieces = Vec::new();
            let mut from = range.min;
            for cut in cuts {
                if cut > from {
                    pieces.push(WeightRange::between(from, cut - 1));
                }
                from = cut + 1;
            }
            if from <= range.max {
                pieces.push(WeightRange::between(from, range.max));
            }
            if face.wins.is_empty() {
                return pieces;
            }
            let distance = |piece: &WeightRange, weight: u32| {
                piece
                    .min
                    .saturating_sub(weight)
                    .max(weight.saturating_sub(piece.max))
            };
            let mut used = vec![false; pieces.len()];
            for &weight in &face.wins {
                let nearest = pieces
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, piece)| distance(piece, weight))
                    .map(|(i, _)| i);
                if let Some(slot) = nearest.and_then(|i| used.get_mut(i)) {
                    *slot = true;
                }
            }
            pieces
                .into_iter()
                .zip(used)
                .filter_map(|(piece, used)| used.then_some(piece))
                .collect()
        })
        .collect()
}

/// The rules of the faces a deck ships, each with the drawn weights it renders, so that no weight falls in two
/// rules of one family and style ([`claimed_ranges`]): a static face keeps its rule, a variable face gets one rule per
/// range it claims. Families are told apart by key ([`FamilyKey`](crate::FamilyKey)).
pub fn claim_rules(faces: Vec<(FontFaceRule, BTreeSet<u32>)>) -> Vec<FontFaceRule> {
    type Members = Vec<(FontFaceRule, BTreeSet<u32>)>;
    let mut families: Vec<(crate::FamilyKey, Members)> = Vec::new();
    for (rule, wins) in faces {
        let key = crate::FamilyKey::of(&rule.family);
        match families.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push((rule, wins)),
            None => families.push((key, vec![(rule, wins)])),
        }
    }
    let mut out = Vec::new();
    for (_, members) in families {
        let emitted: Vec<EmittedFace> = members
            .iter()
            .map(|(rule, wins)| EmittedFace {
                descriptor: rule.descriptor,
                wins: wins.clone(),
            })
            .collect();
        for ((rule, _), ranges) in members.into_iter().zip(claimed_ranges(&emitted)) {
            for weight in ranges {
                out.push(FontFaceRule {
                    descriptor: FaceDescriptor {
                        weight,
                        style: rule.descriptor.style,
                    },
                    ..rule.clone()
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(family: &str, weight: WeightRange, style: FaceStyle) -> FontFaceRule {
        FontFaceRule {
            family: family.to_owned(),
            descriptor: FaceDescriptor { weight, style },
            unicode_range: None,
            format: FontFormat::Woff2,
            display: FontDisplay::Block,
            bytes: b"wOF2".to_vec(),
        }
    }

    #[test]
    fn prints_the_bundle_shape() {
        let mut r = rule("Ro\"bo", WeightRange::point(500), FaceStyle::Normal);
        assert_eq!(
            r.css(),
            "@font-face{font-family:\"Ro\\22 bo\";src:url(data:font/woff2;base64,d09GMg==) format(\"woff2\");\
             font-weight:500;font-style:normal;font-display:block}"
        );
        r.descriptor = FaceDescriptor {
            weight: WeightRange::between(200, 700),
            style: FaceStyle::Italic,
        };
        r.format = FontFormat::TrueType;
        r.unicode_range = Some([0x41, 0x42, 0x20].into_iter().collect());
        r.display = FontDisplay::Swap;
        assert_eq!(
            r.css(),
            "@font-face{font-family:\"Ro\\22 bo\";src:url(data:font/ttf;base64,d09GMg==) format(\"truetype\");\
             font-weight:200 700;font-style:italic;unicode-range:U+20,U+41-42;font-display:swap}"
        );
    }

    #[test]
    fn orders_rules_whatever_the_input_order() {
        let rules = vec![
            rule("b", WeightRange::point(400), FaceStyle::Normal),
            rule("a", WeightRange::point(700), FaceStyle::Normal),
            rule("a", WeightRange::point(400), FaceStyle::Italic),
            rule("a", WeightRange::point(400), FaceStyle::Normal),
        ];
        let mut reversed = rules.clone();
        reversed.reverse();
        let printed = font_rules(&rules);
        assert_eq!(printed, font_rules(&reversed));
        let heads: Vec<&str> = printed
            .iter()
            .map(|r| &r[r.find("font-weight").unwrap()..r.find(";font-display").unwrap()])
            .collect();
        assert_eq!(
            heads,
            [
                "font-weight:400;font-style:normal",
                "font-weight:400;font-style:italic",
                "font-weight:700;font-style:normal",
                "font-weight:400;font-style:normal",
            ]
        );
    }

    fn emitted(weight: WeightRange, wins: &[u32]) -> EmittedFace {
        EmittedFace {
            descriptor: FaceDescriptor {
                weight,
                style: FaceStyle::Normal,
            },
            wins: wins.iter().copied().collect(),
        }
    }

    #[test]
    fn claimed_rules_split_a_variable_face_only_within_its_family() {
        let variable = rule("Oswald", WeightRange::between(200, 700), FaceStyle::Normal);
        let light = rule("Oswald", WeightRange::point(300), FaceStyle::Normal);
        let other = rule("Inter", WeightRange::point(300), FaceStyle::Normal);
        let rules = claim_rules(vec![
            (variable.clone(), BTreeSet::from([500])),
            (light.clone(), BTreeSet::from([300])),
            (other.clone(), BTreeSet::from([300])),
        ]);
        let weights: Vec<(&str, WeightRange)> = rules
            .iter()
            .map(|r| (r.family.as_str(), r.descriptor.weight))
            .collect();
        assert_eq!(
            weights,
            [
                ("Oswald", WeightRange::between(301, 700)),
                ("Oswald", WeightRange::point(300)),
                ("Inter", WeightRange::point(300)),
            ]
        );
        // Without the static beside it, the variable face keeps its whole axis.
        let alone = claim_rules(vec![(variable, BTreeSet::from([500]))]);
        assert_eq!(alone[0].descriptor.weight, WeightRange::between(200, 700));
    }

    #[test]
    fn a_variable_range_steps_around_the_statics_shipped_beside_it() {
        let faces = [
            emitted(WeightRange::point(300), &[300]),
            emitted(WeightRange::between(200, 700), &[500, 700]),
        ];
        assert_eq!(
            claimed_ranges(&faces),
            [
                vec![WeightRange::point(300)],
                vec![WeightRange::between(301, 700)]
            ]
        );
        let both_sides = [
            emitted(WeightRange::point(400), &[400]),
            emitted(WeightRange::between(100, 900), &[200, 700]),
        ];
        assert_eq!(
            claimed_ranges(&both_sides)[1],
            [
                WeightRange::between(100, 399),
                WeightRange::between(401, 900)
            ]
        );
        let alone = [emitted(WeightRange::between(200, 700), &[])];
        assert_eq!(
            claimed_ranges(&alone),
            [vec![WeightRange::between(200, 700)]]
        );
    }

    /// No weight lies in two claimed ranges of one style.
    fn disjoint(claimed: &[Vec<WeightRange>]) -> bool {
        let mut all: Vec<WeightRange> = claimed.iter().flatten().copied().collect();
        all.sort_by_key(|range| range.min);
        all.windows(2).all(|pair| pair[0].max < pair[1].min)
    }

    #[test]
    fn a_variable_face_rendering_weights_outside_its_axis_never_claims_a_static_weight() {
        // Static 400 beside a variable 400–600: the binder draws 700 with the variable face. It claims the piece
        // nearest 700, which the browser's matching reaches first for 700, not its whole axis.
        let faces = [
            emitted(WeightRange::point(400), &[400]),
            emitted(WeightRange::between(400, 600), &[700]),
        ];
        let claimed = claimed_ranges(&faces);
        assert_eq!(
            claimed,
            [
                vec![WeightRange::point(400)],
                vec![WeightRange::between(401, 600)]
            ]
        );
        assert!(disjoint(&claimed));
        let below = [
            emitted(WeightRange::point(600), &[600]),
            emitted(WeightRange::between(400, 900), &[350]),
        ];
        let claimed = claimed_ranges(&below);
        assert_eq!(claimed[1], [WeightRange::between(400, 599)]);
        assert!(disjoint(&claimed));
        // A variable face whose whole axis the statics claim claims nothing.
        let covered = [
            emitted(WeightRange::point(400), &[400]),
            emitted(WeightRange::point(401), &[401]),
            emitted(WeightRange::between(400, 401), &[700]),
        ];
        assert_eq!(claimed_ranges(&covered)[2], []);
    }
}
