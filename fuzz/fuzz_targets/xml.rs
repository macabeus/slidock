//! An XML part as the frontends read it: decoded, parsed under the limits with a presentation's node budget, then
//! walked by the helpers the frontends call (names, namespaces, attributes by prefix, text). The input is parsed twice
//! against one budget, as two parts of a presentation are.
#![no_main]

use libfuzzer_sys::fuzz_target;
use slidock_io::roxmltree::Node;
use slidock_io::xml::{
    attributes, child_element, elements_of, namespace_uri, qualified_name, text_content,
};
use slidock_io::{XmlBudget, XmlLimits, decode_xml, parse_xml};

const PREFIXES: &[(&str, &str)] = &[
    ("http://schemas.openxmlformats.org/drawingml/2006/main", "a"),
    (
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
        "r",
    ),
    ("urn:oasis:names:tc:opendocument:xmlns:drawing:1.0", "draw"),
    ("http://www.w3.org/1999/xlink", "xlink"),
];

/// A small presentation budget, so the second parse can run out of it.
const PRESENTATION_NODES: u64 = 3_000;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = decode_xml(data) else {
        return;
    };
    let limits = XmlLimits {
        presentation_nodes: PRESENTATION_NODES,
        ..XmlLimits::default()
    };
    let mut budget = XmlBudget::new(&limits);
    for _ in 0..2 {
        let before = budget.nodes;
        let parsed = parse_xml(&text, &limits, &mut budget);
        assert!(budget.nodes <= before);
        let Ok(document) = parsed else {
            continue;
        };
        assert!(budget.nodes >= 0);
        let mut elements = 0;
        walk(document.root_element(), 1, &limits, &mut elements);
        assert!(elements <= limits.nodes);
    }
});

fn walk(node: Node<'_, '_>, depth: usize, limits: &XmlLimits, elements: &mut u64) {
    assert!(depth <= limits.depth);
    *elements += 1;
    let _ = qualified_name(node);
    let _ = namespace_uri(node);
    let named = attributes(node, None);
    assert!(named.len() <= limits.attributes);
    let _ = attributes(node, Some(PREFIXES));
    let _ = child_element(
        node,
        "http://schemas.openxmlformats.org/presentationml/2006/main",
        "sp",
    );
    if depth == 1 {
        let _ = text_content(node);
    }
    for child in elements_of(node) {
        walk(child, depth + 1, limits, elements);
    }
}
