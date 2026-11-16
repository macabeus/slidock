//! Seeded mutations for the hostile-input tests: structure-blind byte mutations, and structure-aware mutations of
//! an XML part that keep it well formed, so the readers past the parser see hostile trees and values rather than
//! parse errors.

use slidock_io::roxmltree::{Document, Node, ParsingOptions};

/// A seeded PRNG (mulberry32), so fuzz runs are reproducible from their seed; each call is in [0, 1).
pub fn seeded(seed: u32) -> impl FnMut() -> f64 {
    let mut state = seed;
    move || {
        state = state.wrapping_add(0x6d2b_79f5);
        let mut t = state;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }
}

fn below(random: &mut impl FnMut() -> f64, n: usize) -> usize {
    ((random() * n as f64) as usize).min(n.saturating_sub(1))
}

/// One structure-blind mutation: a truncation, a byte flip, a run of random bytes, or a duplicated slice.
pub fn mutate(bytes: &[u8], random: &mut impl FnMut() -> f64) -> Vec<u8> {
    let at = below(random, bytes.len().max(1)).min(bytes.len());
    let mut out = bytes.to_vec();
    match below(random, 4) {
        0 => out.truncate(at),
        1 => {
            if let Some(byte) = out.get_mut(at) {
                *byte ^= 1 << below(random, 8);
            }
        }
        2 => {
            for byte in out.iter_mut().skip(at).take(8) {
                *byte = below(random, 256) as u8;
            }
        }
        _ => {
            let length = below(random, 64);
            let end = (at + length).min(bytes.len());
            let copy = bytes[at..end].to_vec();
            out.splice(at..at, copy);
        }
    }
    out
}

pub const FUZZ_ITERATIONS: usize = 200;

/// The iterations a fuzz loop runs: `FUZZ_ITERATIONS` from the environment, else [`FUZZ_ITERATIONS`].
pub fn iterations() -> usize {
    std::env::var("FUZZ_ITERATIONS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(FUZZ_ITERATIONS)
}

/// An element as written, to mutate and print again.
#[derive(Clone, Debug)]
struct Tree {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Child>,
}

#[derive(Clone, Debug)]
enum Child {
    Element(Tree),
    Text(String),
}

fn read(node: Node<'_, '_>) -> Tree {
    Tree {
        name: slidock_io::xml::qualified_name(node).to_owned(),
        attrs: slidock_io::xml::attributes(node, None)
            .into_iter()
            .collect(),
        children: node
            .children()
            .filter_map(|child| {
                if child.is_element() {
                    Some(Child::Element(read(child)))
                } else {
                    child
                        .text()
                        .filter(|_| child.is_text())
                        .map(|text| Child::Text(text.to_owned()))
                }
            })
            .collect(),
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn print(tree: &Tree, out: &mut String) {
    out.push('<');
    out.push_str(&tree.name);
    for (name, value) in &tree.attrs {
        out.push_str(&format!(" {name}=\"{}\"", escape(value)));
    }
    if tree.children.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    for child in &tree.children {
        match child {
            Child::Text(text) => out.push_str(&escape(text)),
            Child::Element(element) => print(element, out),
        }
    }
    out.push_str(&format!("</{}>", tree.name));
}

/// Every element's path of child indexes from the root, in document order.
fn paths(tree: &Tree, at: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    out.push(at.clone());
    for (index, child) in tree.children.iter().enumerate() {
        if let Child::Element(element) = child {
            at.push(index);
            paths(element, at, out);
            at.pop();
        }
    }
}

fn get_mut<'t>(tree: &'t mut Tree, path: &[usize]) -> &'t mut Tree {
    let mut at = tree;
    for &index in path {
        at = match &mut at.children[index] {
            Child::Element(element) => element,
            Child::Text(_) => unreachable!("paths name elements"),
        };
    }
    at
}

const EXTREMES: [&str; 9] = [
    "0",
    "-1",
    "9007199254740992",
    "9e15",
    "1e308cm",
    "-99999999999999999999",
    "NaN",
    "",
    "1e308",
];

/// `count` structure-aware mutations of a part's bytes: elements dropped or duplicated, attribute values swapped
/// between elements, values replaced with extremes. The result parses as XML whenever the input did.
pub fn mutate_xml(bytes: &[u8], random: &mut impl FnMut() -> f64, count: usize) -> Vec<u8> {
    let text = std::str::from_utf8(bytes).unwrap();
    let options = ParsingOptions {
        allow_dtd: false,
        ..ParsingOptions::default()
    };
    let document = Document::parse_with_options(text, options).unwrap();
    let mut root = read(document.root_element());
    for _ in 0..count {
        let mut all = Vec::new();
        paths(&root, &mut Vec::new(), &mut all);
        let target = all[below(random, all.len())].clone();
        let names: Vec<String> = get_mut(&mut root, &target)
            .attrs
            .iter()
            .map(|(name, _)| name.clone())
            .filter(|name| !name.starts_with("xmlns"))
            .collect();
        match below(random, 4) {
            0 => {
                if let Some((&last, parent)) = target.split_last() {
                    get_mut(&mut root, parent).children.remove(last);
                }
            }
            1 => {
                if let Some((&last, parent)) = target.split_last() {
                    let parent = get_mut(&mut root, parent);
                    let copy = parent.children[last].clone();
                    parent.children.insert(last, copy);
                }
            }
            2 => {
                let other = all[below(random, all.len())].clone();
                let other_names: Vec<String> = get_mut(&mut root, &other)
                    .attrs
                    .iter()
                    .map(|(name, _)| name.clone())
                    .collect();
                if names.is_empty() || other_names.is_empty() {
                    continue;
                }
                let a = names[below(random, names.len())].clone();
                let b = other_names[below(random, other_names.len())].clone();
                if b.starts_with("xmlns") {
                    continue;
                }
                let value_of = |tree: &mut Tree, name: &str| {
                    tree.attrs
                        .iter()
                        .find(|(key, _)| key == name)
                        .map(|(_, value)| value.clone())
                        .unwrap_or_default()
                };
                let first = value_of(get_mut(&mut root, &target), &a);
                let second = value_of(get_mut(&mut root, &other), &b);
                let set = |tree: &mut Tree, name: &str, value: String| {
                    if let Some(slot) = tree.attrs.iter_mut().find(|(key, _)| key == name) {
                        slot.1 = value;
                    }
                };
                set(get_mut(&mut root, &target), &a, second);
                set(get_mut(&mut root, &other), &b, first);
            }
            _ => {
                if names.is_empty() {
                    continue;
                }
                let name = names[below(random, names.len())].clone();
                let value = EXTREMES[below(random, EXTREMES.len())].to_owned();
                if let Some(slot) = get_mut(&mut root, &target)
                    .attrs
                    .iter_mut()
                    .find(|(key, _)| *key == name)
                {
                    slot.1 = value;
                }
            }
        }
    }
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
    print(&root, &mut out);
    out.into_bytes()
}
