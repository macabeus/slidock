//! The serializer: the bytes `hast-util-to-html` 9 writes with these options (numeric character references, a lowercase
//! doctype, no self-closing slashes). Text escapes only `&` and `<`, attribute values only `"`, `&`, `'`, `` ` `` and
//! NUL, each as an uppercase hex reference; everything else, non-ASCII included, is written raw. HTML void elements
//! have no end tag; inside `<svg>` nothing is void.

use crate::tree::{Attr, Attrs, Element, Node};

/// HTML's void elements, which have no end tag when they have no children.
const VOIDS: &[&str] = &[
    "area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "image", "img",
    "input", "keygen", "link", "meta", "param", "source", "track", "wbr",
];

/// Attributes HTML reads as booleans: the printers only ever give them a bare value.
const BOOLEANS: &[&str] = &["hidden", "muted", "loop", "playsinline", "autoplay"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Space {
    Html,
    Svg,
}

/// A document: the doctype, a newline, the `<html>` element and a final newline.
pub fn document(html: &Element) -> String {
    let mut out = String::from("<!doctype html>\n");
    element(html, Space::Html, &mut out);
    out.push('\n');
    out
}

/// One element and its subtree, as it prints inside an HTML document.
pub fn fragment(node: &Element) -> String {
    let mut out = String::new();
    element(node, Space::Html, &mut out);
    out
}

fn element(node: &Element, space: Space, out: &mut String) {
    let inner = if space == Space::Html && node.tag == "svg" {
        Space::Svg
    } else {
        space
    };
    let void = space == Space::Html && node.children.is_empty() && VOIDS.contains(&node.tag);
    out.push('<');
    out.push_str(node.tag);
    attributes(&node.attrs, out);
    out.push('>');
    let raw = node.tag == "style" || node.tag == "script";
    for child in &node.children {
        match child {
            Node::Element(child) => element(child, inner, out),
            Node::Text(value) if raw => out.push_str(value),
            Node::Text(value) => escape_text(value, out),
        }
    }
    if !void {
        out.push_str("</");
        out.push_str(node.tag);
        out.push('>');
    }
}

fn attributes(attrs: &Attrs, out: &mut String) {
    for (name, value) in attrs.iter() {
        out.push(' ');
        out.push_str(name);
        match value {
            Attr::Bare => {}
            Attr::Value(value) => {
                debug_assert!(
                    !BOOLEANS.contains(name),
                    "boolean attribute {name} with a value"
                );
                out.push_str("=\"");
                escape_attribute(value, out);
                out.push('"');
            }
        }
    }
}

fn escape_text(value: &str, out: &mut String) {
    for c in value.chars() {
        match c {
            '&' => out.push_str("&#x26;"),
            '<' => out.push_str("&#x3C;"),
            _ => out.push(c),
        }
    }
}

fn escape_attribute(value: &str, out: &mut String) {
    for c in value.chars() {
        match c {
            '"' => out.push_str("&#x22;"),
            '&' => out.push_str("&#x26;"),
            '\'' => out.push_str("&#x27;"),
            '`' => out.push_str("&#x60;"),
            '\0' => out.push_str("&#x0;"),
            _ => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{Attrs, text};

    fn el(tag: &'static str, attrs: Attrs, children: Vec<Node>) -> Element {
        Element::new(tag, attrs, children)
    }

    #[test]
    fn escapes_text_and_attribute_values_with_hex_references() {
        let node = el(
            "p",
            Attrs::new().val("title", "a\"b&c'd`e\0f<g>h\u{a0}".to_owned()),
            vec![text("x & y < z > \" ' \u{a0}😀")],
        );
        assert_eq!(
            fragment(&node),
            "<p title=\"a&#x22;b&#x26;c&#x27;d&#x60;e&#x0;f<g>h\u{a0}\">x &#x26; y &#x3C; z > \" ' \u{a0}😀</p>"
        );
    }

    #[test]
    fn writes_bare_attributes_and_voids_without_end_tags_or_slashes() {
        let node = el(
            "div",
            Attrs::new().flag("data-x", true).flag("hidden", true),
            vec![
                Element::empty("br").into(),
                el("img", Attrs::new().val("alt", String::new()), vec![]).into(),
                el("col", Attrs::new(), vec![]).into(),
            ],
        );
        assert_eq!(
            fragment(&node),
            "<div data-x hidden><br><img alt=\"\"><col></div>"
        );
    }

    #[test]
    fn closes_every_element_inside_svg_and_returns_to_html_after_it() {
        let svg = el(
            "svg",
            Attrs::new().val("viewBox", "0 0 1 1".to_owned()),
            vec![
                Element::empty("path").into(),
                Element::empty("image").into(),
            ],
        );
        let node = el(
            "div",
            Attrs::new(),
            vec![svg.into(), Element::empty("img").into()],
        );
        assert_eq!(
            fragment(&node),
            "<div><svg viewBox=\"0 0 1 1\"><path></path><image></image></svg><img></div>"
        );
    }

    #[test]
    fn leaves_style_text_raw_and_escapes_title_text() {
        let node = el(
            "head",
            Attrs::new(),
            vec![
                el("title", Attrs::new(), vec![text("a<b&c")]).into(),
                el("style", Attrs::new(), vec![text("a>b{c:'&'}")]).into(),
            ],
        );
        assert_eq!(
            fragment(&node),
            "<head><title>a&#x3C;b&#x26;c</title><style>a>b{c:'&'}</style></head>"
        );
    }

    #[test]
    fn writes_a_void_element_with_children_as_a_normal_one() {
        let node = el("br", Attrs::new(), vec![text("x")]);
        assert_eq!(fragment(&node), "<br>x</br>");
    }

    #[test]
    fn frames_a_document_with_a_lowercase_doctype_and_newlines() {
        assert_eq!(
            document(&Element::empty("html")),
            "<!doctype html>\n<html></html>\n"
        );
    }
}
