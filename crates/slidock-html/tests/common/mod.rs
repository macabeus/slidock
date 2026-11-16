//! Reading printed documents back in tests: html5ever's tree, through slidock-format's arena.
#![allow(dead_code, clippy::unwrap_used, reason = "test code")]

use slidock_format::dom::{Document, NodeData, NodeId};

/// The text of a node and all its descendants, in document order.
pub fn deep_text(doc: &Document, id: NodeId) -> String {
    let mut out = String::new();
    for child in doc.children(id) {
        match doc.data(*child) {
            Some(NodeData::Text(text)) => out.push_str(text),
            Some(NodeData::Element { .. }) => out.push_str(&deep_text(doc, *child)),
            _ => {}
        }
    }
    out
}

/// Every element of a document, in document order.
pub fn elements(doc: &Document) -> Vec<NodeId> {
    doc.walk(NodeId::DOCUMENT)
}

/// The element whose `data-el` is `id`.
pub fn element_by_id(doc: &Document, id: &str) -> Option<NodeId> {
    elements(doc)
        .into_iter()
        .find(|node| doc.attribute(*node, "data-el") == Some(id))
}

/// The descendants of `id`, in document order.
pub fn descendants(doc: &Document, id: NodeId) -> Vec<NodeId> {
    doc.walk(id)
}

/// The element children of `id`.
pub fn children(doc: &Document, id: NodeId) -> Vec<NodeId> {
    doc.child_elements(id)
}
