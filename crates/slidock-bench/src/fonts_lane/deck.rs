//! What a bundled deck ships and asks for, read back from its files: the faces of `<style id="sd-fonts">` (each
//! WOFF2 decoded for its PostScript name and the emoji bases it maps), where each came from (`LICENSES.txt`), the faces its text draws and their
//! sources (the deck data's `fonts`), its shown slides, its size, and its font diagnostics.

use std::collections::BTreeMap;
use std::path::Path;

use base64::Engine;
use serde::Deserialize;
use slidock_bench_schema::Sha256;
use slidock_bench_schema::lanes::FontStyle;
use slidock_fonts::{CodepointSet, FontOrigin};

use super::attribution;
use crate::error::{BenchError, Result};

/// A face of `<style id="sd-fonts">`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeckFace {
    /// The `@font-face` family.
    pub family: String,
    /// The `font-weight` descriptor as written: `400`, or `100 900`.
    pub weight: String,
    pub style: FontStyle,
    pub postscript_name: String,
    /// The family the file's own `name` table gives.
    pub file_family: String,
    pub sha256: Sha256,
    pub bytes: u64,
    /// From the face's `LICENSES.txt` entry; `None` when no entry matches.
    pub origin: Option<FontOrigin>,
    /// The emoji bases the file maps.
    pub pictographs: CodepointSet,
}

/// Where a face the deck draws takes its file from, as the deck data says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeckSource {
    Embedded,
    Local,
    Google,
    User,
    Bundled,
    /// No file ships for it.
    System,
    /// No file ships for it: an emoji family, which the viewer's emoji font draws.
    Emoji,
}

/// A face the deck's text draws.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct DeckRequest {
    pub family: String,
    pub weight: u32,
    pub style: String,
    pub source: DeckSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct DeckSlide {
    pub id: String,
    pub hidden: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct DeckSize {
    pub w: f64,
    pub h: f64,
}

#[derive(Deserialize)]
struct CodeCount {
    code: String,
    count: u32,
}

#[derive(Deserialize)]
struct Diagnostics {
    codes: Vec<CodeCount>,
}

#[derive(Deserialize)]
struct DeckData {
    size: DeckSize,
    slides: Vec<DeckSlide>,
    fonts: Vec<DeckRequest>,
    diagnostics: Diagnostics,
}

/// What a bundled deck's files say about its fonts.
#[derive(Clone, Debug, PartialEq)]
pub struct BundleFonts {
    pub faces: Vec<DeckFace>,
    pub requested: Vec<DeckRequest>,
    pub slides: Vec<DeckSlide>,
    /// In points.
    pub size: DeckSize,
    /// The bytes of `<style id="sd-fonts">`'s text.
    pub css_bytes: u64,
    /// The deck data's diagnostic counts of the font codes (MID54xx).
    pub notices: BTreeMap<String, u32>,
}

/// The text between `open` and the next `close`.
fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text.get(start..)?.find(close)? + start;
    text.get(start..end)
}

/// Splits `text` at `separator` outside quotes and parentheses.
fn split_top(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut quote, mut escaped, mut start) = (0u32, None, false, 0);
    for (at, c) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, c) {
            (_, '\\') => escaped = true,
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth = depth.saturating_sub(1),
            (None, c) if c == separator && depth == 0 => {
                parts.extend(text.get(start..at));
                start = at + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.extend(text.get(start..));
    parts
}

/// A CSS identifier sequence or string, unquoted and unescaped (`\22 ` and `\"` alike).
pub fn css_unescape(value: &str) -> String {
    let value = value.trim();
    let inner = match (value.chars().next(), value.chars().last()) {
        (Some(q @ ('"' | '\'')), Some(end)) if end == q && value.len() >= 2 => {
            value.get(1..value.len() - 1).unwrap_or_default()
        }
        _ => value,
    };
    let mut out = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let mut hex = String::new();
        while hex.len() < 6 && chars.peek().is_some_and(char::is_ascii_hexdigit) {
            hex.extend(chars.next());
        }
        if hex.is_empty() {
            out.extend(chars.next());
            continue;
        }
        if chars.peek() == Some(&' ') {
            chars.next();
        }
        out.push(
            u32::from_str_radix(&hex, 16)
                .ok()
                .and_then(char::from_u32)
                .unwrap_or(char::REPLACEMENT_CHARACTER),
        );
    }
    out
}

/// A computed `font-family` value as its list of families, unquoted.
pub fn font_families(value: &str) -> Vec<String> {
    split_top(value, ',')
        .into_iter()
        .map(css_unescape)
        .filter(|family| !family.is_empty())
        .collect()
}

fn style_of(text: &str) -> Option<FontStyle> {
    match text.trim() {
        "normal" => Some(FontStyle::Normal),
        "italic" => Some(FontStyle::Italic),
        s if s.starts_with("oblique") => Some(FontStyle::Oblique),
        _ => None,
    }
}

/// The lowest and highest weight a descriptor or a licence entry names.
fn weight_range(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.split_whitespace().map(str::parse::<u32>);
    let low = parts.next()?.ok()?;
    let high = match parts.next() {
        Some(high) => high.ok()?,
        None => low,
    };
    parts.next().is_none().then_some((low, high))
}

/// One `@font-face` rule: family, weight descriptor, style and the file's bytes.
struct Rule {
    family: String,
    weight: String,
    style: FontStyle,
    data: Vec<u8>,
}

fn parse_rules(css: &str) -> Result<Vec<Rule>> {
    let mut rules = Vec::new();
    for block in css.split("@font-face{").skip(1) {
        let body = block.split('}').next().unwrap_or_default();
        let mut family = None;
        let mut weight = None;
        let mut style = None;
        let mut data = None;
        for declaration in split_top(body, ';') {
            let Some((name, value)) = declaration.split_once(':') else {
                continue;
            };
            match name.trim() {
                "font-family" => family = Some(css_unescape(value)),
                "font-weight" => weight = Some(value.trim().to_owned()),
                "font-style" => style = style_of(value),
                "src" => {
                    let encoded = between(value, "base64,", ")").ok_or_else(|| {
                        BenchError::failed("an @font-face src is not a base64 data URL")
                    })?;
                    data = Some(
                        base64::engine::general_purpose::STANDARD
                            .decode(encoded)
                            .map_err(|error| {
                                BenchError::failed(format!("@font-face src: {error}"))
                            })?,
                    );
                }
                _ => {}
            }
        }
        match (family, weight, style, data) {
            (Some(family), Some(weight), Some(style), Some(data)) => rules.push(Rule {
                family,
                weight,
                style,
                data,
            }),
            _ => {
                return Err(BenchError::failed(
                    "an @font-face rule lacks its family, weight, style or src",
                ));
            }
        }
    }
    Ok(rules)
}

/// A `LICENSES.txt` face entry: `<family> <weight> <style> (<origin>)`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct LicenceFace {
    family: String,
    weights: (u32, u32),
    style: FontStyle,
    origin: FontOrigin,
}

const ORIGINS: [FontOrigin; 5] = [
    FontOrigin::Embedded,
    FontOrigin::Local,
    FontOrigin::Google,
    FontOrigin::User,
    FontOrigin::Bundled,
];

fn parse_licences(text: &str) -> Vec<LicenceFace> {
    let mut faces = Vec::new();
    let mut in_fonts = false;
    for line in text.lines() {
        if line == "Fonts" {
            in_fonts = true;
            continue;
        }
        if !in_fonts {
            continue;
        }
        let Some(head) = line.strip_suffix(')') else {
            continue;
        };
        let Some((name, origin)) = head.rsplit_once(" (") else {
            continue;
        };
        let Some(origin) = ORIGINS.into_iter().find(|o| o.describe() == origin) else {
            continue;
        };
        let Some((rest, style)) = name.rsplit_once(' ') else {
            continue;
        };
        let Some(style) = style_of(style) else {
            continue;
        };
        // The weight is one number or two; the family is everything before it.
        let words: Vec<&str> = rest.split(' ').collect();
        let numeric = |word: &&&str| word.parse::<u32>().is_ok();
        let tail = words.iter().rev().take_while(numeric).count().min(2);
        if tail == 0 || tail == words.len() {
            continue;
        }
        let split = words.len() - tail;
        let (family, weight) = (
            words.get(..split).unwrap_or_default().join(" "),
            words.get(split..).unwrap_or_default().join(" "),
        );
        if let Some(weights) = weight_range(&weight) {
            faces.push(LicenceFace {
                family,
                weights,
                style,
                origin,
            });
        }
    }
    faces
}

/// The origin of a rule: the licence entry of its family and style whose weights hold the rule's.
fn origin_of(rule: &Rule, licences: &[LicenceFace]) -> Option<FontOrigin> {
    let (low, high) = weight_range(&rule.weight)?;
    licences
        .iter()
        .find(|entry| {
            entry.family == rule.family
                && entry.style == rule.style
                && entry.weights.0 <= low
                && high <= entry.weights.1
        })
        .map(|entry| entry.origin)
}

/// The PostScript name, the family and the emoji bases of a shipped file (WOFF2, or a raw sfnt).
fn names(data: &[u8]) -> Result<(String, String, CodepointSet)> {
    let failed =
        |error: &dyn std::fmt::Display| BenchError::failed(format!("a shipped face: {error}"));
    let unwrapped =
        slidock_fonts::container::unwrap(data, &slidock_fonts::container::Hint::default())
            .map_err(|error| failed(&error))?;
    let face = slidock_fonts::read_sfnt(&unwrapped.sfnt)
        .map_err(|error| failed(&error))?
        .into_iter()
        .next()
        .ok_or_else(|| BenchError::failed("a shipped face holds no font"))?;
    let pictographs =
        slidock_fonts::sfnt::covered(&unwrapped.sfnt, 0, &attribution::pictographic_set())
            .map_err(|error| failed(&error))?;
    Ok((face.postscript_name, face.family, pictographs))
}

/// Reads `index.html` and `LICENSES.txt` of a deck folder.
pub fn read_bundle(dir: &Path) -> Result<BundleFonts> {
    let index = dir.join("index.html");
    let html = std::fs::read_to_string(&index).map_err(BenchError::io(&index))?;
    let licences_path = dir.join("LICENSES.txt");
    let licences = parse_licences(
        &std::fs::read_to_string(&licences_path).map_err(BenchError::io(&licences_path))?,
    );
    let css = between(&html, "<style id=\"sd-fonts\">", "</style>").unwrap_or_default();
    let mut faces = Vec::new();
    for rule in parse_rules(css)? {
        let (postscript_name, file_family, pictographs) = names(&rule.data)?;
        faces.push(DeckFace {
            postscript_name,
            file_family,
            pictographs,
            sha256: Sha256::of(&rule.data),
            bytes: rule.data.len() as u64,
            origin: origin_of(&rule, &licences),
            family: rule.family,
            weight: rule.weight,
            style: rule.style,
        });
    }
    let json = between(
        &html,
        "<script type=\"application/json\" id=\"sd-deck\">",
        "</script>",
    )
    .ok_or_else(|| BenchError::failed(format!("{}: no deck data", index.display())))?;
    let deck: DeckData = serde_json::from_str(json)
        .map_err(|error| BenchError::failed(format!("{}: deck data: {error}", index.display())))?;
    let notices = deck
        .diagnostics
        .codes
        .into_iter()
        .filter(|code| code.code.starts_with("MID54"))
        .map(|code| (code.code, code.count))
        .collect();
    Ok(BundleFonts {
        faces,
        requested: deck.fonts,
        slides: deck.slides,
        size: deck.size,
        css_bytes: css.len() as u64,
        notices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_unquote_and_unescape() {
        assert_eq!(
            font_families("\"Roboto Medium\", Roboto, sans-serif"),
            ["Roboto Medium", "Roboto", "sans-serif"]
        );
        assert_eq!(font_families("\"Ro\\22 bo\", 'a,b'"), ["Ro\"bo", "a,b"]);
        assert_eq!(css_unescape("\"x\\\"y\""), "x\"y");
    }

    #[test]
    fn rules_split_outside_the_data_url() {
        let css = "@font-face{font-family:\"A;B\";src:url(data:font/woff2;base64,AAEC) format(\"woff2\");\
                   font-weight:100 900;font-style:italic;unicode-range:U+20-7E;font-display:block}\
                   @font-face{font-family:\"C\";src:url(data:font/ttf;base64,AA==) format(\"truetype\");\
                   font-weight:400;font-style:normal;font-display:block}";
        let rules = parse_rules(css).unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(
            (
                rules[0].family.as_str(),
                rules[0].weight.as_str(),
                rules[0].style
            ),
            ("A;B", "100 900", FontStyle::Italic)
        );
        assert_eq!(rules[0].data, [0, 1, 2]);
        assert_eq!(rules[1].data, [0]);
        assert!(parse_rules("@font-face{font-family:\"C\";font-weight:400}").is_err());
    }

    #[test]
    fn licence_entries_give_each_rule_its_origin() {
        let text = "slidock's notice\n\nFonts\n\nBarlow Condensed 400 normal (embedded in the presentation)\nCopyright\n\n\
                    Caveat 400 700 normal (added by the user)\nLicence: OFL-1.1\n\n\
                    Liberation Sans 700 italic (a slidock substitute)\n";
        let licences = parse_licences(text);
        assert_eq!(licences.len(), 3);
        assert_eq!(licences[1].weights, (400, 700));
        let rule = |family: &str, weight: &str, style| Rule {
            family: family.into(),
            weight: weight.into(),
            style,
            data: Vec::new(),
        };
        assert_eq!(
            origin_of(
                &rule("Barlow Condensed", "400", FontStyle::Normal),
                &licences
            ),
            Some(FontOrigin::Embedded)
        );
        // A variable face's rule split around a static one keeps its file's origin.
        assert_eq!(
            origin_of(&rule("Caveat", "401 700", FontStyle::Normal), &licences),
            Some(FontOrigin::User)
        );
        assert_eq!(
            origin_of(
                &rule("Liberation Sans", "700", FontStyle::Italic),
                &licences
            ),
            Some(FontOrigin::Bundled)
        );
        assert_eq!(
            origin_of(
                &rule("Liberation Sans", "700", FontStyle::Normal),
                &licences
            ),
            None
        );
    }
}
