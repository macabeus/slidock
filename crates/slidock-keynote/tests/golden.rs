//! The golden files (`--features golden`, tests/golden; skips when they are absent): the derived schema's field tables
//! and type registry as recorded (`slidock-keynote/schema.txt`), every Keynote deck's object graph as recorded
//! (`slidock-keynote/graph/<deck>.json`: its stats and issues, and every object in order with its type, file and a
//! digest of its decoded message), and the 1 GB deck opening from its index alone.
#![cfg(feature = "golden")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

#[path = "../../../tests/golden/support.rs"]
mod golden;

use std::path::{Path, PathBuf};

use golden::Golden;
use serde_json::Value;
use slidock_core_api::{Input, Limits};
use slidock_io::{FileReader, RangeReader};
use slidock_keynote::graph::IndexBudget;
use slidock_keynote::iwa::schema::{FieldKind, messages};
use slidock_keynote::iwa::{dump, is_kept, registry};
use slidock_keynote::proto::kn;
use slidock_keynote::{
    GraphIssue, KeynotePackage, ObjectGraph, ObjectId, open_document, read_object_graph,
};

fn kind_name(kind: &FieldKind) -> String {
    match kind {
        FieldKind::Message(at) => format!("message {}", messages()[*at].name),
        other => format!("{other:?}").to_lowercase(),
    }
}

/// The trimmed schema as text: each message with its fields (number, name, repeated, kind), then each registered type
/// id with its name and whether the decoder keeps it.
fn schema_text() -> String {
    let mut out = String::new();
    for message in messages() {
        out.push_str(&format!("message {}\n", message.name));
        for field in message.fields {
            out.push_str(&format!(
                "  {} {}{} {}\n",
                field.number,
                field.name,
                if field.repeated { " repeated" } else { "" },
                kind_name(&field.kind)
            ));
        }
    }
    for (id, name) in registry() {
        out.push_str(&format!(
            "type {id} {name}{}\n",
            if is_kept(id) { " kept" } else { "" }
        ));
    }
    out
}

#[test]
fn the_schema_is_as_recorded_field_for_field() {
    let Some(golden) = Golden::open("slidock-keynote schema") else {
        return;
    };
    assert_eq!(messages().len(), 119);
    let problem = golden.record("slidock-keynote/schema.txt", schema_text().as_bytes());
    assert!(problem.is_none(), "{}", problem.unwrap_or_default());
}

/// The Keynote decks: every corpus pair, and the private `basic.key` fixture.
fn decks(golden: &Golden) -> Vec<(String, PathBuf)> {
    let mut decks: Vec<(String, PathBuf)> = golden
        .pairs()
        .iter()
        .filter(|pair| pair.frontend == "keynote")
        .map(|pair| (pair.presentation.clone(), golden.source(pair)))
        .collect();
    decks.push(("basic".to_owned(), golden.fixture("keynote/basic.key")));
    decks
}

fn graph_of(path: &Path) -> ObjectGraph {
    let reader = FileReader::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let package = KeynotePackage::open("deck.key", reader, Limits::default().zip)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    read_object_graph(&package, &IndexBudget::default()).unwrap()
}

fn stats_json(graph: &ObjectGraph) -> Value {
    let s = &graph.stats;
    let unknown: Vec<Value> = s
        .unknown_type_ids
        .iter()
        .map(|(id, count)| serde_json::json!([id.map_or(-1, i64::from), count]))
        .collect();
    serde_json::json!({
        "objects": s.objects,
        "decoded": s.decoded,
        "skipped": s.skipped,
        "unknownTypeIds": unknown,
        "decodeErrors": s.decode_errors,
        "mergeRecords": s.merge_records,
        "extraPayloads": s.extra_payloads,
        "messages": s.messages,
    })
}

fn issues_json(graph: &ObjectGraph) -> Value {
    graph
        .issues
        .iter()
        .map(|issue| match issue {
            GraphIssue::MergeUnapplied { id, file } => {
                serde_json::json!({ "kind": "merge-unapplied", "id": id.to_string(), "file": file })
            }
            GraphIssue::FileUnreadable { file, reason } => {
                serde_json::json!({ "kind": "file-unreadable", "file": file, "reason": reason })
            }
        })
        .collect()
}

/// A graph as recorded: its stats and issues, and every object in order as `[id, type id, type name, file, digest of
/// its decoded message]`.
fn graph_json(graph: &ObjectGraph) -> Value {
    let objects: Vec<Value> = graph
        .objects()
        .map(|object| {
            let digest = object
                .archive
                .as_ref()
                .map(|archive| slidock_canon::sha256_hex(dump(archive).unwrap()));
            serde_json::json!([
                object.id.to_string(),
                object.type_id.map_or(-1, i64::from),
                object.type_name,
                graph.file_of(object.id),
                digest
            ])
        })
        .collect();
    serde_json::json!({
        "stats": stats_json(graph),
        "issues": issues_json(graph),
        "objects": objects,
    })
}

#[test]
fn every_keynote_deck_reads_its_object_graph_as_recorded() {
    let Some(golden) = Golden::open("slidock-keynote graphs") else {
        return;
    };
    let mut failures = Vec::new();
    let mut summary = Vec::new();
    for (deck, path) in decks(&golden) {
        let graph = graph_of(&path);
        let text =
            slidock_canon::to_canonical_json(&graph_json(&graph), slidock_canon::Layout::Indented)
                .unwrap();
        failures.extend(golden.record(
            &format!("slidock-keynote/graph/{deck}.json"),
            text.as_bytes(),
        ));
        summary.push(format!(
            "{deck}: {} objects, {} messages costing {} bytes",
            graph.objects().count(),
            graph.stats.messages,
            graph.stats.decoded_bytes
        ));
    }
    println!("{}", summary.join("\n"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_large_deck_opens_reading_only_its_index() {
    let Some(golden) = Golden::open("slidock-keynote large deck") else {
        return;
    };
    let path = golden.input("corpus:retro.keynote");
    let reader = FileReader::open(&path).unwrap();
    assert!(reader.len() > 1_000_000_000, "the 1 GB deck");
    let input = Input::new("en-retro-game-decompilation copy.key", &reader);
    let document = open_document(&input, &Limits::default()).unwrap();
    let read = document.index_bytes_read();
    assert!(read < 4 * 1024 * 1024, "read {read} bytes");
    println!("the 1 GB deck read {read} bytes");
    assert_eq!(document.graph.stats.merge_records, 3);
    // The three builds that exist only as change records stand in for their objects.
    for id in [5_213_335, 5_220_478, 5_225_405] {
        let id = ObjectId(id);
        assert!(
            document.graph.get_id::<kn::BuildArchive>(id).is_some(),
            "{id}"
        );
    }
}
