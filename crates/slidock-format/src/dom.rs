//! Documents as a browser parses them: html5ever's WHATWG tree construction into a small arena, so the lint and the
//! composer tell markup from text and comments as a browser does, and read `<template>` content too. The serialiser
//! writes the HTML fragment serialisation that parse5 7 writes, which the compose goldens pin byte for byte.

use std::borrow::Cow;
use std::cell::{Ref, RefCell};

use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::{Attribute, ParseOpts, QualName, local_name, ns, parse_document};

/// A node of a [`Document`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(usize);

impl NodeId {
    /// The document node.
    pub const DOCUMENT: NodeId = NodeId(0);
}

/// An attribute: its local name (`href` for `xlink:href`), its namespace and prefix, and its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attr {
    pub name: QualName,
    pub value: String,
}

impl Attr {
    /// The local name: what the lint and the composer match.
    pub fn local(&self) -> &str {
        &self.name.local
    }
}

#[derive(Clone, Debug)]
pub enum NodeData {
    Document,
    /// A template's content.
    Fragment,
    Doctype {
        name: String,
    },
    Element {
        name: QualName,
        attrs: Vec<Attr>,
        /// The content fragment of a `<template>`.
        template: Option<NodeId>,
    },
    Text(String),
    Comment(String),
}

#[derive(Clone, Debug)]
struct Node {
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    data: NodeData,
}

/// A parsed document: an arena of nodes under [`NodeId::DOCUMENT`].
#[derive(Clone, Debug)]
pub struct Document {
    nodes: Vec<Node>,
}

const NO_CHILDREN: &[NodeId] = &[];
const NO_ATTRS: &[Attr] = &[];

impl Document {
    /// Parses `html` as a whole document, as a browser with scripting enabled does.
    pub fn parse(html: &str) -> Document {
        let sink = Sink {
            arena: RefCell::new(Arena {
                document: Document::empty(),
                no_name: QualName::new(None, ns!(), local_name!("")),
            }),
        };
        parse_document(sink, ParseOpts::default()).one(StrTendril::from(html))
    }

    fn empty() -> Document {
        Document {
            nodes: vec![Node {
                parent: None,
                children: Vec::new(),
                data: NodeData::Document,
            }],
        }
    }

    fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.0)
    }

    fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id.0)
    }

    fn push(&mut self, data: NodeData) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node {
            parent: None,
            children: Vec::new(),
            data,
        });
        id
    }

    pub fn data(&self, id: NodeId) -> Option<&NodeData> {
        self.node(id).map(|node| &node.data)
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).and_then(|node| node.parent)
    }

    /// The child nodes of `id` (a template's own children, which the parser leaves empty).
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.node(id).map_or(NO_CHILDREN, |node| &node.children)
    }

    pub fn is_element(&self, id: NodeId) -> bool {
        matches!(self.data(id), Some(NodeData::Element { .. }))
    }

    /// The tag name: an element's local name, with SVG's camelCase names (`linearGradient`) as the parser adjusts
    /// them; empty for any other node.
    pub fn tag_name(&self, id: NodeId) -> &str {
        match self.data(id) {
            Some(NodeData::Element { name, .. }) => &name.local,
            _ => "",
        }
    }

    pub fn qual_name(&self, id: NodeId) -> Option<&QualName> {
        match self.data(id) {
            Some(NodeData::Element { name, .. }) => Some(name),
            _ => None,
        }
    }

    fn is_html_element(&self, id: NodeId, local: &str) -> bool {
        self.qual_name(id)
            .is_some_and(|name| name.ns == ns!(html) && &*name.local == local)
    }

    /// The content fragment of a `<template>`.
    pub fn template_content(&self, id: NodeId) -> Option<NodeId> {
        match self.data(id) {
            Some(NodeData::Element { template, .. }) => *template,
            _ => None,
        }
    }

    pub fn attrs(&self, id: NodeId) -> &[Attr] {
        match self.data(id) {
            Some(NodeData::Element { attrs, .. }) => attrs,
            _ => NO_ATTRS,
        }
    }

    pub fn attrs_mut(&mut self, id: NodeId) -> Option<&mut Vec<Attr>> {
        match self.node_mut(id).map(|node| &mut node.data) {
            Some(NodeData::Element { attrs, .. }) => Some(attrs),
            _ => None,
        }
    }

    /// An attribute's value, matched by local name (`href` matches `xlink:href`).
    pub fn attribute(&self, id: NodeId, name: &str) -> Option<&str> {
        self.attrs(id)
            .iter()
            .find(|attr| attr.local() == name)
            .map(|attr| attr.value.as_str())
    }

    pub fn has_attribute(&self, id: NodeId, name: &str) -> bool {
        self.attribute(id, name).is_some()
    }

    /// The element children of a node, a template's content included.
    pub fn child_elements(&self, id: NodeId) -> Vec<NodeId> {
        let container = self.template_content(id).unwrap_or(id);
        self.children(container)
            .iter()
            .copied()
            .filter(|child| self.is_element(*child))
            .collect()
    }

    /// Every element under `id`, depth-first in document order, `<template>` content included.
    pub fn walk(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack: Vec<NodeId> = self.child_elements(id).into_iter().rev().collect();
        while let Some(next) = stack.pop() {
            out.push(next);
            stack.extend(self.child_elements(next).into_iter().rev());
        }
        out
    }

    /// The text of a node's direct text children.
    pub fn text_content(&self, id: NodeId) -> String {
        self.children(id)
            .iter()
            .filter_map(|child| match self.data(*child) {
                Some(NodeData::Text(text)) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// The first `<tag>` child element of `parent`.
    fn child_named(&self, parent: NodeId, tag: &str) -> Option<NodeId> {
        self.child_elements(parent)
            .into_iter()
            .find(|child| self.tag_name(*child) == tag)
    }

    /// The `<head>` the parser always creates.
    pub fn head(&self) -> Option<NodeId> {
        let html = self.child_named(NodeId::DOCUMENT, "html")?;
        self.child_named(html, "head")
    }

    /// The `<body>` the parser always creates.
    pub fn body(&self) -> Option<NodeId> {
        let html = self.child_named(NodeId::DOCUMENT, "html")?;
        self.child_named(html, "body")
    }

    /// Renames an element, keeping its namespace.
    pub fn set_tag_name(&mut self, id: NodeId, local: &str) {
        if let Some(Node {
            data: NodeData::Element { name, .. },
            ..
        }) = self.node_mut(id)
        {
            name.local = local.into();
        }
    }

    /// A new HTML element with `attrs` (by local name, in no namespace), not yet in the tree.
    pub fn create_element(&mut self, local: &str, attrs: &[(&str, &str)]) -> NodeId {
        self.push(NodeData::Element {
            name: QualName::new(None, ns!(html), local.into()),
            attrs: attrs
                .iter()
                .map(|(name, value)| Attr {
                    name: QualName::new(None, ns!(), (*name).into()),
                    value: (*value).to_owned(),
                })
                .collect(),
            template: None,
        })
    }

    /// A new text node, not yet in the tree.
    pub fn create_text(&mut self, text: &str) -> NodeId {
        self.push(NodeData::Text(text.to_owned()))
    }

    /// Replaces the child nodes of `parent` (its own, not a template's content) with `children`, detaching each
    /// from its previous parent.
    pub fn set_children(&mut self, parent: NodeId, children: Vec<NodeId>) {
        for child in &children {
            self.detach(*child);
        }
        let old = match self.node_mut(parent) {
            Some(node) => std::mem::replace(&mut node.children, children.clone()),
            None => return,
        };
        for child in old {
            if let Some(node) = self.node_mut(child) {
                node.parent = None;
            }
        }
        for child in children {
            if let Some(node) = self.node_mut(child) {
                node.parent = Some(parent);
            }
        }
    }

    fn detach(&mut self, id: NodeId) {
        let Some(parent) = self.parent(id) else {
            return;
        };
        if let Some(node) = self.node_mut(parent) {
            node.children.retain(|child| *child != id);
        }
        if let Some(node) = self.node_mut(id) {
            node.parent = None;
        }
    }

    /// A deep copy of `id` from `other` into this arena, not yet in the tree. Iterative, so any depth fits the
    /// stack.
    pub fn import(&mut self, other: &Document, id: NodeId) -> Option<NodeId> {
        let root = self.push(other.data(id)?.clone());
        let mut pending = vec![(id, root)];
        while let Some((source, copy)) = pending.pop() {
            if let Some(content) = other.template_content(source)
                && let Some(data) = other.data(content)
            {
                let content_copy = self.push(data.clone());
                if let Some(Node {
                    data: NodeData::Element { template, .. },
                    ..
                }) = self.node_mut(copy)
                {
                    *template = Some(content_copy);
                }
                pending.push((content, content_copy));
            }
            let mut children = Vec::with_capacity(other.children(source).len());
            for child in other.children(source) {
                let Some(data) = other.data(*child) else {
                    continue;
                };
                let child_copy = self.push(data.clone());
                children.push(child_copy);
                pending.push((*child, child_copy));
            }
            self.set_children(copy, children);
        }
        Some(root)
    }

    /// The serialised children of `id` (a template's content for a `<template>`): `parse5.serialize`.
    pub fn serialize(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.write_children(id, &mut out);
        out
    }

    /// The serialised node itself: `parse5.serializeOuter`.
    pub fn serialize_outer(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.write_node(id, &mut out);
        out
    }

    /// The nodes whose markup `id`'s children serialise to: a template's content for an HTML `<template>`.
    fn serialised_children(&self, id: NodeId) -> &[NodeId] {
        let container = if self.is_html_element(id, "template") {
            self.template_content(id).unwrap_or(id)
        } else {
            id
        };
        self.children(container)
    }

    fn write_children(&self, id: NodeId, out: &mut String) {
        let mut pending: Vec<Step> = self
            .serialised_children(id)
            .iter()
            .rev()
            .map(|child| Step::Open(*child))
            .collect();
        self.write_steps(&mut pending, out);
    }

    fn write_node(&self, id: NodeId, out: &mut String) {
        self.write_steps(&mut vec![Step::Open(id)], out);
    }

    /// Writes nodes from an explicit stack, so any depth fits the call stack.
    fn write_steps(&self, pending: &mut Vec<Step>, out: &mut String) {
        while let Some(step) = pending.pop() {
            let id = match step {
                Step::Open(id) => id,
                Step::Close(id) => {
                    out.push_str("</");
                    out.push_str(self.tag_name(id));
                    out.push('>');
                    continue;
                }
            };
            match self.data(id) {
                Some(NodeData::Element { name, attrs, .. }) => {
                    out.push('<');
                    out.push_str(&name.local);
                    for attr in attrs {
                        write_attribute(attr, out);
                    }
                    out.push('>');
                    if !is_void(name) {
                        pending.push(Step::Close(id));
                        pending.extend(
                            self.serialised_children(id)
                                .iter()
                                .rev()
                                .map(|child| Step::Open(*child)),
                        );
                    }
                }
                Some(NodeData::Text(text)) => {
                    let raw = self
                        .parent(id)
                        .and_then(|parent| self.qual_name(parent))
                        .is_some_and(|parent| {
                            parent.ns == ns!(html) && RAW_TEXT_PARENTS.contains(&&*parent.local)
                        });
                    if raw {
                        out.push_str(text);
                    } else {
                        escape_into(text, out, |c| match c {
                            '&' => Some("&amp;"),
                            '<' => Some("&lt;"),
                            '>' => Some("&gt;"),
                            '\u{a0}' => Some("&nbsp;"),
                            _ => None,
                        });
                    }
                }
                Some(NodeData::Comment(text)) => {
                    out.push_str("<!--");
                    out.push_str(text);
                    out.push_str("-->");
                }
                Some(NodeData::Doctype { name }) => {
                    out.push_str("<!DOCTYPE ");
                    out.push_str(name);
                    out.push('>');
                }
                Some(NodeData::Document | NodeData::Fragment) => {
                    pending.extend(
                        self.serialised_children(id)
                            .iter()
                            .rev()
                            .map(|child| Step::Open(*child)),
                    );
                }
                None => {}
            }
        }
    }
}

/// A serialiser step: a node to write, or an element to close.
enum Step {
    Open(NodeId),
    Close(NodeId),
}

/// HTML elements whose text serialises unescaped (scripting enabled, so `<noscript>` too).
const RAW_TEXT_PARENTS: &[&str] = &[
    "style",
    "script",
    "xmp",
    "iframe",
    "noembed",
    "noframes",
    "plaintext",
    "noscript",
];

const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img", "input",
    "keygen", "link", "meta", "param", "source", "track", "wbr",
];

fn is_void(name: &QualName) -> bool {
    name.ns == ns!(html) && VOID_ELEMENTS.contains(&&*name.local)
}

fn write_attribute(attr: &Attr, out: &mut String) {
    out.push(' ');
    let name = &attr.name;
    if name.ns == ns!() {
        // No namespace.
    } else if name.ns == ns!(xml) {
        out.push_str("xml:");
    } else if name.ns == ns!(xmlns) {
        if &*name.local != "xmlns" {
            out.push_str("xmlns:");
        }
    } else if name.ns == ns!(xlink) {
        out.push_str("xlink:");
    } else if let Some(prefix) = &name.prefix {
        out.push_str(prefix);
        out.push(':');
    }
    out.push_str(&name.local);
    out.push_str("=\"");
    escape_into(&attr.value, out, |c| match c {
        '&' => Some("&amp;"),
        '"' => Some("&quot;"),
        '\u{a0}' => Some("&nbsp;"),
        _ => None,
    });
    out.push('"');
}

fn escape_into(text: &str, out: &mut String, entity: impl Fn(char) -> Option<&'static str>) {
    for c in text.chars() {
        match entity(c) {
            Some(replacement) => out.push_str(replacement),
            None => out.push(c),
        }
    }
}

struct Arena {
    document: Document,
    /// The name `elem_name` gives a node that is not an element, which the tree builder never asks for.
    no_name: QualName,
}

impl Arena {
    fn append_text(&mut self, parent: NodeId, index: Option<usize>, text: &str) {
        let children = self.document.children(parent);
        let previous = match index {
            Some(i) => i.checked_sub(1).and_then(|p| children.get(p)),
            None => children.last(),
        }
        .copied();
        if let Some(previous) = previous
            && let Some(Node {
                data: NodeData::Text(existing),
                ..
            }) = self.document.node_mut(previous)
        {
            existing.push_str(text);
            return;
        }
        let node = self.document.push(NodeData::Text(text.to_owned()));
        self.insert(parent, index, node);
    }

    fn insert(&mut self, parent: NodeId, index: Option<usize>, child: NodeId) {
        self.document.detach(child);
        let index = index.map(|i| i.min(self.document.children(parent).len()));
        if let Some(node) = self.document.node_mut(parent) {
            match index {
                Some(i) => node.children.insert(i, child),
                None => node.children.push(child),
            }
        }
        if let Some(node) = self.document.node_mut(child) {
            node.parent = Some(parent);
        }
    }

    fn add(&mut self, parent: NodeId, index: Option<usize>, child: NodeOrText<NodeId>) {
        match child {
            NodeOrText::AppendNode(node) => self.insert(parent, index, node),
            NodeOrText::AppendText(text) => self.append_text(parent, index, &text),
        }
    }

    fn index_in_parent(&self, id: NodeId) -> Option<(NodeId, usize)> {
        let parent = self.document.parent(id)?;
        let index = self
            .document
            .children(parent)
            .iter()
            .position(|child| *child == id)?;
        Some((parent, index))
    }
}

/// html5ever's sink: the tree builder drives it through shared references.
struct Sink {
    arena: RefCell<Arena>,
}

impl TreeSink for Sink {
    type Handle = NodeId;
    type Output = Document;
    type ElemName<'a> = Ref<'a, QualName>;

    fn finish(self) -> Document {
        self.arena.into_inner().document
    }

    fn parse_error(&self, _msg: Cow<'static, str>) {}

    fn get_document(&self) -> NodeId {
        NodeId::DOCUMENT
    }

    fn elem_name<'a>(&'a self, target: &'a NodeId) -> Ref<'a, QualName> {
        Ref::map(self.arena.borrow(), |arena| {
            arena.document.qual_name(*target).unwrap_or(&arena.no_name)
        })
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> NodeId {
        let mut arena = self.arena.borrow_mut();
        let template = flags
            .template
            .then(|| arena.document.push(NodeData::Fragment));
        let attrs = attrs
            .into_iter()
            .map(|attr| Attr {
                name: attr.name,
                value: attr.value.to_string(),
            })
            .collect();
        arena.document.push(NodeData::Element {
            name,
            attrs,
            template,
        })
    }

    fn create_comment(&self, text: StrTendril) -> NodeId {
        self.arena
            .borrow_mut()
            .document
            .push(NodeData::Comment(text.to_string()))
    }

    fn create_pi(&self, _target: StrTendril, data: StrTendril) -> NodeId {
        self.arena
            .borrow_mut()
            .document
            .push(NodeData::Comment(data.to_string()))
    }

    fn append(&self, parent: &NodeId, child: NodeOrText<NodeId>) {
        self.arena.borrow_mut().add(*parent, None, child);
    }

    fn append_based_on_parent_node(
        &self,
        element: &NodeId,
        prev_element: &NodeId,
        child: NodeOrText<NodeId>,
    ) {
        let has_parent = self.arena.borrow().document.parent(*element).is_some();
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        _public_id: StrTendril,
        _system_id: StrTendril,
    ) {
        let mut arena = self.arena.borrow_mut();
        let doctype = arena.document.push(NodeData::Doctype {
            name: name.to_string(),
        });
        arena.insert(NodeId::DOCUMENT, None, doctype);
    }

    fn get_template_contents(&self, target: &NodeId) -> NodeId {
        self.arena
            .borrow()
            .document
            .template_content(*target)
            .unwrap_or(*target)
    }

    fn same_node(&self, x: &NodeId, y: &NodeId) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, _mode: QuirksMode) {}

    fn append_before_sibling(&self, sibling: &NodeId, new_node: NodeOrText<NodeId>) {
        let mut arena = self.arena.borrow_mut();
        if let NodeOrText::AppendNode(node) = &new_node {
            arena.document.detach(*node);
        }
        if let Some((parent, index)) = arena.index_in_parent(*sibling) {
            arena.add(parent, Some(index), new_node);
        }
    }

    fn add_attrs_if_missing(&self, target: &NodeId, attrs: Vec<Attribute>) {
        let mut arena = self.arena.borrow_mut();
        if let Some(existing) = arena.document.attrs_mut(*target) {
            for attr in attrs {
                if !existing.iter().any(|old| old.name == attr.name) {
                    existing.push(Attr {
                        name: attr.name,
                        value: attr.value.to_string(),
                    });
                }
            }
        }
    }

    fn remove_from_parent(&self, target: &NodeId) {
        self.arena.borrow_mut().document.detach(*target);
    }

    fn reparent_children(&self, node: &NodeId, new_parent: &NodeId) {
        let mut arena = self.arena.borrow_mut();
        let children = arena.document.children(*node).to_vec();
        for child in children {
            arena.insert(*new_parent, None, child);
        }
    }

    fn allow_declarative_shadow_roots(&self, _intended_parent: &NodeId) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(html: &str) -> String {
        Document::parse(html).serialize(NodeId::DOCUMENT)
    }

    #[test]
    fn serialises_as_parse5_does() {
        assert_eq!(
            roundtrip(
                "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n</head>\n<body>\n<p a b=\"&quot;&lt;\">x &amp; &lt;y&gt;\u{a0}</p>\n</body>\n</html>\n"
            ),
            "<!DOCTYPE html><html><head>\n<meta charset=\"utf-8\">\n</head>\n<body>\n<p a=\"\" b=\"&quot;<\">x &amp; &lt;y&gt;&nbsp;</p>\n\n\n</body></html>"
        );
    }

    #[test]
    fn keeps_raw_text_and_comments() {
        assert_eq!(
            roundtrip("<style>a>b{}</style><!-- c --><script>if (a < b) {}</script>"),
            "<html><head><style>a>b{}</style><!-- c --><script>if (a < b) {}</script></head><body></body></html>"
        );
        // Every raw-text element, `<noscript>` included because scripting is enabled.
        for name in ["style", "script", "xmp", "iframe", "noembed", "noscript"] {
            let html = format!("<body><{name}>a & <b> </{name}></body>");
            assert_eq!(
                roundtrip(&html),
                format!("<html><head></head><body><{name}>a & <b> </{name}></body></html>"),
                "{name}"
            );
        }
        assert_eq!(
            roundtrip("<body><plaintext>a & <b>"),
            "<html><head></head><body><plaintext>a & <b></plaintext></body></html>"
        );
        assert_eq!(
            roundtrip("<body><textarea>a & <b></textarea></body>"),
            "<html><head></head><body><textarea>a &amp; &lt;b&gt;</textarea></body></html>"
        );
    }

    #[test]
    fn reads_template_content_and_svg_names() {
        let document = Document::parse(
            "<template><i a=1></i></template><svg viewbox='0 0 1 1'><lineargradient xlink:href='#g'/></svg>",
        );
        let tags: Vec<&str> = document
            .walk(NodeId::DOCUMENT)
            .into_iter()
            .map(|id| document.tag_name(id))
            .collect();
        assert_eq!(
            tags,
            [
                "html",
                "head",
                "template",
                "i",
                "body",
                "svg",
                "linearGradient"
            ]
        );
        let gradient = document.walk(NodeId::DOCUMENT).into_iter().last().unwrap();
        assert_eq!(document.attribute(gradient, "href"), Some("#g"));
        assert_eq!(
            document.serialize(NodeId::DOCUMENT),
            "<html><head><template><i a=\"1\"></i></template></head><body><svg viewBox=\"0 0 1 1\"><linearGradient xlink:href=\"#g\"></linearGradient></svg></body></html>"
        );
    }

    #[test]
    fn merges_adjacent_text_and_fosters_table_content() {
        let document = Document::parse("<table>a<tr><td>b</td></tr>c</table>");
        assert_eq!(
            document.serialize(NodeId::DOCUMENT),
            "<html><head></head><body>ac<table><tbody><tr><td>b</td></tr></tbody></table></body></html>"
        );
    }

    #[test]
    fn handles_any_depth() {
        let depth = 100_000;
        let html = format!("{}x{}", "<b>".repeat(depth), "</b>".repeat(depth));
        let document = Document::parse(&html);
        let body = document.body().unwrap();
        let serialised = document.serialize(body);
        assert!(serialised.starts_with("<b><b>") && serialised.ends_with("</b></b>"));
        let mut copy = Document::parse("");
        let first = document.child_elements(body)[0];
        let imported = copy.import(&document, first).unwrap();
        assert_eq!(
            copy.serialize_outer(imported),
            document.serialize_outer(first)
        );
        assert_eq!(document.walk(body).len(), copy.walk(imported).len() + 1);
    }

    #[test]
    fn imports_and_reparents() {
        let mut target = Document::parse("<main id=m></main>");
        let source = Document::parse("<template><b>t</b></template><p>x</p>");
        let template = source
            .walk(NodeId::DOCUMENT)
            .into_iter()
            .find(|id| source.tag_name(*id) == "template")
            .unwrap();
        let main = target
            .walk(NodeId::DOCUMENT)
            .into_iter()
            .find(|id| target.tag_name(*id) == "main")
            .unwrap();
        let copy = target.import(&source, template).unwrap();
        let line = target.create_text("\n");
        target.set_children(main, vec![line, copy]);
        assert_eq!(
            target.serialize_outer(main),
            "<main id=\"m\">\n<template><b>t</b></template></main>"
        );
    }
}
