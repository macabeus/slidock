//! The golden files (`--features golden`, tests/golden; skips when they are absent): the resolved IR `slidock compile`
//! recorded for each IR fixture prints the document set it recorded, byte for byte, and the private full resolved
//! presentation prints the one this suite recorded (`slidock-html/full/`); each backend report is the recorded one
//! (`slidock-html/<name>.report.json`).
#![cfg(feature = "golden")]
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod common;

#[path = "../../../tests/golden/support.rs"]
mod golden;

use std::collections::BTreeMap;
use std::sync::OnceLock;

use golden::Golden;
use serde_json::Value;
use slidock_format::dom::Document;
use slidock_format::manifest::Generator;
use slidock_format::markers::{PARAGRAPH, TEXT};
use slidock_html::{EmitOptions, Emitted, Provenance, emit_documents};
use slidock_ir::resolved::Element;
use slidock_ir::table::table_cells;
use slidock_ir::text::{Inline, ResolvedBullet, ResolvedParagraph};
use slidock_ir::{parse, resolved};

/// The golden files, opened once; `None` when they are absent.
fn golden() -> Option<&'static Golden> {
    static GOLDEN: OnceLock<Option<Golden>> = OnceLock::new();
    GOLDEN.get_or_init(|| Golden::open("slidock-html")).as_ref()
}

fn opened() -> &'static Golden {
    golden().unwrap()
}

/// The private full resolved presentation.
const FULL: &str = "full";
const FULL_FIXTURE: &str = "presentation/resolved/full.presentation-ir.json";

/// The golden names: one per IR fixture `slidock compile` recorded, and the full resolved presentation.
fn names() -> Vec<String> {
    let mut names = opened().ir_fixtures();
    names.push(FULL.to_owned());
    names
}

fn presentation(name: &str) -> resolved::Presentation {
    let text = if name == FULL {
        std::fs::read_to_string(opened().fixture(FULL_FIXTURE)).unwrap()
    } else {
        opened().output_text(&format!("fixtures/{name}/presentation-ir.json"))
    };
    parse::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// The provenance the recorded manifest names, as the driver hands it to the backend; for the full presentation, its
/// file's.
fn provenance(name: &str) -> Provenance {
    if name == FULL {
        let bytes = std::fs::read(opened().fixture(FULL_FIXTURE)).unwrap();
        return Provenance {
            format: "ir".to_owned(),
            name: "full.presentation-ir.json".to_owned(),
            sha256: slidock_canon::sha256_hex(&bytes),
            bytes: bytes.len() as u64,
        };
    }
    let manifest: Value =
        serde_json::from_slice(&opened().output(&format!("fixtures/{name}/presentation.json")))
            .unwrap();
    let source = &manifest["source"];
    Provenance {
        format: source["format"].as_str().unwrap().to_owned(),
        name: source["name"].as_str().unwrap().to_owned(),
        sha256: source["sha256"].as_str().unwrap().to_owned(),
        bytes: source["bytes"].as_u64().unwrap(),
    }
}

/// The generator the CLI names.
fn generator() -> Generator {
    Generator {
        name: "slidock".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    }
}

fn emit(name: &str) -> Emitted {
    let options = EmitOptions {
        provenance: provenance(name),
        generator: generator(),
    };
    emit_documents(&presentation(name), &options).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn prints_every_golden_file_set_byte_for_byte() {
    let Some(golden) = golden() else {
        return;
    };
    let mut compared = 0;
    let mut mismatched = Vec::new();
    for name in names() {
        let emitted = emit(&name);
        if name == FULL {
            for (path, text) in &emitted.files.text {
                compared += 1;
                let golden_path = format!("slidock-html/{FULL}/{path}");
                mismatched.extend(golden.record(&golden_path, text.as_bytes()));
            }
            continue;
        }
        let documents: Vec<String> = golden
            .output_paths(&format!("fixtures/{name}"))
            .into_iter()
            .filter(|path| {
                !path.starts_with("media/")
                    && path != "presentation-ir.json"
                    && path != "report.json"
            })
            .collect();
        let printed: Vec<&String> = emitted.files.text.keys().collect();
        assert_eq!(
            printed,
            documents.iter().collect::<Vec<_>>(),
            "{name}: file set"
        );
        for (path, text) in &emitted.files.text {
            compared += 1;
            mismatched.extend(golden.compare(&format!("fixtures/{name}/{path}"), text.as_bytes()));
        }
    }
    assert!(mismatched.is_empty(), "{}", mismatched.join("\n\n"));
    // 7 presentations: 29 documents and manifests.
    assert_eq!(compared, 29, "files compared");
    eprintln!("compared {compared} files byte for byte");
}

#[test]
fn writes_every_golden_report() {
    let Some(golden) = golden() else {
        return;
    };
    let mut compared = 0;
    for name in names() {
        let report = emit(&name).report().unwrap();
        let problem = golden.record(
            &format!("slidock-html/{name}.report.json"),
            report.as_bytes(),
        );
        assert!(problem.is_none(), "{}", problem.unwrap_or_default());
        compared += 1;
    }
    assert_eq!(compared, 7, "reports compared");
}

#[test]
fn every_golden_file_set_lints_clean() {
    if golden().is_none() {
        return;
    }
    for name in names() {
        assert_eq!(emit(&name).files.lint(), vec![], "{name}");
    }
}

#[test]
fn writes_the_same_bytes_twice() {
    if golden().is_none() {
        return;
    }
    for name in names() {
        assert_eq!(emit(&name), emit(&name), "{name}");
    }
}
/// The text a paragraph shows: its label (when it has text), runs, cached fields and tabs.
fn paragraph_text(paragraph: &ResolvedParagraph) -> String {
    let mut out = String::new();
    if let ResolvedBullet::Label { text, .. } = &paragraph.props.bullet
        && !paragraph.shows_no_text()
    {
        out.push_str(text);
    }
    for inline in &paragraph.inlines {
        match inline {
            Inline::Run { text, .. } => out.push_str(text),
            Inline::Field { cached, .. } => out.push_str(cached),
            Inline::Tab {} => out.push('\t'),
            Inline::Break { .. } => {}
        }
    }
    out
}

fn walk<'a>(elements: &'a [Element], out: &mut BTreeMap<String, &'a Element>) {
    for element in elements {
        out.insert(element.id().to_string(), element);
        match element {
            Element::Group(group) => walk(&group.children, out),
            Element::Foreign(foreign) => {
                if let Some(children) = foreign.fallback.as_ref().and_then(|f| f.elements.as_ref())
                {
                    walk(children, out);
                }
            }
            _ => {}
        }
    }
}

#[test]
fn adds_no_whitespace_inside_a_paragraph() {
    if golden().is_none() {
        return;
    }
    for name in names() {
        let presentation = presentation(&name);
        let mut printed = BTreeMap::new();
        for master in presentation.masters.values() {
            walk(master.body.elements(), &mut printed);
        }
        for layout in presentation.layouts.values() {
            walk(layout.body.elements(), &mut printed);
        }
        for slide in &presentation.slides {
            walk(slide.body.elements(), &mut printed);
        }
        let emitted = emit(&name);
        let mut checked = 0;
        for (path, html) in &emitted.files.text {
            if !path.ends_with(".html") {
                continue;
            }
            let doc = Document::parse(html);
            for node in common::elements(&doc) {
                let Some(found) = doc
                    .attribute(node, "data-el")
                    .and_then(|id| printed.get(id))
                else {
                    continue;
                };
                let bodies: Vec<_> = match found {
                    Element::Shape(shape) => shape.text.iter().collect(),
                    Element::Table(table) => table_cells(&table.rows)
                        .map(|(cell, _, _)| &cell.text)
                        .collect(),
                    _ => continue,
                };
                let paragraphs: Vec<String> = common::descendants(&doc, node)
                    .into_iter()
                    .filter(|child| matches!(doc.tag_name(*child), "p" | "li"))
                    .map(|child| common::deep_text(&doc, child))
                    .collect();
                let expected: Vec<String> = bodies
                    .iter()
                    .flat_map(|body| body.paragraphs.iter().map(paragraph_text))
                    .collect();
                assert_eq!(paragraphs, expected, "{name} {path} {}", found.id());
                checked += paragraphs.len();
            }
        }
        if name != "agent" && name != "min" {
            assert!(checked > 0, "{name}");
        }
    }
}

/// The presentation with every inheritance edge and explicit placeholder binding removed.
fn without_inheritance(elements: &mut [Element]) {
    macro_rules! strip {
        ($e:expr) => {{
            $e.inherits = None;
            if let Some(placeholder) = &mut $e.placeholder {
                placeholder.target = None;
            }
        }};
    }
    for element in elements {
        match element {
            Element::Shape(e) => strip!(e),
            Element::Picture(e) => strip!(e),
            Element::Media(e) => strip!(e),
            Element::Table(e) => strip!(e),
            Element::Group(e) => {
                strip!(e);
                without_inheritance(&mut e.children);
            }
            Element::Foreign(e) => {
                strip!(e);
                if let Some(children) = e.fallback.as_mut().and_then(|f| f.elements.as_mut()) {
                    without_inheritance(children);
                }
            }
        }
    }
}

fn elements_mut<B>(
    body: &mut resolved::Body<B>,
    get: impl Fn(&mut B) -> &mut Vec<Element>,
) -> &mut [Element] {
    match body {
        resolved::Body::Built(built) => get(built),
        resolved::Body::Failed { .. } => &mut [],
    }
}

#[test]
fn prints_the_same_files_with_every_inheritance_edge_removed() {
    if golden().is_none() {
        return;
    }
    for name in names() {
        let mut stripped = presentation(&name);
        for master in stripped.masters.values_mut() {
            without_inheritance(elements_mut(&mut master.body, |b| &mut b.elements));
        }
        for layout in stripped.layouts.values_mut() {
            without_inheritance(elements_mut(&mut layout.body, |b| &mut b.elements));
        }
        for slide in &mut stripped.slides {
            without_inheritance(elements_mut(&mut slide.body, |b| &mut b.elements));
        }
        let options = EmitOptions {
            provenance: provenance(&name),
            generator: generator(),
        };
        assert_eq!(
            emit_documents(&stripped, &options).unwrap(),
            emit(&name),
            "{name}"
        );
    }
}

#[test]
fn places_every_element_without_transform_and_numbers_its_paragraphs() {
    if golden().is_none() {
        return;
    }
    let mut boxes = 0;
    for name in names() {
        for (path, html) in &emit(&name).files.text {
            if !path.ends_with(".html") {
                continue;
            }
            let doc = Document::parse(html);
            for node in common::elements(&doc) {
                let style = doc.attribute(node, "style").unwrap_or("");
                assert!(!style.contains("transform"), "{name} {path}: {style}");
                if doc.attribute(node, "data-el").is_some() {
                    assert!(
                        style.starts_with("position:absolute;left:"),
                        "{name} {path}: {style}"
                    );
                    let names: Vec<&str> = doc.attrs(node).iter().map(|a| a.local()).collect();
                    assert_eq!(
                        names.get(..2),
                        Some(&["data-el", "data-kind"][..]),
                        "{name} {path}"
                    );
                }
                if doc.has_attribute(node, TEXT) {
                    let indices: Vec<&str> = common::descendants(&doc, node)
                        .into_iter()
                        .filter(|child| matches!(doc.tag_name(*child), "p" | "li"))
                        .map(|child| doc.attribute(child, PARAGRAPH).unwrap())
                        .collect();
                    let expected: Vec<String> = (0..indices.len()).map(|i| i.to_string()).collect();
                    assert_eq!(indices, expected, "{name} {path}");
                    boxes += 1;
                }
            }
        }
    }
    assert!(boxes > 20, "{boxes} text boxes");
}

mod support;

/// The printer tests' fixture, built in Rust, prints the full resolved fixture's file set and report.
#[test]
fn the_printer_fixture_prints_as_the_full_resolved_fixture() {
    if golden().is_none() {
        return;
    }
    let emitted = support::emit_with(&support::full(), provenance(FULL), generator());
    let expected = emit(FULL);
    for (path, text) in &expected.files.text {
        assert_eq!(&emitted.files.text[path], text, "{path}");
    }
    assert_eq!(emitted.files.text.len(), expected.files.text.len());
    assert_eq!(emitted.report().unwrap(), expected.report().unwrap());
}
