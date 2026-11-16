//! The ODF vocabulary the frontend reads: namespaces, an element tree built from a parsed part, element and
//! attribute readers, and style-name decoding.
//!
//! Elements match by namespace URI. Attributes match by their canonical prefix (`draw:name`): every attribute of a
//! namespace the frontend knows is named by that namespace's prefix, whatever prefix the writer bound; one of an
//! unknown namespace is `{uri}local`. Text is kept as written, a comment or processing instruction inside a run of
//! text not breaking it.

use std::collections::BTreeMap;

use slidock_canon::is_js_whitespace;
use slidock_io::roxmltree::{Document, Node};
use slidock_io::xml::{attributes, namespace_uri, qualified_name};

use crate::names::QName;

/// The namespaces the frontend reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ns {
    Office,
    Style,
    Text,
    Table,
    Draw,
    Fo,
    Xlink,
    Meta,
    Presentation,
    Svg,
    Dr3d,
    Anim,
    Smil,
    Script,
    Manifest,
    Loext,
    Xml,
}

/// Each namespace's URI and canonical prefix.
const NAMESPACES: [(Ns, &str, &str); 17] = [
    (
        Ns::Office,
        "urn:oasis:names:tc:opendocument:xmlns:office:1.0",
        "office",
    ),
    (
        Ns::Style,
        "urn:oasis:names:tc:opendocument:xmlns:style:1.0",
        "style",
    ),
    (
        Ns::Text,
        "urn:oasis:names:tc:opendocument:xmlns:text:1.0",
        "text",
    ),
    (
        Ns::Table,
        "urn:oasis:names:tc:opendocument:xmlns:table:1.0",
        "table",
    ),
    (
        Ns::Draw,
        "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0",
        "draw",
    ),
    (
        Ns::Fo,
        "urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0",
        "fo",
    ),
    (Ns::Xlink, "http://www.w3.org/1999/xlink", "xlink"),
    (
        Ns::Meta,
        "urn:oasis:names:tc:opendocument:xmlns:meta:1.0",
        "meta",
    ),
    (
        Ns::Presentation,
        "urn:oasis:names:tc:opendocument:xmlns:presentation:1.0",
        "presentation",
    ),
    (
        Ns::Svg,
        "urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0",
        "svg",
    ),
    (
        Ns::Dr3d,
        "urn:oasis:names:tc:opendocument:xmlns:dr3d:1.0",
        "dr3d",
    ),
    (
        Ns::Anim,
        "urn:oasis:names:tc:opendocument:xmlns:animation:1.0",
        "anim",
    ),
    (
        Ns::Smil,
        "urn:oasis:names:tc:opendocument:xmlns:smil-compatible:1.0",
        "smil",
    ),
    (
        Ns::Script,
        "urn:oasis:names:tc:opendocument:xmlns:script:1.0",
        "script",
    ),
    (
        Ns::Manifest,
        "urn:oasis:names:tc:opendocument:xmlns:manifest:1.0",
        "manifest",
    ),
    (
        Ns::Loext,
        "urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0",
        "loext",
    ),
    (Ns::Xml, "http://www.w3.org/XML/1998/namespace", "xml"),
];

impl Ns {
    pub(crate) fn uri(self) -> &'static str {
        NAMESPACES
            .iter()
            .find(|(ns, _, _)| *ns == self)
            .map_or("", |(_, uri, _)| uri)
    }
}

/// The canonical prefix of every namespace the frontend knows, by URI.
fn canonical_prefixes() -> [(&'static str, &'static str); 17] {
    NAMESPACES.map(|(_, uri, prefix)| (uri, prefix))
}

/// An element or a run of text.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum XmlNode {
    Element(Element),
    Text(String),
}

/// An element of a part: its names, its attributes by canonical name, and its children in document order.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Element {
    /// The qualified name as written: `draw:frame`.
    pub name: String,
    pub local: String,
    /// The namespace URI its prefix resolves to; `""` for none.
    pub uri: String,
    /// Attributes by canonical name, namespace declarations included.
    pub attrs: BTreeMap<String, String>,
    pub children: Vec<XmlNode>,
}

impl Element {
    /// The root element of a parsed part.
    pub(crate) fn from_document(document: &Document<'_>) -> Element {
        let prefixes = canonical_prefixes();
        convert(document.root_element(), &prefixes)
    }

    pub(crate) fn is(&self, ns: Ns, local: &str) -> bool {
        self.local == local && self.uri == ns.uri()
    }

    pub(crate) fn in_namespace(&self, ns: Ns) -> bool {
        self.uri == ns.uri()
    }

    /// The element children, skipping text.
    pub(crate) fn kids(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|node| match node {
            XmlNode::Element(element) => Some(element),
            XmlNode::Text(_) => None,
        })
    }

    pub(crate) fn child(&self, ns: Ns, local: &str) -> Option<&Element> {
        self.kids().find(|candidate| candidate.is(ns, local))
    }

    pub(crate) fn children_of<'s, 'n>(
        &'s self,
        ns: Ns,
        local: &'n str,
    ) -> impl Iterator<Item = &'s Element> + use<'s, 'n> {
        self.kids().filter(move |candidate| candidate.is(ns, local))
    }

    /// An attribute by its canonical qualified name (`draw:style-name`).
    pub(crate) fn attr(&self, name: QName) -> Option<&str> {
        self.attrs.get(name.as_str()).map(String::as_str)
    }

    /// `xsd:boolean`.
    pub(crate) fn bool_attr(&self, name: QName) -> Option<bool> {
        match self.attr(name).map(slidock_canon::js_trim) {
            Some("true" | "1") => Some(true),
            Some("false" | "0") => Some(false),
            _ => None,
        }
    }

    /// An integer of at most 15 digits, with an optional sign, between white space.
    pub(crate) fn int_attr(&self, name: QName) -> Option<i64> {
        parse_int(self.attr(name)?)
    }

    /// The concatenated text of the element and its descendants.
    pub(crate) fn text_of(&self) -> String {
        let mut out = String::new();
        self.collect_text(&mut out);
        out
    }

    fn collect_text(&self, out: &mut String) {
        for node in &self.children {
            match node {
                XmlNode::Text(text) => out.push_str(text),
                XmlNode::Element(element) => element.collect_text(out),
            }
        }
    }

    /// Each element child with its origin path: the qualified name and 1-based position among its namesakes.
    pub(crate) fn positioned<'s>(&'s self, path: &str) -> Vec<(&'s Element, String)> {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        self.kids()
            .map(|element| {
                let n = counts.entry(element.name.as_str()).or_insert(0);
                *n += 1;
                (element, format!("{path}/{}[{n}]", element.name))
            })
            .collect()
    }
}

fn convert(node: Node<'_, '_>, prefixes: &[(&str, &str)]) -> Element {
    let name = qualified_name(node).to_owned();
    let mut element = Element {
        local: node.tag_name().name().to_owned(),
        uri: namespace_uri(node).to_owned(),
        attrs: attributes(node, Some(prefixes)).into_iter().collect(),
        children: Vec::new(),
        name,
    };
    for child in node.children() {
        if child.is_element() {
            element
                .children
                .push(XmlNode::Element(convert(child, prefixes)));
        } else if child.is_text() {
            let text = child.text().unwrap_or_default();
            match element.children.last_mut() {
                Some(XmlNode::Text(last)) => last.push_str(text),
                _ => element.children.push(XmlNode::Text(text.to_owned())),
            }
        }
    }
    element
}

/// `^\s*[-+]?\d{1,15}\s*$` as a number.
fn parse_int(value: &str) -> Option<i64> {
    let trimmed = value.trim_matches(is_js_whitespace);
    let digits = trimmed
        .strip_prefix('-')
        .or_else(|| trimmed.strip_prefix('+'))
        .unwrap_or(trimmed);
    if digits.is_empty() || digits.len() > 15 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let magnitude: i64 = digits.parse().ok()?;
    Some(if trimmed.starts_with('-') {
        -magnitude
    } else {
        magnitude
    })
}

/// A style name as displayed: ODF writers escape characters an XML name cannot hold as `_xx_` (hex), so
/// `TITLE_5f_AND_5f_BODY` is `TITLE_AND_BODY` and `msArrowEnd_20_5` is `msArrowEnd 5`.
pub(crate) fn decode_style_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(c) = rest.chars().next() {
        let bytes = rest.as_bytes();
        if let [b'_', high, low, b'_', ..] = bytes
            && high.is_ascii_hexdigit()
            && low.is_ascii_hexdigit()
        {
            let code = hex_value(*high) * 16 + hex_value(*low);
            out.push(char::from(code));
            rest = rest.get(4..).unwrap_or_default();
            continue;
        }
        out.push(c);
        rest = rest.get(c.len_utf8()..).unwrap_or_default();
    }
    out
}

fn hex_value(digit: u8) -> u8 {
    match digit {
        b'0'..=b'9' => digit - b'0',
        b'a'..=b'f' => digit - b'a' + 10,
        b'A'..=b'F' => digit - b'A' + 10,
        _ => 0,
    }
}

/// A part parsed from text, for tests.
#[cfg(test)]
pub(crate) fn parse(text: &str) -> Element {
    let limits = slidock_io::XmlLimits::default();
    let mut budget = slidock_io::XmlBudget::new(&limits);
    let document =
        slidock_io::parse_xml(text, &limits, &mut budget).unwrap_or_else(|e| panic!("{e}"));
    Element::from_document(&document)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_text_runs_whole_across_comments_and_names_attributes_canonically() {
        let root = parse(concat!(
            r#"<d:a xmlns:d="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:o="urn:example" "#,
            r#"d:name="x" o:name="y" plain="z">one <!-- c --> two<![CDATA[ three]]><d:b/></d:a>"#
        ));
        assert!(root.is(Ns::Draw, "a"));
        assert_eq!(root.name, "d:a");
        assert_eq!(root.attr(crate::names::qn!("draw:name")), Some("x"));
        assert_eq!(root.attrs["{urn:example}name"], "y");
        assert_eq!(root.attrs["plain"], "z");
        assert_eq!(
            root.children.first(),
            Some(&XmlNode::Text("one  two three".to_owned()))
        );
        assert_eq!(root.positioned("/r")[0].1, "/r/d:b[1]");
    }

    #[test]
    fn decodes_escaped_style_names() {
        assert_eq!(decode_style_name("TITLE_5f_AND_5f_BODY"), "TITLE_AND_BODY");
        assert_eq!(decode_style_name("msArrowEnd_20_5"), "msArrowEnd 5");
        assert_eq!(decode_style_name("_5f_5f_"), "_5f_");
        assert_eq!(decode_style_name("a_zz_b_e9_"), "a_zz_bé");
        assert_eq!(decode_style_name("é_41_"), "éA");
    }

    #[test]
    fn reads_integers_strictly() {
        assert_eq!(parse_int(" +12 "), Some(12));
        assert_eq!(parse_int("-3"), Some(-3));
        assert_eq!(parse_int("123456789012345"), Some(123_456_789_012_345));
        assert_eq!(parse_int("1234567890123456"), None);
        assert_eq!(parse_int("1.5"), None);
        assert_eq!(parse_int(""), None);
        assert_eq!(parse_int("\u{a0}7\u{2028}"), Some(7));
    }
}
