//! The tree the printers build and the serializer writes once: elements with their attributes in insertion order (the
//! order is fixed per element kind and is part of the output), text, and the newline separators that keep diffs
//! line-based.

/// A node of a document.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
}

/// An attribute value: a bare name, or a name with a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Attr {
    Bare,
    Value(String),
}

/// An element. The children of a `<template>` are its content fragment.
#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub tag: &'static str,
    pub attrs: Attrs,
    pub children: Vec<Node>,
}

impl Element {
    pub fn new(tag: &'static str, attrs: Attrs, children: Vec<Node>) -> Element {
        Element {
            tag,
            attrs,
            children,
        }
    }

    /// An element with no attributes or children.
    pub fn empty(tag: &'static str) -> Element {
        Element::new(tag, Attrs::new(), Vec::new())
    }

    /// The value of an attribute, `""` for a bare one.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(|value| match value {
            Attr::Bare => "",
            Attr::Value(value) => value.as_str(),
        })
    }
}

impl From<Element> for Node {
    fn from(element: Element) -> Node {
        Node::Element(element)
    }
}

/// A text node.
pub fn text(value: impl Into<String>) -> Node {
    Node::Text(value.into())
}

/// Attributes in insertion order. Absent values are skipped, as `h()` skips `undefined` and `false`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attrs(Vec<(&'static str, Attr)>);

impl Attrs {
    pub fn new() -> Attrs {
        Attrs(Vec::new())
    }

    /// `name="value"`, or nothing when `value` is `None`.
    pub fn val(mut self, name: &'static str, value: impl Into<Option<String>>) -> Attrs {
        if let Some(value) = value.into() {
            self.set(name, Attr::Value(value));
        }
        self
    }

    /// The bare `name` when `on`.
    pub fn flag(mut self, name: &'static str, on: bool) -> Attrs {
        if on {
            self.set(name, Attr::Bare);
        }
        self
    }

    /// Every attribute of `other`, after these, as an object spread merges them.
    pub fn with(mut self, other: Attrs) -> Attrs {
        for (name, value) in other.0 {
            self.set(name, value);
        }
        self
    }

    /// Sets `name`, keeping its position when it is already present (as an object spread does).
    pub fn set(&mut self, name: &'static str, value: Attr) {
        match self.0.iter_mut().find(|(existing, _)| *existing == name) {
            Some(slot) => slot.1 = value,
            None => self.0.push((name, value)),
        }
    }

    pub fn get(&self, name: &str) -> Option<&Attr> {
        self.0
            .iter()
            .find(|(existing, _)| *existing == name)
            .map(|(_, value)| value)
    }

    pub fn iter(&self) -> impl Iterator<Item = &(&'static str, Attr)> {
        self.0.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Children each on their own line: a newline before each and after the last. Only for blocks and positioned
/// siblings, whose whitespace stays invisible; inside a paragraph, a link or a run it would render.
pub fn lines(children: Vec<Element>) -> Vec<Node> {
    if children.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(children.len() * 2 + 1);
    for child in children {
        out.push(text("\n"));
        out.push(Node::Element(child));
    }
    out.push(text("\n"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_absent_values_and_keeps_the_first_position_of_a_repeated_name() {
        let mut attrs = Attrs::new()
            .val("a", Some("1".to_owned()))
            .val("b", None::<String>)
            .flag("c", false)
            .flag("d", true)
            .val("e", "2".to_owned());
        attrs.set("a", Attr::Value("3".into()));
        let names: Vec<&str> = attrs.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["a", "d", "e"]);
        assert_eq!(attrs.get("a"), Some(&Attr::Value("3".into())));
        let merged = Attrs::new().val("x", "1".to_owned()).with(
            Attrs::new()
                .val("y", "2".to_owned())
                .val("x", "3".to_owned()),
        );
        let pairs: Vec<_> = merged.iter().cloned().collect();
        assert_eq!(
            pairs,
            [
                ("x", Attr::Value("3".into())),
                ("y", Attr::Value("2".into()))
            ]
        );
    }

    #[test]
    fn separates_lines_around_each_child() {
        assert!(lines(Vec::new()).is_empty());
        let children = lines(vec![Element::empty("p"), Element::empty("p")]);
        assert_eq!(children.len(), 5);
        assert_eq!(children[0], text("\n"));
        assert_eq!(children[4], text("\n"));
    }
}
