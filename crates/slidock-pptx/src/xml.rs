//! A part's XML as a small owned tree, so elements can outlive the text they were parsed from: a theme's format
//! scheme and the presentation's table styles serve every container. Parsing and its hostile-input limits are
//! `slidock-io`'s; this module keeps what the readers ask of an element: its qualified name, local name and namespace,
//! its attributes by qualified name as written, and its children in order, adjacent text merged.

use std::rc::Rc;

use slidock_canon::js_trim;
use slidock_io::roxmltree::{Document, Node};
use slidock_io::xml::{attributes, namespace_uri, qualified_name};

/// The namespaces the readers match elements in, transitional and strict URIs alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ns {
    /// PresentationML.
    P,
    /// DrawingML.
    A,
    /// Markup compatibility.
    Mc,
    /// Package relationships.
    Rel,
    /// PowerPoint 2010.
    P14,
    /// Office 2016's SVG pictures (`asvg:svgBlip`).
    Asvg,
    /// Office 2019's decorative flag (`adec:decorative`).
    Adec,
    Other,
}

impl Ns {
    fn of(uri: &str) -> Ns {
        match uri {
            "http://schemas.openxmlformats.org/presentationml/2006/main"
            | "http://purl.oclc.org/ooxml/presentationml/main" => Ns::P,
            "http://schemas.openxmlformats.org/drawingml/2006/main"
            | "http://purl.oclc.org/ooxml/drawingml/main" => Ns::A,
            "http://schemas.openxmlformats.org/markup-compatibility/2006" => Ns::Mc,
            "http://schemas.openxmlformats.org/package/2006/relationships" => Ns::Rel,
            "http://schemas.microsoft.com/office/powerpoint/2010/main" => Ns::P14,
            "http://schemas.microsoft.com/office/drawing/2016/SVG/main" => Ns::Asvg,
            "http://schemas.microsoft.com/office/drawing/2017/decorative" => Ns::Adec,
            _ => Ns::Other,
        }
    }
}

/// A child of an element.
#[derive(Debug)]
pub(crate) enum XmlNode {
    Element(Rc<Element>),
    Text(Box<str>),
}

#[derive(Debug)]
pub(crate) struct Element {
    /// The qualified name as written: `p:sp`.
    pub name: Box<str>,
    /// The local name: `sp`.
    pub local: Box<str>,
    pub ns: Ns,
    /// Attributes by qualified name (`r:id`), namespace declarations included, in written order.
    attrs: Vec<(Box<str>, Box<str>)>,
    children: Vec<XmlNode>,
}

/// An element's text-free children.
pub(crate) type Kids<'a> =
    std::iter::FilterMap<std::slice::Iter<'a, XmlNode>, fn(&'a XmlNode) -> Option<&'a Rc<Element>>>;

fn element_of(node: &XmlNode) -> Option<&Rc<Element>> {
    match node {
        XmlNode::Element(element) => Some(element),
        XmlNode::Text(_) => None,
    }
}

impl Element {
    pub fn is(&self, ns: Ns, local: &str) -> bool {
        self.ns == ns && &*self.local == local
    }

    /// The element children, skipping text.
    pub fn kids(&self) -> Kids<'_> {
        self.children.iter().filter_map(element_of)
    }

    pub fn child(&self, ns: Ns, local: &str) -> Option<&Rc<Element>> {
        self.kids().find(|kid| kid.is(ns, local))
    }

    pub fn children<'a, 'n>(
        &'a self,
        ns: Ns,
        local: &'n str,
    ) -> impl Iterator<Item = &'a Rc<Element>> + use<'a, 'n> {
        self.kids().filter(move |kid| kid.is(ns, local))
    }

    pub fn rel_attr(&self, local: &str) -> Option<&str> {
        Some(self).rel_attr(local)
    }

    pub fn int_attr(&self, name: &str) -> Option<i64> {
        Some(self).int_attr(name)
    }

    pub fn bool_attr(&self, name: &str) -> Option<bool> {
        Some(self).bool_attr(name)
    }

    /// The descendant at a path of child names.
    pub fn path(&self, steps: &[(Ns, &str)]) -> Option<&Element> {
        let mut at: &Element = self;
        for (ns, local) in steps {
            at = at.child(*ns, local)?;
        }
        Some(at)
    }

    /// An attribute by its qualified name as written.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| &**key == name)
            .map(|(_, value)| &**value)
    }

    /// Every attribute by qualified name, in written order.
    pub fn attrs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.attrs.iter().map(|(name, value)| (&**name, &**value))
    }

    /// The concatenated text of the element and its descendants.
    pub fn text_content(&self) -> String {
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
}

/// The reads every element and absent element answer alike, as the readers chain them through optional children.
pub(crate) trait El<'a>: Copy {
    fn element(self) -> Option<&'a Element>;

    fn child(self, ns: Ns, local: &str) -> Option<&'a Element> {
        self.element()?.child(ns, local).map(|rc| &**rc)
    }

    fn kids(self) -> Vec<&'a Element> {
        self.element()
            .map(|element| element.kids().map(|rc| &**rc).collect())
            .unwrap_or_default()
    }

    fn children(self, ns: Ns, local: &str) -> Vec<&'a Element> {
        self.element()
            .map(|element| element.children(ns, local).map(|rc| &**rc).collect())
            .unwrap_or_default()
    }

    fn attr(self, name: &str) -> Option<&'a str> {
        self.element()?.attr(name)
    }

    /// A relationship attribute (`r:id`, `r:embed`, `r:link`) by its local name: the first prefixed attribute with
    /// that local name, whatever prefix a writer chose, declarations aside.
    fn rel_attr(self, local: &str) -> Option<&'a str> {
        self.element()?.attrs().find_map(|(name, value)| {
            let (prefix, name_local) = name.split_once(':')?;
            (!prefix.is_empty() && name_local == local && prefix != "xmlns").then_some(value)
        })
    }

    /// An integer attribute of up to 15 digits, with optional sign and surrounding whitespace; anything else is
    /// `None`.
    fn int_attr(self, name: &str) -> Option<i64> {
        parse_int(self.attr(name)?)
    }

    /// `xsd:boolean` (1/true, 0/false), plus on/off.
    fn bool_attr(self, name: &str) -> Option<bool> {
        match js_trim(self.attr(name)?) {
            "1" | "true" | "on" => Some(true),
            "0" | "false" | "off" => Some(false),
            _ => None,
        }
    }
}

impl<'a> El<'a> for &'a Element {
    fn element(self) -> Option<&'a Element> {
        Some(self)
    }
}

impl<'a> El<'a> for Option<&'a Element> {
    fn element(self) -> Option<&'a Element> {
        self
    }
}

impl<'a> El<'a> for Option<&'a Rc<Element>> {
    fn element(self) -> Option<&'a Element> {
        self.map(|rc| &**rc)
    }
}

/// `INTEGER = /^\s*[-+]?\d{1,15}\s*$/` read with `Number`.
pub(crate) fn parse_int(text: &str) -> Option<i64> {
    let trimmed = js_trim(text);
    let digits = trimmed
        .strip_prefix('-')
        .or_else(|| trimmed.strip_prefix('+'))
        .unwrap_or(trimmed);
    if digits.is_empty() || digits.len() > 15 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let value: i64 = digits.parse().ok()?;
    Some(if trimmed.starts_with('-') {
        -value
    } else {
        value
    })
}

/// The tree of a parsed document's root element.
pub(crate) fn tree(document: &Document<'_>) -> Rc<Element> {
    convert(document.root_element())
}

fn convert(node: Node<'_, '_>) -> Rc<Element> {
    let mut children: Vec<XmlNode> = Vec::new();
    let mut text: Option<String> = None;
    for child in node.children() {
        if child.is_element() {
            if let Some(run) = text.take() {
                children.push(XmlNode::Text(run.into()));
            }
            children.push(XmlNode::Element(convert(child)));
        } else if let Some(value) = child.text().filter(|_| child.is_text()) {
            text.get_or_insert_with(String::new).push_str(value);
        }
    }
    if let Some(run) = text {
        children.push(XmlNode::Text(run.into()));
    }
    let attrs = attributes(node, None)
        .into_iter()
        .map(|(name, value)| (name.into_boxed_str(), value.into_boxed_str()))
        .collect();
    Rc::new(Element {
        name: qualified_name(node).into(),
        local: node.tag_name().name().into(),
        ns: Ns::of(namespace_uri(node)),
        attrs,
        children,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use slidock_io::{XmlBudget, XmlLimits, parse_xml};

    fn parse(text: &str) -> Rc<Element> {
        let limits = XmlLimits::default();
        let document = parse_xml(text, &limits, &mut XmlBudget::new(&limits)).unwrap();
        tree(&document)
    }

    #[test]
    fn reads_names_namespaces_attributes_and_merged_text() {
        let root = parse(
            r#"<p:sld xmlns:p="http://purl.oclc.org/ooxml/presentationml/main" xmlns:x="urn:x" x:id="7" id="3"><p:t>a<!-- c -->b<![CDATA[<c>]]></p:t></p:sld>"#,
        );
        assert_eq!(
            (&*root.name, &*root.local, root.ns),
            ("p:sld", "sld", Ns::P)
        );
        assert_eq!(root.attr("id"), Some("3"));
        assert_eq!(root.attr("x:id"), Some("7"));
        assert_eq!(root.rel_attr("id"), Some("7"));
        assert!(root.attr("xmlns:p").is_some());
        let t = root.child(Ns::P, "t").unwrap();
        assert_eq!(t.text_content(), "ab<c>");
        assert_eq!(t.children.len(), 1);
    }

    #[test]
    fn reads_integers_of_up_to_fifteen_digits_and_xsd_booleans() {
        assert_eq!(parse_int(" -12 "), Some(-12));
        assert_eq!(parse_int("+7"), Some(7));
        assert_eq!(parse_int("999999999999999"), Some(999_999_999_999_999));
        for text in ["1000000000000000", "1.5", "1e3", "", "-", "0x10", "١"] {
            assert_eq!(parse_int(text), None, "{text}");
        }
        let root = parse(r#"<a b=" on " c="0" d="yes"/>"#);
        assert_eq!(root.bool_attr("b"), Some(true));
        assert_eq!(root.bool_attr("c"), Some(false));
        assert_eq!(root.bool_attr("d"), None);
    }
}
