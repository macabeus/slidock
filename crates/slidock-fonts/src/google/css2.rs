//! The Google Fonts `css2` API: request URLs built only for catalog families, and a tolerant reader of the
//! `@font-face` rules it answers with.
//!
//! The same URL serves both hosts. A browser (whose `User-Agent` it cannot change) gets WOFF2 files split into
//! per-script `unicode-range` chunks, which the host fetches and subsets locally; a non-browser `User-Agent` (the
//! CLI) gets one static TrueType file per face. The `text=` parameter is never used, so a deck's text is never sent.

use slidock_ir::fonts::FaceStyle;
use thiserror::Error;

use super::catalog::GoogleCatalog;
use super::unicode_range::UnicodeRange;
use crate::bind::WeightRange;

pub const CSS2_ENDPOINT: &str = "https://fonts.googleapis.com/css2";
/// The only places a font file may come from.
const ALLOWED_URLS: &[&str] = &[
    "https://fonts.gstatic.com/s/",
    "https://fonts.gstatic.com/l/font?kit=",
];
/// A larger response is refused unread.
pub const MAX_CSS2_BYTES: usize = 1 << 20;
const MAX_FACES: usize = 4096;

/// The weight a face of a weight-named request draws: the name's weight, raised to bold for a bold slot.
fn drawn_weight(named: Option<u32>, weight: u32) -> u32 {
    match named {
        None => weight,
        Some(named) if weight >= 700 => named.max(700),
        Some(named) => named,
    }
}

/// `+` for spaces, unreserved characters as they are, everything else percent-encoded.
fn encode_family(family: &str) -> String {
    let mut out = String::with_capacity(family.len());
    for byte in family.bytes() {
        match byte {
            b' ' => out.push('+'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(byte))
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The faces to ask `css2` for, so that every slot draws the face a browser would match: for each slot, the
/// family's nearest face (an exact weight inside a `wght` range), each once, sorted by style then weight (the
/// order `css2` requires). `family` may carry a trailing weight name, which sets the drawn weight.
pub fn css2_faces(
    catalog: &GoogleCatalog,
    family: &str,
    slots: &[(u32, FaceStyle)],
) -> Vec<(u32, FaceStyle)> {
    let Some(found) = catalog.find(family) else {
        return Vec::new();
    };
    let mut faces: Vec<(u32, FaceStyle)> = slots
        .iter()
        .filter_map(|&(weight, style)| {
            found
                .family
                .nearest_face(drawn_weight(found.weight, weight), style)
        })
        .collect();
    faces.sort_by_key(|&(weight, style)| (style == FaceStyle::Italic, weight));
    faces.dedup();
    faces
}

/// The `css2` URL for the faces of a family that draw these slots (each `(weight, style)` as the deck draws it), or
/// `None` when the family is not in the catalog. Only faces the family has are asked for, so the request never
/// fails for a missing style or weight. No slot asks for the regular face.
pub fn css2_url(
    catalog: &GoogleCatalog,
    family: &str,
    slots: &[(u32, FaceStyle)],
) -> Option<String> {
    let name = &catalog.find(family)?.family.family;
    let regular = [(400, FaceStyle::Normal)];
    let slots = if slots.is_empty() {
        &regular[..]
    } else {
        slots
    };
    let faces = css2_faces(catalog, family, slots);
    if faces.is_empty() {
        return None;
    }
    let italic = faces.iter().any(|&(_, style)| style == FaceStyle::Italic);
    let tuples: Vec<String> = faces
        .iter()
        .map(|&(weight, style)| match (italic, style) {
            (false, _) => weight.to_string(),
            (true, FaceStyle::Normal) => format!("0,{weight}"),
            (true, FaceStyle::Italic) => format!("1,{weight}"),
        })
        .collect();
    let axes = if italic { "ital,wght" } else { "wght" };
    Some(format!(
        "{CSS2_ENDPOINT}?family={}:{axes}@{}&display=block",
        encode_family(name),
        tuples.join(";")
    ))
}

/// A face one `@font-face` rule of a `css2` response describes.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteFace {
    pub family: String,
    pub style: FaceStyle,
    /// One weight, or a range for a variable file.
    pub weight: WeightRange,
    pub url: String,
    /// The `format()` hint: `woff2`, `truetype`, …
    pub format: Option<String>,
    /// Set on browser responses, which split a face into per-script chunks.
    pub unicode_range: Option<UnicodeRange>,
    /// The subset named by the comment before the rule (`latin`, `cyrillic-ext`).
    pub subset: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum Css2Error {
    #[error("The response is larger than {MAX_CSS2_BYTES} bytes")]
    TooLarge,
    #[error("The response has no @font-face rule")]
    NoFontFace,
    #[error("The response has more than {MAX_FACES} rules")]
    TooManyFaces,
    #[error("A rule's font file is not on fonts.gstatic.com: {0}")]
    UrlRefused(String),
    #[error("A rule cannot be read: {0}")]
    Syntax(String),
}

/// The text between `open` and its matching `close` (quotes and parentheses respected), and the rest after it.
fn block(text: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    for (at, c) in text.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth = depth.saturating_sub(1),
            (None, '}') if depth == 0 => {
                return Some((text.get(..at)?, text.get(at + 1..)?));
            }
            _ => {}
        }
    }
    None
}

/// Declarations split at `;` outside quotes and parentheses, as `(property, value)`.
fn declarations<'a>(body: &'a str) -> Vec<(String, &'a str)> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut start = 0;
    let mut push = |part: &'a str| {
        if let Some((property, value)) = part.split_once(':') {
            out.push((property.trim().to_ascii_lowercase(), value.trim()));
        }
    };
    for (at, c) in body.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth = depth.saturating_sub(1),
            (None, ';') if depth == 0 => {
                push(body.get(start..at).unwrap_or_default());
                start = at + 1;
            }
            _ => {}
        }
    }
    push(body.get(start..).unwrap_or_default());
    out
}

fn unquote(value: &str) -> &str {
    let value = value.trim();
    for q in ['"', '\''] {
        if let Some(inner) = value.strip_prefix(q).and_then(|v| v.strip_suffix(q)) {
            return inner;
        }
    }
    value
}

/// The first `url(…)` of a `src` list and its `format(…)`.
fn source(value: &str) -> Option<(String, Option<String>)> {
    let rest = value.trim().strip_prefix("url(")?;
    let close = rest.find(')')?;
    let url = unquote(rest.get(..close)?).to_owned();
    let after = rest.get(close + 1..)?.trim_start();
    let format = after
        .strip_prefix("format(")
        .and_then(|f| f.split_once(')'))
        .map(|(f, _)| unquote(f).to_owned());
    Some((url, format))
}

fn weight(value: &str) -> Option<WeightRange> {
    let parse = |w: &str| match w {
        "normal" => Some(400),
        "bold" => Some(700),
        _ => w.parse::<u32>().ok().filter(|w| (1..=1000).contains(w)),
    };
    let mut parts = value.split_whitespace();
    let first = parse(parts.next()?)?;
    let second = match parts.next() {
        Some(w) => parse(w)?,
        None => first,
    };
    if parts.next().is_some() {
        return None;
    }
    Some(WeightRange::between(first, second))
}

/// What the reader stops at between rules.
enum Token {
    Comment,
    Rule,
}

/// The next comment or `@font-face` in `text`: its offset and kind. Reads `text` only up to it.
fn next_token(text: &str) -> Option<(usize, Token)> {
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(skip) = bytes
        .get(from..)?
        .iter()
        .position(|&b| b == b'/' || b == b'@')
    {
        let at = from + skip;
        let tail = bytes.get(at..)?;
        if tail.starts_with(b"/*") {
            return Some((at, Token::Comment));
        }
        if tail.starts_with(b"@font-face") {
            return Some((at, Token::Rule));
        }
        from = at + 1;
    }
    None
}

/// The faces of a `css2` response, read in one pass. Any `src` outside fonts.gstatic.com refuses the whole response.
pub fn parse_css2(css: &str) -> Result<Vec<RemoteFace>, Css2Error> {
    if css.len() > MAX_CSS2_BYTES {
        return Err(Css2Error::TooLarge);
    }
    let mut faces = Vec::new();
    let mut subset: Option<String> = None;
    let mut rest = css;
    while let Some((at, token)) = next_token(rest) {
        match token {
            Token::Comment => {
                let after = rest.get(at + 2..).unwrap_or_default();
                let end = after
                    .find("*/")
                    .ok_or_else(|| Css2Error::Syntax("an unclosed comment".into()))?;
                let text = after.get(..end).unwrap_or_default().trim();
                subset = Some(text.to_owned()).filter(|t| !t.is_empty());
                rest = after.get(end + 2..).unwrap_or_default();
            }
            Token::Rule => {
                let after = rest.get(at + "@font-face".len()..).unwrap_or_default();
                let open = after
                    .trim_start()
                    .strip_prefix('{')
                    .ok_or_else(|| Css2Error::Syntax("@font-face without a block".into()))?;
                let (body, tail) = block(open)
                    .ok_or_else(|| Css2Error::Syntax("an unclosed @font-face block".into()))?;
                if faces.len() == MAX_FACES {
                    return Err(Css2Error::TooManyFaces);
                }
                faces.push(face(body, subset.take())?);
                rest = tail;
            }
        }
    }
    if faces.is_empty() {
        return Err(Css2Error::NoFontFace);
    }
    Ok(faces)
}

fn face(body: &str, subset: Option<String>) -> Result<RemoteFace, Css2Error> {
    let (mut family, mut style, mut weights, mut src, mut range) =
        (None, FaceStyle::Normal, None, None, None);
    for (property, value) in declarations(body) {
        match property.as_str() {
            "font-family" => family = Some(unquote(value).to_owned()),
            "font-style" => {
                style = match value {
                    "normal" => FaceStyle::Normal,
                    "italic" => FaceStyle::Italic,
                    v if v.starts_with("oblique") => FaceStyle::Italic,
                    other => return Err(Css2Error::Syntax(format!("font-style {other}"))),
                }
            }
            "font-weight" => {
                weights = Some(
                    weight(value)
                        .ok_or_else(|| Css2Error::Syntax(format!("font-weight {value}")))?,
                )
            }
            "src" => {
                src = Some(source(value).ok_or_else(|| Css2Error::Syntax(format!("src {value}")))?)
            }
            "unicode-range" => {
                range = Some(
                    UnicodeRange::parse(value)
                        .ok_or_else(|| Css2Error::Syntax(format!("unicode-range {value}")))?,
                )
            }
            _ => {}
        }
    }
    let family = family
        .filter(|f| !f.is_empty())
        .ok_or_else(|| Css2Error::Syntax("a rule without font-family".into()))?;
    let (url, format) = src.ok_or_else(|| Css2Error::Syntax("a rule without src".into()))?;
    if !ALLOWED_URLS.iter().any(|prefix| url.starts_with(prefix))
        || url.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(Css2Error::UrlRefused(url));
    }
    Ok(RemoteFace {
        family,
        style,
        weight: weights.unwrap_or(WeightRange::point(400)),
        url,
        format,
        unicode_range: range,
        subset,
    })
}
