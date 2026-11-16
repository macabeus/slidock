//! SVG documents the output shows as pictures. Office suites keep a vector original beside a small raster preview
//! (Keynote's `originalSVGData`, PowerPoint's `asvg:svgBlip`, an ODF frame's first `draw:image`); the preview is a
//! thumbnail, so a vector picture is drawn from its SVG when the SVG passes [`check_svg`].
//!
//! The check is an allow-list over the parsed document, so a passed file can run no script, load nothing and style
//! nothing outside itself, wherever it is opened: strict UTF-8; no DOCTYPE but an external identifier with no internal
//! subset (so no entities), which browsers never fetch and the written picture drops; no processing instructions;
//! every element in the SVG namespace and one of the plain drawing, text and label elements; every attribute without a
//! namespace (or `xml:space`) and one of their presentation attributes; values with no CSS escape and no function but a
//! colour, a transform or `url(…)` to a fragment of the same document; text only in `text`, `tspan` and the labels; a
//! `style` sheet of `@font-face` rules whose sources are fonts embedded as `data:` URLs and of plain class, id or
//! element rules that set presentation properties; text drawn only in a family such a rule embeds (inside an `<img>`
//! the SVG reaches no font but its own and the viewer's); and a root whose size and `preserveAspectRatio` are read.
//! Anything else refuses the whole file, which then falls back to the raster its source offers.
//!
//! A picture fills its frame whatever its proportions (Keynote, PowerPoint and LibreOffice stretch it), but an `<img>`
//! fits an SVG's `viewBox` to its box by the root's `preserveAspectRatio`, which letterboxes by default. So the output
//! writes [`picture_svg`]: the passed document with its root set to `preserveAspectRatio="none"` and a `viewBox` of what
//! its own viewport shows, which draws the same picture at its own size and stretches it to any other.

use roxmltree::{Node, NodeType};

use crate::xml::{XmlBudget, XmlErrorReason, XmlLimits, parse_xml};

const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// The largest SVG read: a slide background is tens of kilobytes.
pub const MAX_SVG_BYTES: u64 = 4 * 1024 * 1024;

/// The largest side an SVG may state, in CSS pixels.
const MAX_SIDE: f64 = 100_000.0;

/// The drawing elements a passed SVG may hold.
const ELEMENTS: [&str; 21] = [
    "svg",
    "g",
    "defs",
    "title",
    "desc",
    "metadata",
    "style",
    "text",
    "tspan",
    "mask",
    "path",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "linearGradient",
    "radialGradient",
    "stop",
    "clipPath",
];

/// Elements whose text is a label, never drawn.
const LABELS: [&str; 3] = ["title", "desc", "metadata"];

/// Elements whose text is drawn.
const TEXTS: [&str; 2] = ["text", "tspan"];

/// The namespace of `xml:space`, the one namespaced attribute shown.
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// Geometry and structure attributes.
const GEOMETRY: [&str; 38] = [
    "id",
    "class",
    "version",
    "role",
    "aria-label",
    "aria-labelledby",
    "aria-hidden",
    "viewBox",
    "preserveAspectRatio",
    "transform",
    "d",
    "x",
    "y",
    "x1",
    "y1",
    "x2",
    "y2",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "fx",
    "fy",
    "width",
    "height",
    "points",
    "offset",
    "gradientUnits",
    "gradientTransform",
    "spreadMethod",
    "dx",
    "dy",
    "rotate",
    "textLength",
    "lengthAdjust",
    "maskUnits",
    "maskContentUnits",
];

/// Presentation attributes, which `style` may also set as properties.
const PRESENTATION: [&str; 39] = [
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-opacity",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-dasharray",
    "stroke-dashoffset",
    "opacity",
    "clip-path",
    "clip-rule",
    "clipPathUnits",
    "shape-rendering",
    "color",
    "display",
    "visibility",
    "stop-color",
    "stop-opacity",
    "mask",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "font-variant",
    "font-stretch",
    "text-anchor",
    "dominant-baseline",
    "alignment-baseline",
    "baseline-shift",
    "direction",
    "writing-mode",
    "letter-spacing",
    "word-spacing",
    "text-decoration",
    "unicode-bidi",
    "white-space",
];

/// Properties `style` may set besides the presentation attributes: the root's own size.
const STYLE_ONLY: [&str; 2] = ["width", "height"];

/// Attributes that name or label, which no browser reads as CSS.
const NAMES: [&str; 7] = [
    "id",
    "class",
    "version",
    "role",
    "aria-label",
    "aria-labelledby",
    "aria-hidden",
];

/// The functions a value may call, lowercased: colours, transforms and `url(#…)`.
const FUNCTIONS: [&str; 11] = [
    "url",
    "rgb",
    "rgba",
    "hsl",
    "hsla",
    "matrix",
    "translate",
    "scale",
    "rotate",
    "skewx",
    "skewy",
];

/// Why an SVG is not shown.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SvgRefusal {
    #[error("the SVG is over {MAX_SVG_BYTES} bytes")]
    TooLarge,
    #[error("the SVG is not UTF-8")]
    Encoding,
    #[error("the SVG does not parse ({})", .0.as_str())]
    Xml(XmlErrorReason),
    #[error("the root is not an SVG element")]
    Root,
    #[error("the element {0} is not shown")]
    Element(String),
    #[error("the attribute {0} is not shown")]
    Attribute(String),
    #[error(
        "the value of {0} holds an escape or a function other than a colour, a transform or url(#…)"
    )]
    Value(String),
    #[error("the SVG has text outside a text element or a label")]
    Text,
    #[error("the SVG's style sheet {0}")]
    Sheet(&'static str),
    #[error("the SVG draws text in a font it does not embed")]
    Font,
    #[error("the SVG has a processing instruction")]
    Instruction,
    #[error("the SVG states no size in pixels")]
    Size,
    #[error("the root's preserveAspectRatio is not read")]
    Aspect,
}

/// A passed SVG's intrinsic size, in CSS pixels, rounded to whole pixels (at least 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SvgFacts {
    pub width: u32,
    pub height: u32,
}

fn limits() -> XmlLimits {
    XmlLimits {
        depth: 64,
        nodes: 200_000,
        presentation_nodes: 200_000,
        attributes: 64,
        attribute_bytes: 1024 * 1024,
    }
}

/// Whether an SVG may be shown, and its size; a refusal names the first thing it holds that may not.
pub fn check_svg(bytes: &[u8]) -> Result<SvgFacts, SvgRefusal> {
    let text = without_doctype(utf8(bytes)?);
    let document = parse(&text)?;
    viewport_of(document.root_element())?.facts()
}

/// A passed SVG as the output writes it for a picture: its root set to `preserveAspectRatio="none"` with a `viewBox`
/// of what its own viewport shows (its `viewBox` fitted as its `preserveAspectRatio` says, or its size in pixels when
/// it has none), so it draws as before at its own size and stretches to fill a box of any other proportions. Nothing
/// else changes but its external DOCTYPE, which is dropped; a document that already stretches is returned as it is
/// (without a byte-order mark or that DOCTYPE).
pub fn picture_svg(bytes: &[u8]) -> Result<Vec<u8>, SvgRefusal> {
    let text = without_doctype(utf8(bytes)?);
    let text = text.as_ref();
    let document = parse(text)?;
    let root = document.root_element();
    let viewport = viewport_of(root)?;
    let shown = viewport.shown()?;
    let keeps_view_box = viewport.view_box == Some(shown);
    if keeps_view_box && viewport.fit == Fit::Stretch {
        return Ok(text.as_bytes().to_vec());
    }
    // Each attribute replaced goes with the white space before it.
    let mut removed: Vec<std::ops::Range<usize>> = root
        .attributes()
        .filter(|attribute| {
            attribute.namespace().is_none()
                && (attribute.name() == "preserveAspectRatio"
                    || (attribute.name() == "viewBox" && !keeps_view_box))
        })
        .map(|attribute| {
            let range = attribute.range();
            text[..range.start]
                .trim_end_matches([' ', '\t', '\n', '\r'])
                .len()..range.end
        })
        .collect();
    removed.sort_by_key(|range| range.start);
    let mut inserted = String::new();
    if !keeps_view_box {
        let [x, y, w, h] = shown;
        inserted.push_str(&format!(r#" viewBox="{x} {y} {w} {h}""#));
    }
    inserted.push_str(r#" preserveAspectRatio="none""#);
    // After the root's last attribute: it states a size or a viewBox, so it has one.
    let at = root
        .attributes()
        .map(|attribute| attribute.range().end)
        .max()
        .ok_or(SvgRefusal::Size)?;
    let mut out = String::with_capacity(text.len() + inserted.len());
    let mut from = 0;
    for range in removed {
        out.push_str(&text[from..range.start]);
        from = range.end;
    }
    out.push_str(&text[from..at]);
    out.push_str(&inserted);
    out.push_str(&text[at..]);
    // The rewrite passes again, and stretches what the original showed.
    let rewritten = viewport_of(parse(&out)?.root_element())?;
    if rewritten.fit != Fit::Stretch || rewritten.view_box != Some(shown) {
        return Err(SvgRefusal::Aspect);
    }
    Ok(out.into_bytes())
}

/// The bytes as strict UTF-8 (a leading byte-order mark dropped) whose XML declaration, if any, names UTF-8.
fn utf8(bytes: &[u8]) -> Result<&str, SvgRefusal> {
    if bytes.len() as u64 > MAX_SVG_BYTES {
        return Err(SvgRefusal::TooLarge);
    }
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|_| SvgRefusal::Encoding)?;
    if declares_utf8(text) {
        Ok(text)
    } else {
        Err(SvgRefusal::Encoding)
    }
}

/// XML white space.
fn is_xml_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// The text without its document type declaration when that declaration is `<!DOCTYPE svg PUBLIC "…" "…">` or
/// `<!DOCTYPE svg SYSTEM "…">` in the prolog (after the XML declaration, white space and comments), with no internal
/// subset: it declares no entity, and no browser fetches the external one. Any other DOCTYPE stays, and the parser
/// refuses it.
fn without_doctype(text: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    let mut at = 0;
    if text.starts_with("<?xml") {
        match text.find("?>") {
            Some(end) => at = end + 2,
            None => return Cow::Borrowed(text),
        }
    }
    loop {
        at = text.len() - text[at..].trim_start_matches(is_xml_space).len();
        if !text[at..].starts_with("<!--") {
            break;
        }
        match text[at + 4..].find("-->") {
            Some(end) => at += 4 + end + 3,
            None => return Cow::Borrowed(text),
        }
    }
    match external_doctype(&text[at..]) {
        Some(length) => Cow::Owned(format!("{}{}", &text[..at], &text[at + length..])),
        None => Cow::Borrowed(text),
    }
}

/// The length of the external document type declaration of an `svg` root that `text` starts with.
fn external_doctype(text: &str) -> Option<usize> {
    let rest = text.strip_prefix("<!DOCTYPE")?;
    let spaced = |rest: &str| -> Option<usize> {
        let trimmed = rest.trim_start_matches(is_xml_space);
        (trimmed.len() < rest.len()).then_some(rest.len() - trimmed.len())
    };
    // A quoted literal of printable ASCII that holds no markup or subset delimiter.
    let literal = |rest: &str| -> Option<usize> {
        let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let end = rest[1..].find(quote)?;
        rest[1..1 + end]
            .chars()
            .all(|c| (' '..='~').contains(&c) && !matches!(c, '<' | '>' | '[' | ']'))
            .then_some(end + 2)
    };
    let mut rest = &rest[spaced(rest)?..];
    rest = rest.strip_prefix("svg")?;
    rest = &rest[spaced(rest)?..];
    let literals = if let Some(after) = rest.strip_prefix("PUBLIC") {
        rest = after;
        2
    } else {
        rest = rest.strip_prefix("SYSTEM")?;
        1
    };
    for _ in 0..literals {
        rest = &rest[spaced(rest)?..];
        rest = &rest[literal(rest)?..];
    }
    rest = rest.trim_start_matches(is_xml_space);
    rest = rest.strip_prefix('>')?;
    Some(text.len() - rest.len())
}

/// The parsed document, every node checked against the allow-list.
fn parse(text: &str) -> Result<roxmltree::Document<'_>, SvgRefusal> {
    let limits = limits();
    let mut budget = XmlBudget::new(&limits);
    let document = parse_xml(text, &limits, &mut budget).map_err(|e| SvgRefusal::Xml(e.reason))?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(SVG_NS) || root.tag_name().name() != "svg" {
        return Err(SvgRefusal::Root);
    }
    let mut fonts = Fonts::default();
    for node in document.root().descendants() {
        match node.node_type() {
            NodeType::Root | NodeType::Comment => {}
            NodeType::PI => return Err(SvgRefusal::Instruction),
            NodeType::Text => check_text(node)?,
            NodeType::Element => check_element(node, &mut fonts)?,
        }
    }
    check_fonts(&document, &fonts)?;
    Ok(document)
}

/// An XML declaration's encoding, when it states one, is UTF-8.
fn declares_utf8(text: &str) -> bool {
    let Some(declaration) = text.strip_prefix("<?xml") else {
        return true;
    };
    let Some((declaration, _)) = declaration.split_once("?>") else {
        return false;
    };
    let Some((_, rest)) = declaration.split_once("encoding") else {
        return true;
    };
    let Some(rest) = rest.trim_start().strip_prefix('=') else {
        return false;
    };
    let value = rest.trim_start().trim_start_matches(['"', '\'']);
    let value = value.split(['"', '\'', ' ']).next().unwrap_or("");
    value.eq_ignore_ascii_case("utf-8") || value.eq_ignore_ascii_case("utf8")
}

fn check_text(node: Node<'_, '_>) -> Result<(), SvgRefusal> {
    // A style sheet's text is checked with its element.
    let label = node.parent_element().is_some_and(|parent| {
        let name = parent.tag_name().name();
        LABELS.contains(&name) || TEXTS.contains(&name) || name == "style"
    });
    let blank = node
        .text()
        .unwrap_or("")
        .chars()
        .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r'));
    if label || blank {
        Ok(())
    } else {
        Err(SvgRefusal::Text)
    }
}

fn check_element(node: Node<'_, '_>, fonts: &mut Fonts) -> Result<(), SvgRefusal> {
    let name = node.tag_name();
    if name.namespace() != Some(SVG_NS) || !ELEMENTS.contains(&name.name()) {
        return Err(SvgRefusal::Element(clip(name.name())));
    }
    let is_sheet = name.name() == "style";
    for attribute in node.attributes() {
        let key = attribute.name();
        let plain = attribute.namespace().is_none();
        if plain && key == "style" {
            check_style(attribute.value())?;
            continue;
        }
        if attribute.namespace() == Some(XML_NS) && key == "space" {
            if matches!(attribute.value(), "default" | "preserve") {
                continue;
            }
            return Err(SvgRefusal::Value("xml:space".to_owned()));
        }
        // A style sheet names its language, which can only be CSS.
        if is_sheet && plain && key == "type" {
            if attribute.value().trim() == "text/css" {
                continue;
            }
            return Err(SvgRefusal::Value("type".to_owned()));
        }
        if !plain || !(GEOMETRY.contains(&key) || PRESENTATION.contains(&key)) {
            return Err(SvgRefusal::Attribute(clip(key)));
        }
        check_value(key, attribute.value())?;
    }
    if is_sheet {
        if node.children().any(|child| child.is_element()) {
            return Err(SvgRefusal::Sheet("holds an element"));
        }
        let sheet: String = node
            .children()
            .filter(Node::is_text)
            .filter_map(|child| child.text())
            .collect();
        check_sheet(&sheet, fonts)?;
    }
    Ok(())
}

/// A value read as CSS holds no escape (CSS reads `u\72l(` as `url(`) and calls no function but a colour, a transform or
/// `url(…)` to a fragment of the same document. Names and labels are not read as CSS.
fn check_value(key: &str, value: &str) -> Result<(), SvgRefusal> {
    if NAMES.contains(&key) {
        return Ok(());
    }
    let refused = || SvgRefusal::Value(clip(key));
    if value.contains('\\') {
        return Err(refused());
    }
    let lower = value.to_ascii_lowercase();
    // A character of an identifier, as CSS reads one: any non-ASCII character is.
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_' || !c.is_ascii();
    for (at, _) in lower.match_indices('(') {
        let head = &lower[..at];
        // The character found is ASCII, so the name starts one byte after it.
        let name = &head[head.rfind(|c: char| !ident(c)).map_or(0, |i| i + 1)..];
        if !FUNCTIONS.contains(&name) {
            return Err(refused());
        }
        let argument =
            lower[at + 1..].trim_start_matches([' ', '\t', '\n', '\r', '\x0c', '"', '\'']);
        if name == "url" && !argument.starts_with('#') {
            return Err(refused());
        }
    }
    Ok(())
}

/// A `style` holds only `property: value` declarations of presentation properties, in plain characters: no escapes,
/// comments, at-rules, quotes or blocks.
fn check_style(style: &str) -> Result<(), SvgRefusal> {
    let refused = || SvgRefusal::Attribute("style".to_owned());
    if style.contains(['\\', '@', '{', '}', '<', '>', '"', '\'', '!']) || style.contains("/*") {
        return Err(refused());
    }
    for declaration in style.split(';') {
        if declaration.trim().is_empty() {
            continue;
        }
        let (property, value) = declaration.split_once(':').ok_or_else(refused)?;
        let property = property.trim();
        if !(PRESENTATION.contains(&property) || STYLE_ONLY.contains(&property)) {
            return Err(SvgRefusal::Attribute(format!("style {}", clip(property))));
        }
        check_value("style", value)?;
    }
    Ok(())
}

/// The `@font-face` descriptors a sheet may set.
const FACE_DESCRIPTORS: [&str; 7] = [
    "font-family",
    "src",
    "font-weight",
    "font-style",
    "font-stretch",
    "font-display",
    "unicode-range",
];

/// The media types of an embedded font source.
const FONT_TYPES: [&str; 13] = [
    "font/woff2",
    "font/woff",
    "font/ttf",
    "font/otf",
    "font/sfnt",
    "font/collection",
    "application/font-woff",
    "application/font-woff2",
    "application/x-font-woff",
    "application/x-font-ttf",
    "application/x-font-truetype",
    "application/x-font-opentype",
    "application/vnd.ms-opentype",
];

/// The first four bytes of a font file: WOFF2, WOFF, TrueType (two ways), OpenType CFF and a collection.
const FONT_MAGIC: [&[u8; 4]; 6] = [b"wOF2", b"wOFF", b"\0\x01\0\0", b"true", b"OTTO", b"ttcf"];

/// A `style` element's sheet: `@font-face` rules whose sources are fonts embedded as `data:` URLs, and rules of plain
/// selectors (an element name, classes and ids, in a comma list) that set presentation properties as a `style`
/// attribute may. No escapes, comments, markup, `!important`, other at-rules or nested blocks, so the sheet loads
/// nothing and styles nothing outside its document. What it says about fonts goes into `fonts`.
fn check_sheet(sheet: &str, fonts: &mut Fonts) -> Result<(), SvgRefusal> {
    if sheet.contains(['\\', '<', '>', '!']) || sheet.contains("/*") {
        return Err(SvgRefusal::Sheet("holds an escape, a comment or markup"));
    }
    let mut rest = sheet;
    loop {
        rest = rest.trim_start_matches(is_xml_space);
        if rest.is_empty() {
            return Ok(());
        }
        let open = rest
            .find('{')
            .ok_or(SvgRefusal::Sheet("has a rule without a block"))?;
        let prelude = rest[..open].trim_matches(is_xml_space);
        let body_start = open + 1;
        let close = rest[body_start..]
            .find('}')
            .ok_or(SvgRefusal::Sheet("has a block that is not closed"))?;
        let body = &rest[body_start..body_start + close];
        if prelude.contains(['}', ';']) || body.contains('{') {
            return Err(SvgRefusal::Sheet("has a nested or broken rule"));
        }
        if prelude == "@font-face" {
            if let Some(family) = check_font_face(body)? {
                fonts.embedded.insert(family);
            }
        } else if prelude.contains('@') {
            return Err(SvgRefusal::Sheet("has an at-rule other than @font-face"));
        } else {
            if !prelude.split(',').all(plain_selector) {
                return Err(SvgRefusal::Sheet(
                    "has a selector other than an element, class or id",
                ));
            }
            check_style(body)
                .map_err(|_| SvgRefusal::Sheet("sets a property or value a style may not"))?;
            for value in style_families(body) {
                for selector in prelude.split(',') {
                    if fonts.rules.len() == MAX_FONT_RULES {
                        return Err(SvgRefusal::Sheet("sets a font family in too many rules"));
                    }
                    fonts.rules.push((
                        selector.trim_matches(is_xml_space).to_owned(),
                        value.to_owned(),
                    ));
                }
            }
        }
        rest = &rest[body_start + close + 1..];
    }
}

/// An element name, then classes and ids (`text.label#x`), with at least one of them.
fn plain_selector(selector: &str) -> bool {
    let selector = selector.trim_matches(is_xml_space);
    let name = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    let (element, mut tail) =
        selector.split_at(selector.find(['.', '#']).unwrap_or(selector.len()));
    if selector.is_empty()
        || !(element.is_empty()
            || (name(element) && element.starts_with(|c: char| c.is_ascii_alphabetic())))
    {
        return false;
    }
    // Each part starts with its `.` or `#`.
    while let Some(rest) = tail.get(1..) {
        let end = rest.find(['.', '#']).unwrap_or(rest.len());
        if !name(&rest[..end]) {
            return false;
        }
        tail = &rest[end..];
    }
    true
}

/// An `@font-face` block: its descriptors, each once at most, with plain values and `src` of embedded fonts only. The
/// family it embeds, as [`first_family`] reads it, when it names one.
fn check_font_face(body: &str) -> Result<Option<String>, SvgRefusal> {
    let mut seen = Vec::new();
    for declaration in split_outside(body, ';')? {
        let (property, value) = declaration
            .split_once(':')
            .ok_or(SvgRefusal::Sheet("has a declaration without a value"))?;
        let property = property.trim_matches(is_xml_space);
        let value = value.trim_matches(is_xml_space);
        if !FACE_DESCRIPTORS.contains(&property) || seen.contains(&property) {
            return Err(SvgRefusal::Sheet("has a font descriptor not shown"));
        }
        seen.push(property);
        let plain = |extra: &str| {
            value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == ' ' || extra.contains(c))
        };
        let passes = match property {
            "src" => split_outside(value, ',')?.into_iter().all(embedded_font),
            "font-family" => unquote(value).is_some_and(|unquoted| {
                !unquoted.is_empty()
                    && unquoted
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_'))
            }),
            "unicode-range" => plain("+?,-"),
            _ => plain("%.-"),
        };
        if !passes {
            return Err(SvgRefusal::Sheet(
                "has a font descriptor whose value is not shown",
            ));
        }
    }
    if !seen.contains(&"src") {
        return Err(SvgRefusal::Sheet("has a font face without a source"));
    }
    Ok(split_outside(body, ';')?
        .into_iter()
        .find_map(|declaration| {
            let (property, value) = declaration.split_once(':')?;
            (property.trim_matches(is_xml_space) == "font-family").then(|| first_family(value))?
        }))
}

/// The parts of a block or value between the separators outside parentheses (a `data:` URL holds `;` and `,`), blank
/// parts left out.
fn split_outside(body: &str, separator: char) -> Result<Vec<&str>, SvgRefusal> {
    let mut out = Vec::new();
    let (mut depth, mut from) = (0u32, 0);
    for (at, c) in body.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or(SvgRefusal::Sheet("has an unbalanced parenthesis"))?;
            }
            c if c == separator && depth == 0 => {
                out.push(&body[from..at]);
                from = at + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(SvgRefusal::Sheet("has an unbalanced parenthesis"));
    }
    out.push(&body[from..]);
    Ok(out
        .into_iter()
        .filter(|declaration| !declaration.trim_matches(is_xml_space).is_empty())
        .collect())
}

/// One `src` item: `url(data:<font type>;base64,<data>)`, the URL quoted or not, whose data starts with a font's
/// magic number, then at most a `format(…)` hint of a plain name.
fn embedded_font(item: &str) -> bool {
    let item = item.trim_matches(is_xml_space);
    let Some(rest) = item.strip_prefix("url(") else {
        return false;
    };
    let Some(close) = rest.find(')') else {
        return false;
    };
    let (url, after) = (rest[..close].trim_matches(is_xml_space), &rest[close + 1..]);
    let Some(url) = unquote(url) else {
        return false;
    };
    let Some((head, data)) = url
        .strip_prefix("data:")
        .and_then(|u| u.split_once(";base64,"))
    else {
        return false;
    };
    if !FONT_TYPES.contains(&head.to_ascii_lowercase().as_str())
        || !data
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
        || !data
            .get(..8)
            .and_then(base64_head)
            .is_some_and(|magic| FONT_MAGIC.contains(&&magic))
    {
        return false;
    }
    let after = after.trim_matches(is_xml_space);
    if after.is_empty() {
        return true;
    }
    let Some(hint) = after
        .strip_prefix("format(")
        .and_then(|h| h.strip_suffix(')'))
    else {
        return false;
    };
    unquote(hint).is_some_and(|hint| {
        !hint.is_empty() && hint.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// A CSS value inside one pair of matching quotes, or the value itself when it opens none; `None` when it opens a
/// quote that only its last character may close, and does not (CSS would read the rest of the sheet into the string).
fn unquote(value: &str) -> Option<&str> {
    match value.chars().next() {
        Some(quote @ ('"' | '\'')) => value[1..].strip_suffix(quote),
        _ => Some(value),
    }
}

/// The most rules of a document's sheets that may set `font-family`, counting each selector of a list: what decides
/// an element's font is read rule by rule, element by element.
const MAX_FONT_RULES: usize = 64;

/// What a document's style sheets say about fonts.
#[derive(Default)]
struct Fonts {
    /// The families its `@font-face` rules embed, as [`first_family`] reads them.
    embedded: std::collections::BTreeSet<String>,
    /// Each rule that sets `font-family`: one plain selector and the value.
    rules: Vec<(String, String)>,
}

/// The `font-family` values a checked `style` declaration list sets.
fn style_families(style: &str) -> impl Iterator<Item = &str> {
    style.split(';').filter_map(|declaration| {
        let (property, value) = declaration.split_once(':')?;
        (property.trim() == "font-family").then_some(value)
    })
}

/// Unquoted names CSS reads as keywords: the generic families, which name no face, and the CSS-wide keywords (with
/// `default`), which are not names at all, so neither names an embedded face.
const GENERIC_FAMILIES: [&str; 13] = [
    "serif",
    "sans-serif",
    "cursive",
    "fantasy",
    "monospace",
    "system-ui",
    "emoji",
    "math",
    "fangsong",
    "ui-serif",
    "ui-sans-serif",
    "ui-monospace",
    "ui-rounded",
];
const CSS_WIDE: [&str; 6] = [
    "inherit",
    "initial",
    "unset",
    "revert",
    "revert-layer",
    "default",
];

/// The first family a `font-family` value names, as CSS matches family names (ASCII case folded; an unquoted name's
/// words joined by one space), or `None` for a generic family, a keyword or a name CSS does not read.
fn first_family(value: &str) -> Option<String> {
    let first = value
        .split(',')
        .next()?
        .trim_matches(|c| is_xml_space(c) || c == '\x0c');
    if let Some(inner) = first.strip_prefix(['"', '\'']) {
        let quote = &first[..1];
        let inner = inner.strip_suffix(quote)?;
        if inner.is_empty() || inner.contains(['"', '\'', '\\', '\n', '\r', '\x0c']) {
            return None;
        }
        return Some(inner.to_ascii_lowercase());
    }
    let words: Vec<String> = first
        .split(|c| is_xml_space(c) || c == '\x0c')
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    // Each word an identifier: a letter, `_` or any non-ASCII character first (after a `-`), then those, digits and `-`.
    let ident = |word: &str| {
        let start = word.strip_prefix('-').unwrap_or(word);
        start
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '-' || !c.is_ascii())
            && word
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || !c.is_ascii())
    };
    let keyword = |word: &String| CSS_WIDE.contains(&word.as_str());
    if words.is_empty()
        || !words.iter().all(|word| ident(word))
        || words.iter().any(keyword)
        || matches!(words.as_slice(), [only] if GENERIC_FAMILIES.contains(&only.as_str()))
    {
        return None;
    }
    Some(words.join(" "))
}

/// Whether a plain selector (see [`plain_selector`]) selects an element: its name, and each class and id.
fn selects(selector: &str, node: Node<'_, '_>) -> bool {
    let (element, mut tail) =
        selector.split_at(selector.find(['.', '#']).unwrap_or(selector.len()));
    if !element.is_empty() && element != node.tag_name().name() {
        return false;
    }
    while let Some(rest) = tail.get(1..) {
        let end = rest.find(['.', '#']).unwrap_or(rest.len());
        let part = &rest[..end];
        let found = if tail.starts_with('.') {
            node.attribute("class").is_some_and(|classes| {
                classes
                    .split(|c| is_xml_space(c) || c == '\x0c')
                    .any(|class| class == part)
            })
        } else {
            node.attribute("id") == Some(part)
        };
        if !found {
            return false;
        }
        tail = &rest[end..];
    }
    true
}

/// Drawn text is drawn in a font the document embeds. Inside an `<img>` an SVG reaches no font but its own and the
/// viewer's, so text in any other family would be drawn in whatever the viewer has, unlike the preview it replaces.
/// For each `text` or `tspan` that holds more than white space, the nearest of it and its ancestors that sets
/// `font-family` (by attribute, `style` or a rule) sets only values whose first family an `@font-face` embeds; when
/// none sets it, the text is in the viewer's default font, and refused.
fn check_fonts(document: &roxmltree::Document<'_>, fonts: &Fonts) -> Result<(), SvgRefusal> {
    let drawn = |node: &Node<'_, '_>| {
        node.is_text()
            && node
                .parent_element()
                .is_some_and(|parent| TEXTS.contains(&parent.tag_name().name()))
            && !node
                .text()
                .unwrap_or("")
                .chars()
                .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r'))
    };
    if !document.descendants().any(|node| drawn(&node)) {
        return Ok(());
    }
    // Whether each element's font is embedded (`None`: no family set on it or above), in document order, so a parent
    // comes before its children.
    let mut embedded: Vec<Option<bool>> = Vec::new();
    for node in document.descendants() {
        let index = node.id().get_usize();
        if embedded.len() <= index {
            embedded.resize(index + 1, None);
        }
        if !node.is_element() {
            continue;
        }
        let mut values: Vec<&str> = node.attribute("font-family").into_iter().collect();
        values.extend(node.attribute("style").into_iter().flat_map(style_families));
        values.extend(
            fonts
                .rules
                .iter()
                .filter(|(selector, _)| selects(selector, node))
                .map(|(_, value)| value.as_str()),
        );
        let own = if values.is_empty() {
            node.parent()
                .and_then(|parent| embedded.get(parent.id().get_usize()).copied().flatten())
        } else {
            Some(values.iter().all(|value| {
                first_family(value).is_some_and(|family| fonts.embedded.contains(&family))
            }))
        };
        if let Some(slot) = embedded.get_mut(index) {
            *slot = own;
        }
    }
    for node in document.descendants().filter(drawn) {
        let parent = node.parent_element().ok_or(SvgRefusal::Font)?;
        if embedded.get(parent.id().get_usize()).copied().flatten() != Some(true) {
            return Err(SvgRefusal::Font);
        }
    }
    Ok(())
}

/// The first four bytes that eight base64 characters encode.
fn base64_head(text: &str) -> Option<[u8; 4]> {
    let mut bits: u64 = 0;
    for c in text.bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        bits = (bits << 6) | u64::from(value);
    }
    // Eight characters are 48 bits; the first 32 are the four bytes.
    let head = u32::try_from(bits >> 16).ok()?;
    Some(head.to_be_bytes())
}

/// How the root fits its `viewBox` to its viewport (`preserveAspectRatio`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fit {
    /// `none`.
    Stretch,
    /// Scaled alike on both axes, to fit inside (`meet`) or to cover (`slice`), placed at a fraction of the slack on
    /// each axis (`Min` 0, `Mid` ½, `Max` 1).
    Align { x: f64, y: f64, slice: bool },
}

/// `none`, or `x(Min|Mid|Max)Y(Min|Mid|Max)` then `meet` or `slice`; `xMidYMid meet` when absent. Anything else
/// (`defer`, a misspelling) is not read.
fn fit_of(value: Option<&str>) -> Option<Fit> {
    let Some(value) = value else {
        return Some(Fit::Align {
            x: 0.5,
            y: 0.5,
            slice: false,
        });
    };
    let mut words = value.split_ascii_whitespace();
    let align = words.next()?;
    let slice = match words.next() {
        None | Some("meet") => false,
        Some("slice") => true,
        Some(_) => return None,
    };
    if words.next().is_some() {
        return None;
    }
    if align == "none" {
        return Some(Fit::Stretch);
    }
    let axis = |text: &str| match text {
        "Min" => Some(0.0),
        "Mid" => Some(0.5),
        "Max" => Some(1.0),
        _ => None,
    };
    let (x, y) = align.strip_prefix('x')?.split_once('Y')?;
    Some(Fit::Align {
        x: axis(x)?,
        y: axis(y)?,
        slice,
    })
}

/// The root's viewport: its size in pixels, its `viewBox` and how it fits one to the other.
#[derive(Clone, Copy, Debug)]
struct Viewport {
    width: f64,
    height: f64,
    view_box: Option<[f64; 4]>,
    fit: Fit,
}

impl Viewport {
    fn facts(&self) -> Result<SvgFacts, SvgRefusal> {
        let side = |v: f64| -> Result<u32, SvgRefusal> {
            if v.is_finite() && v > 0.0 && v <= MAX_SIDE {
                // At most MAX_SIDE, so it fits.
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Ok((v.round() as u32).max(1))
            } else {
                Err(SvgRefusal::Size)
            }
        };
        Ok(SvgFacts {
            width: side(self.width)?,
            height: side(self.height)?,
        })
    }

    /// The region of user space the viewport shows: the `viewBox` widened (`meet`) or narrowed (`slice`) on one axis
    /// to the viewport's proportions and placed by the alignment, or the pixels `0 0 width height` without a `viewBox`.
    fn shown(&self) -> Result<[f64; 4], SvgRefusal> {
        let shown = match (self.view_box, self.fit) {
            (None, _) => [0.0, 0.0, self.width, self.height],
            (Some(view_box), Fit::Stretch) => view_box,
            (
                Some([x, y, w, h]),
                Fit::Align {
                    x: ax,
                    y: ay,
                    slice,
                },
            ) => {
                let (sx, sy) = (self.width / w, self.height / h);
                let scale = if slice { sx.max(sy) } else { sx.min(sy) };
                let (sw, sh) = (self.width / scale, self.height / scale);
                [x - (sw - w) * ax, y - (sh - h) * ay, sw, sh]
            }
        };
        if shown.iter().all(|v| v.is_finite()) && shown[2] > 0.0 && shown[3] > 0.0 {
            Ok(shown)
        } else {
            Err(SvgRefusal::Size)
        }
    }
}

/// The root's viewport: its `width` and `height` in pixels (unitless or `px`), else its `viewBox`'s; its `viewBox`
/// when it states one (four finite numbers, the width and height positive); and its `preserveAspectRatio`.
fn viewport_of(root: Node<'_, '_>) -> Result<Viewport, SvgRefusal> {
    let pixels = |key: &str| -> Option<f64> {
        let text = root.attribute(key)?.trim();
        let number = text.strip_suffix("px").unwrap_or(text).trim();
        number.parse::<f64>().ok()
    };
    let view_box = match root.attribute("viewBox") {
        None => None,
        Some(text) => {
            let numbers: Vec<f64> = text
                .split([' ', ',', '\t', '\n', '\r'])
                .filter(|part| !part.is_empty())
                .map(|part| part.parse::<f64>().map_err(|_| SvgRefusal::Size))
                .collect::<Result<_, _>>()?;
            match numbers.as_slice() {
                &[x, y, w, h]
                    if [x, y, w, h].iter().all(|v| v.is_finite()) && w > 0.0 && h > 0.0 =>
                {
                    Some([x, y, w, h])
                }
                _ => return Err(SvgRefusal::Size),
            }
        }
    };
    let (width, height) = match (pixels("width"), pixels("height"), view_box) {
        (Some(w), Some(h), _) => (w, h),
        (None, None, Some([_, _, w, h]))
            if root.attribute("width").is_none() && root.attribute("height").is_none() =>
        {
            (w, h)
        }
        _ => return Err(SvgRefusal::Size),
    };
    let viewport = Viewport {
        width,
        height,
        view_box,
        fit: fit_of(root.attribute("preserveAspectRatio")).ok_or(SvgRefusal::Aspect)?,
    };
    viewport.facts()?;
    Ok(viewport)
}

fn clip(text: &str) -> String {
    text.chars().take(64).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 190 108" width="1024px" height="582.06px" style="width: 1024px; height: 582.06px;">"#;

    fn doc(body: &str) -> String {
        format!("{HEAD}<title>t</title>{body}</svg>")
    }

    #[test]
    fn passes_plain_drawing_and_reads_its_size() {
        let svg = doc(
            r##"<defs><linearGradient id="a"><stop offset="0" stop-color="#fff"/></linearGradient></defs>
            <g transform="scale(2)"><rect width="10" height="10" fill="url(#a)"/>
            <path d="M 0 0 H 9 V 9 Z" fill="#FE7867" fill-rule="evenodd"/></g>"##,
        );
        assert_eq!(
            check_svg(svg.as_bytes()),
            Ok(SvgFacts {
                width: 1024,
                height: 582
            })
        );
        let bare = r#"<?xml version="1.0" encoding="UTF-8"?><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 20"/>"#;
        assert_eq!(
            check_svg(bare.as_bytes()),
            Ok(SvgFacts {
                width: 40,
                height: 20
            })
        );
    }

    #[test]
    fn refuses_anything_that_runs_loads_or_styles() {
        let refused = |body: &str| check_svg(doc(body).as_bytes()).map(|_| ()).unwrap_err();
        assert_eq!(
            refused("<script>1</script>"),
            SvgRefusal::Element("script".into())
        );
        assert_eq!(
            refused(r#"<rect onload="1" width="1" height="1"/>"#),
            SvgRefusal::Attribute("onload".into())
        );
        assert_eq!(
            refused("<style>*{}</style>"),
            SvgRefusal::Sheet("has a selector other than an element, class or id")
        );
        assert_eq!(
            refused(r#"<image width="1" height="1"/>"#),
            SvgRefusal::Element("image".into())
        );
        assert_eq!(
            refused(r#"<rect fill="url(https://x/y#a)" width="1" height="1"/>"#),
            SvgRefusal::Value("fill".into())
        );
        assert_eq!(
            refused(r#"<rect style="fill: url( 'data:x')" width="1" height="1"/>"#),
            SvgRefusal::Attribute("style".into())
        );
        assert_eq!(
            refused(r#"<rect style="fill: url(data:x)" width="1" height="1"/>"#),
            SvgRefusal::Value("style".into())
        );
        assert_eq!(
            refused(r#"<rect style="behavior: x" width="1" height="1"/>"#),
            SvgRefusal::Attribute("style behavior".into())
        );
        assert_eq!(
            refused(r##"<text><textPath href="#p">hi</textPath></text>"##),
            SvgRefusal::Element("textPath".into())
        );
        assert_eq!(refused("<g>hi</g>"), SvgRefusal::Text);
        assert_eq!(refused("<?php x ?>"), SvgRefusal::Instruction);
        assert!(matches!(
            refused(r##"<a xmlns:x="urn:x" x:href="#a"/>"##),
            SvgRefusal::Element(_)
        ));
        assert_eq!(
            refused(r##"<rect xmlns:x="http://www.w3.org/1999/xlink" x:href="#a"/>"##),
            SvgRefusal::Attribute("href".into())
        );
        assert_eq!(
            refused(r#"<foreignObject xmlns="http://www.w3.org/1999/xhtml"/>"#),
            SvgRefusal::Element("foreignObject".into())
        );
    }

    #[test]
    fn refuses_a_value_that_escapes_or_calls_another_function() {
        let refused = |body: &str| check_svg(doc(body).as_bytes()).map(|_| ()).unwrap_err();
        // CSS reads both as url(…), which a document opened on its own would load.
        assert_eq!(
            refused(r#"<rect fill="u\72l(http://x/y#a)" width="1" height="1"/>"#),
            SvgRefusal::Value("fill".into())
        );
        assert_eq!(
            refused(r#"<rect clip-path="u\rl(http://x/y#c)" width="1" height="1"/>"#),
            SvgRefusal::Value("clip-path".into())
        );
        // No escape at all, even one that calls nothing.
        assert_eq!(
            refused(r##"<rect fill="#f\30 0" width="1" height="1"/>"##),
            SvgRefusal::Value("fill".into())
        );
        for value in [
            "src(http://x/y)",
            "image-set(x)",
            "var(--x)",
            "attr(x)",
            "(x)",
            "\u{ff55}rl(http://x/y)",
        ] {
            assert_eq!(
                refused(&format!(r#"<rect stroke="{value}" width="1" height="1"/>"#)),
                SvgRefusal::Value("stroke".into()),
                "{value}"
            );
        }
        let plain = doc(
            r##"<g transform="rotate(30 5 5) translate(1,2) skewX(3)" aria-label="a (b) \c">
            <rect fill="RGB(1, 2, 3)" stroke="url( '#a')" style="stop-color: hsla(1, 2%, 3%, 0.5)" width="1" height="1"/></g>"##,
        );
        assert!(check_svg(plain.as_bytes()).is_ok());
    }

    #[test]
    fn reads_the_roots_fit_and_refuses_one_it_does_not_know() {
        let root = |fit: &str| {
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1" width="1" height="1" preserveAspectRatio="{fit}"/>"#
            )
        };
        for fit in ["none", "xMinYMax", "xMidYMid meet", " xMaxYMin  slice "] {
            assert!(check_svg(root(fit).as_bytes()).is_ok(), "{fit}");
        }
        for fit in [
            "",
            "defer xMidYMid",
            "xMidYMid cover",
            "xmidymid",
            "xMidYMid meet slice",
        ] {
            assert_eq!(
                check_svg(root(fit).as_bytes()),
                Err(SvgRefusal::Aspect),
                "{fit}"
            );
        }
        let bad_box =
            br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 0 1" width="1" height="1"/>"#;
        assert_eq!(check_svg(bad_box), Err(SvgRefusal::Size));
    }

    fn picture(svg: &str) -> String {
        String::from_utf8(picture_svg(svg.as_bytes()).unwrap()).unwrap()
    }

    #[test]
    fn writes_a_picture_that_stretches_and_shows_what_its_viewport_showed() {
        let body = r##"><rect width="100" height="100" fill="#FE7867"/></svg>"##;
        let svg = |root: &str| format!(r#"<svg xmlns="http://www.w3.org/2000/svg" {root}{body}"#);
        // The same proportions: the viewBox stays as written.
        assert_eq!(
            picture(&svg(r#"viewBox="0 0 100 100" width="100" height="100""#)),
            svg(r#"viewBox="0 0 100 100" width="100" height="100" preserveAspectRatio="none""#)
        );
        // Wider than its viewBox, centred (the default): the viewport showed 50 units of slack each side.
        assert_eq!(
            picture(&svg(r#"width="200" height="100" viewBox="0 0 100 100""#)),
            svg(r#"width="200" height="100" viewBox="-50 0 200 100" preserveAspectRatio="none""#)
        );
        // Covering, at the bottom: the viewport showed the lower half.
        assert_eq!(
            picture(&svg(
                r#"width="200" height="100" preserveAspectRatio="xMinYMax slice" viewBox="0 0 100 100""#
            )),
            svg(r#"width="200" height="100" viewBox="0 50 100 50" preserveAspectRatio="none""#)
        );
        // No viewBox: its pixels.
        assert_eq!(
            picture(&svg(r#"width="40px" height="20""#)),
            svg(r#"width="40px" height="20" viewBox="0 0 40 20" preserveAspectRatio="none""#)
        );
        // Already stretching: the bytes as they are, the byte-order mark dropped.
        let stretched = svg(r#"viewBox="0 0 1 1" preserveAspectRatio="none""#);
        assert_eq!(picture(&format!("\u{feff}{stretched}")), stretched);
        // What it writes passes, at the same size; what the check refuses, it refuses.
        let written = picture(&svg(r#"width="200" height="100" viewBox="0 0 100 100""#));
        assert_eq!(
            check_svg(written.as_bytes()),
            Ok(SvgFacts {
                width: 200,
                height: 100
            })
        );
        assert_eq!(
            picture_svg(svg(r#"width="2" height="1" preserveAspectRatio="defer""#).as_bytes()),
            Err(SvgRefusal::Aspect)
        );
        assert_eq!(
            picture_svg(doc("<script/>").as_bytes()),
            Err(SvgRefusal::Element("script".into()))
        );
    }

    /// A WOFF2 head (`wOF2`, then version 1) in base64, as an embedded font source starts.
    const WOFF2: &str = "d09GMgABAAAAAA";

    /// A drawing as Excalidraw exports one: a public DOCTYPE, a comment, empty metadata and masks, a sheet of an
    /// embedded font and a class rule, and text.
    fn labelled() -> String {
        format!(
            r##"<?xml version="1.0" standalone="no"?><!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd"><svg xmlns="http://www.w3.org/2000/svg" version="1.1" viewBox="0 0 200 100" width="200px" height="100px" preserveAspectRatio="none"><!-- svg-source:probe --><metadata/><defs><style class="style-fonts" type="text/css">
      @font-face {{ font-family: "Probe Hand"; src: url(data:font/woff2;base64,{WOFF2}) format("woff2"); font-display: swap; }}
      .axis, text.tick#t1 {{ stroke: #04455E; stroke-width: 2 }}</style></defs><mask/><g class="axis"><path d="M 10 90 H 190"/></g><text x="10" y="80" xml:space="preserve" font-family="Probe Hand, sans-serif" font-size="16px" text-anchor="start" dominant-baseline="alphabetic" direction="ltr" style="white-space: pre;">Cobras <tspan dx="2" font-weight="bold">100</tspan></text></svg>"##
        )
    }

    #[test]
    fn passes_text_embedded_fonts_and_an_external_doctype_which_the_picture_drops() {
        let svg = labelled();
        assert_eq!(
            check_svg(svg.as_bytes()),
            Ok(SvgFacts {
                width: 200,
                height: 100
            })
        );
        let written = picture(&svg);
        assert!(!written.contains("DOCTYPE"), "{written}");
        assert_eq!(
            written,
            svg.replace(
                r#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">"#,
                ""
            )
        );
        // A system identifier, after a comment, and a picture rewritten to stretch.
        let system = r#"<!-- x --> <!DOCTYPE svg SYSTEM 'svg.dtd' ><svg xmlns="http://www.w3.org/2000/svg" width="20" height="10" viewBox="0 0 10 10"/>"#;
        assert!(check_svg(system.as_bytes()).is_ok());
        assert_eq!(
            picture(system),
            r#"<!-- x --> <svg xmlns="http://www.w3.org/2000/svg" width="20" height="10" viewBox="-5 0 20 10" preserveAspectRatio="none"/>"#
        );
    }

    #[test]
    fn refuses_a_doctype_that_declares_or_names_anything_else() {
        let body = r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>"#;
        for doctype in [
            r#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "x.dtd" [<!ENTITY a "b">]>"#,
            r#"<!DOCTYPE svg SYSTEM "x.dtd" []>"#,
            r#"<!DOCTYPE svg SYSTEM "x[.dtd">"#,
            r#"<!DOCTYPE html>"#,
            r#"<!DOCTYPE svg>"#,
            r#"<!DOCTYPE svg PUBLIC "x">"#,
            r#"<!doctype svg SYSTEM "x.dtd">"#,
            r#"<!DOCTYPE svg SYSTEM "x.dtd"><!DOCTYPE svg SYSTEM "x.dtd">"#,
        ] {
            assert!(
                matches!(
                    check_svg(format!("{doctype}{body}").as_bytes()),
                    Err(SvgRefusal::Xml(_))
                ),
                "{doctype}"
            );
        }
    }

    #[test]
    fn refuses_a_sheet_that_loads_reaches_outside_or_is_not_read() {
        let face = |descriptors: &str| format!("<style>@font-face {{ {descriptors} }}</style>");
        let src = format!("src: url(data:font/woff2;base64,{WOFF2})");
        let refused = |body: &str| check_svg(doc(body).as_bytes()).map(|_| ()).unwrap_err();
        assert!(check_svg(doc(&face(&format!("font-family: A; {src}"))).as_bytes()).is_ok());
        // A format hint is a plain name, bare or in one pair of matching quotes.
        for hint in [r#"format("woff2")"#, "format('woff2')", "format(woff2)"] {
            assert!(
                check_svg(doc(&face(&format!("{src} {hint}"))).as_bytes()).is_ok(),
                "{hint}"
            );
        }
        for (body, reason) in [
            (
                "<style>@import url(x.css);</style>",
                "has a rule without a block",
            ),
            (
                "<style>@media print { }</style>",
                "has an at-rule other than @font-face",
            ),
            (
                "<style>@font-face { font-family: A }</style>",
                "has a font face without a source",
            ),
            (
                &face("src: url(https://x/y.woff2)"),
                "has a font descriptor whose value is not shown",
            ),
            (
                &face("src: local(Arial)"),
                "has a font descriptor whose value is not shown",
            ),
            (
                &face(r#"src: url("data:image/svg+xml;base64,d09GMgABAAAAAA")"#),
                "has a font descriptor whose value is not shown",
            ),
            // Base64 of bytes that are no font.
            (
                &face("src: url(data:font/woff2;base64,AAAAAAAAAAAA)"),
                "has a font descriptor whose value is not shown",
            ),
            (
                &face(&format!("{src} format(x y)")),
                "has a font descriptor whose value is not shown",
            ),
            // A quote left open would read the rest of the sheet into a string, so the browser would read less
            // than the check did.
            (
                &face(&format!("{src} format('woff2)")),
                "has a font descriptor whose value is not shown",
            ),
            (
                &face(&format!(r#"{src} format("woff2')"#)),
                "has a font descriptor whose value is not shown",
            ),
            (
                &face(&format!("{src} format('''woff2)")),
                "has a font descriptor whose value is not shown",
            ),
            (
                &face(&format!("{src} format(woff2')")),
                "has a font descriptor whose value is not shown",
            ),
            (
                &face(&format!("src: url('data:font/woff2;base64,{WOFF2})")),
                "has a font descriptor whose value is not shown",
            ),
            (
                &face(&format!("{src}; font-family: 'A")),
                "has a font descriptor whose value is not shown",
            ),
            (
                &face(&format!("{src}; {src}")),
                "has a font descriptor not shown",
            ),
            (
                &face(&format!("{src}; font-feature-settings: x")),
                "has a font descriptor not shown",
            ),
            (
                &face(&format!("{src}; font-family: a(b)")),
                "has a font descriptor whose value is not shown",
            ),
            (
                &face(&format!("{src}; font-family: \"a\" b\"")),
                "has a font descriptor whose value is not shown",
            ),
            (
                &face(&format!("{src}; font-weight: var(--w)")),
                "has a font descriptor whose value is not shown",
            ),
            (
                "<style>rect { fill: u\\72l(http://x/y) }</style>",
                "holds an escape, a comment or markup",
            ),
            (
                "<style>rect { fill: red !important }</style>",
                "holds an escape, a comment or markup",
            ),
            (
                "<style>/* x */</style>",
                "holds an escape, a comment or markup",
            ),
            (
                "<style><![CDATA[<x>]]></style>",
                "holds an escape, a comment or markup",
            ),
            (
                "<style>svg rect { fill: red }</style>",
                "has a selector other than an element, class or id",
            ),
            (
                "<style>a:hover { fill: red }</style>",
                "has a selector other than an element, class or id",
            ),
            (
                "<style>[x] { fill: red }</style>",
                "has a selector other than an element, class or id",
            ),
            (
                "<style>.a. { fill: red }</style>",
                "has a selector other than an element, class or id",
            ),
            (
                "<style>1a { fill: red }</style>",
                "has a selector other than an element, class or id",
            ),
            (
                "<style>.a { fill: url(https://x/y) }</style>",
                "sets a property or value a style may not",
            ),
            (
                "<style>.a { behavior: x }</style>",
                "sets a property or value a style may not",
            ),
            (
                "<style>.a { fill: red } .b { fill: red</style>",
                "has a block that is not closed",
            ),
            (
                "<style>.a { .b { fill: red } }</style>",
                "has a nested or broken rule",
            ),
            (
                "<style>.a { fill: red</style>",
                "has a block that is not closed",
            ),
            (
                "<style>.a { fill: red } }</style>",
                "has a rule without a block",
            ),
            (
                &face(&format!("{src}; font-family: a)")),
                "has an unbalanced parenthesis",
            ),
            ("<style><title>x</title></style>", "holds an element"),
        ] {
            assert_eq!(refused(body), SvgRefusal::Sheet(reason), "{body}");
        }
        assert_eq!(
            refused(r#"<style type="text/xsl"></style>"#),
            SvgRefusal::Value("type".into())
        );
        assert_eq!(
            refused(r#"<style media="print"></style>"#),
            SvgRefusal::Attribute("media".into())
        );
        assert_eq!(
            refused(r#"<rect type="text/css" width="1" height="1"/>"#),
            SvgRefusal::Attribute("type".into())
        );
        assert_eq!(
            refused(r#"<text xml:space="keep">x</text>"#),
            SvgRefusal::Value("xml:space".into())
        );
        assert_eq!(
            refused(r#"<text xml:lang="en">x</text>"#),
            SvgRefusal::Attribute("lang".into())
        );
        assert_eq!(
            refused(r#"<text font-family="u\72l(http://x/y)">x</text>"#),
            SvgRefusal::Value("font-family".into())
        );
        // Text is drawn only from a text element; a label's stays a label.
        assert_eq!(refused("<mask>x</mask>"), SvgRefusal::Text);
        assert!(check_svg(doc("<metadata>made by a probe</metadata>").as_bytes()).is_ok());
    }

    #[test]
    fn reads_a_font_familys_first_family_as_css_matches_it() {
        for (value, family) in [
            ("Probe Hand, sans-serif", Some("probe hand")),
            ("  probe\t HAND ", Some("probe hand")),
            ("'Probe  Hand'", Some("probe  hand")),
            ("\"serif\", x", Some("serif")),
            ("Noto Serif", Some("noto serif")),
            ("-x", Some("-x")),
            ("sans-serif, Probe Hand", None),
            ("SERIF", None),
            ("inherit", None),
            ("Probe default", None),
            ("1x", None),
            ("-1x", None),
            ("'x", None),
            ("'x\"", None),
            ("''", None),
            ("", None),
        ] {
            assert_eq!(first_family(value).as_deref(), family, "{value:?}");
        }
    }

    #[test]
    fn passes_text_only_in_a_font_its_sheet_embeds() {
        let sheet = |extra: &str| {
            format!(
                "<style>@font-face {{ font-family: \"Probe Hand\"; src: url(data:font/woff2;base64,{WOFF2}) }} {extra}</style>"
            )
        };
        let with = |extra: &str, body: &str| doc(&format!("{}{body}", sheet(extra)));
        // The family set on the text, an ancestor, a `style`, a rule, quoted or in another case; text of white space
        // only, and a label's text, draw no glyph.
        for svg in [
            with("", r#"<text font-family="Probe Hand, sans-serif">x</text>"#),
            with("", r#"<text font-family="'probe hand'">x</text>"#),
            with(
                "",
                r#"<g font-family="Probe Hand"><g><text>x<tspan>y</tspan></text></g></g>"#,
            ),
            with(
                "",
                r#"<text style="font-family: PROBE  hand; font-size: 9px">x</text>"#,
            ),
            with(
                ".l { font-family: Probe Hand }",
                r#"<text class="k l">x</text>"#,
            ),
            with(
                "text#t { font-family: Probe Hand }",
                r#"<text id="t">x</text>"#,
            ),
            with(
                "",
                r#"<text font-family="Other"><tspan font-family="Probe Hand">x</tspan> </text>"#,
            ),
            doc("<text> </text><metadata>x</metadata>"),
        ] {
            assert!(check_svg(svg.as_bytes()).is_ok(), "{svg}");
        }
        // No family set (the viewer's default font), a family not embedded, a generic or keyword first, a tspan in
        // another family, and a rule or `style` that overrides an embedded family with another.
        for svg in [
            doc("<text>x</text>"),
            with("", "<text>x</text>"),
            with("", r#"<text font-family="Helvetica">x</text>"#),
            with("", r#"<text font-family="Helvetica, Probe Hand">x</text>"#),
            with("", r#"<text font-family="sans-serif">x</text>"#),
            with("", r#"<text font-family="inherit">x</text>"#),
            with(
                "",
                r#"<text font-family="Probe Hand">x<tspan font-family="Other">y</tspan></text>"#,
            ),
            with(
                "",
                r#"<g font-family="Probe Hand"><g font-family="Other"><text>x</text></g></g>"#,
            ),
            with(
                "text { font-family: Other }",
                r#"<text font-family="Probe Hand">x</text>"#,
            ),
            with(
                "",
                r#"<text font-family="Probe Hand" style="font-family: Other">x</text>"#,
            ),
            with(
                ".l { font-family: Other }",
                r#"<g font-family="Probe Hand"><text class="l">x</text></g>"#,
            ),
            // A face whose name is a generic family names no face.
            doc(&format!(
                r#"<style>@font-face {{ font-family: serif; src: url(data:font/woff2;base64,{WOFF2}) }}</style><text font-family="serif">x</text>"#
            )),
        ] {
            assert_eq!(check_svg(svg.as_bytes()), Err(SvgRefusal::Font), "{svg}");
        }
        // What decides an element's font is read rule by rule, so the rules that set one are counted.
        let rules = |n: usize| -> String {
            (0..n)
                .map(|i| format!(".c{i} {{ font-family: A }}"))
                .collect()
        };
        assert_eq!(
            check_svg(doc(&sheet(&rules(65))).as_bytes()),
            Err(SvgRefusal::Sheet("sets a font family in too many rules"))
        );
        assert!(check_svg(doc(&sheet(&rules(64))).as_bytes()).is_ok());
    }

    #[test]
    fn refuses_unreadable_or_unsized_documents() {
        let doctype = format!(r#"<!DOCTYPE svg [<!ENTITY a "<script/>">]>{}"#, doc(""));
        assert!(matches!(
            check_svg(doctype.as_bytes()),
            Err(SvgRefusal::Xml(_))
        ));
        let latin = format!(r#"<?xml version="1.0" encoding="ISO-8859-1"?>{}"#, doc(""));
        assert_eq!(check_svg(latin.as_bytes()), Err(SvgRefusal::Encoding));
        assert_eq!(check_svg(b"<svg \xff/>"), Err(SvgRefusal::Encoding));
        assert_eq!(
            check_svg(br#"<svg xmlns="http://www.w3.org/2000/svg" width="10cm" height="5cm"/>"#),
            Err(SvgRefusal::Size)
        );
        assert_eq!(
            check_svg(br#"<svg xmlns="http://www.w3.org/2000/svg" width="100%" height="100%" viewBox="0 0 1 1"/>"#),
            Err(SvgRefusal::Size)
        );
        assert_eq!(
            check_svg(br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#),
            Err(SvgRefusal::Size)
        );
        assert_eq!(
            check_svg(br#"<svg width="1" height="1"/>"#),
            Err(SvgRefusal::Root)
        );
        let big = vec![b' '; (MAX_SVG_BYTES + 1) as usize];
        assert_eq!(check_svg(&big), Err(SvgRefusal::TooLarge));
    }
}
