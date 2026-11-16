//! The bench's private files against the schema: the corpus manifest and every lane's baseline under
//! `$SLIDOCK_PRIVATE` (default `~/.slidock`) parse strictly and print back byte for byte, and no committed file of the
//! crate or of the UI's generated declarations names a private deck. Each test says it skipped, and passes, when the
//! private files are absent.
#![cfg(feature = "golden")]
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use std::path::{Path, PathBuf};

use slidock_bench_schema::corpus::CorpusManifest;
use slidock_bench_schema::results::Results;

fn private() -> PathBuf {
    std::env::var_os("SLIDOCK_PRIVATE")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap()).join(".slidock"))
}

/// The private manifest (`SLIDOCK_CORPUS_MANIFEST`, as the bench reads it), or `None` with a message.
fn manifest_path() -> Option<PathBuf> {
    let path = std::env::var_os("SLIDOCK_CORPUS_MANIFEST")
        .filter(|path| !path.is_empty())
        .map_or_else(
            || private().join("bench/corpus/manifest.json"),
            PathBuf::from,
        );
    if path.is_file() {
        Some(path)
    } else {
        eprintln!("skipped: no private corpus manifest at {}", path.display());
        None
    }
}

/// The baselines (`results.json` of the static lane, `<lane>-results.json` of the others) under
/// `SLIDOCK_BENCH_BASELINE`; kept copies (`results.<name>.json`) are earlier formats and are not read.
fn baselines() -> Vec<PathBuf> {
    let dir = std::env::var_os("SLIDOCK_BENCH_BASELINE")
        .filter(|path| !path.is_empty())
        .map_or_else(|| private().join("bench/baseline"), PathBuf::from);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("results.json"))
        })
        .collect();
    files.sort();
    files
}

/// `text` parsed by `parse` prints back as `text`; returns the first line that differs otherwise.
fn round_trips(text: &str, printed: &str) -> Result<(), String> {
    if text == printed {
        return Ok(());
    }
    let line = text
        .lines()
        .zip(printed.lines())
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| text.lines().count().min(printed.lines().count()));
    Err(format!("first differs at line {}", line + 1))
}

#[test]
fn the_private_manifest_parses_strictly_and_prints_back_byte_for_byte() {
    let Some(path) = manifest_path() else { return };
    let text = std::fs::read_to_string(&path).unwrap();
    let manifest =
        CorpusManifest::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(!manifest.presentations.is_empty());
    round_trips(&text, &manifest.to_canonical().unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

#[test]
fn every_baseline_parses_strictly_and_prints_back_byte_for_byte() {
    let files = baselines();
    if files.is_empty() {
        eprintln!("skipped: no private baseline");
        return;
    }
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        let results = Results::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(!results.rows.is_empty(), "{}", path.display());
        round_trips(&text, &results.to_canonical().unwrap())
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
}

/// Every file of the crate and the UI's generated declarations.
fn committed_files() -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else if path
                .extension()
                .is_some_and(|e| e == "rs" || e == "json" || e == "ts" || e == "toml")
            {
                let text = std::fs::read_to_string(&path).unwrap();
                out.push((path, text));
            }
        }
    }
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in ["src", "tests", "examples"] {
        if crate_dir.join(dir).is_dir() {
            walk(&crate_dir.join(dir), &mut files);
        }
    }
    walk(
        &crate_dir.join("../../apps/bench-web/src/schema"),
        &mut files,
    );
    files.push((
        crate_dir.join("Cargo.toml"),
        read_to(&crate_dir.join("Cargo.toml")),
    ));
    files
}

fn read_to(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// What names a private deck: ids, titles, file names, notes, slide ids and every hash.
#[derive(Default)]
struct PrivateTerms {
    /// Matched as whole tokens (`[A-Za-z0-9._-]+`).
    tokens: std::collections::BTreeSet<String>,
    /// Matched anywhere.
    texts: std::collections::BTreeSet<String>,
}

impl PrivateTerms {
    fn of(manifest: &serde_json::Value) -> PrivateTerms {
        fn hashes(value: &serde_json::Value, terms: &mut PrivateTerms) {
            let is_hash = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit());
            match value {
                serde_json::Value::String(s) if is_hash(s) => {
                    terms.texts.insert(s.clone());
                }
                serde_json::Value::Array(items) => items.iter().for_each(|v| hashes(v, terms)),
                serde_json::Value::Object(map) => {
                    for (key, v) in map {
                        if is_hash(key) {
                            terms.texts.insert(key.clone());
                        }
                        hashes(v, terms);
                    }
                }
                _ => {}
            }
        }
        let mut terms = PrivateTerms::default();
        for p in manifest["presentations"].as_array().unwrap() {
            hashes(p, &mut terms);
            terms.tokens.insert(p["id"].as_str().unwrap().to_owned());
            if let Some(title) = p["title"].as_str() {
                terms.texts.insert(title.to_owned());
            }
            for source in p["sources"].as_array().unwrap() {
                terms
                    .texts
                    .insert(source["file"].as_str().unwrap().to_owned());
                for row in source["rows"].as_array().into_iter().flatten() {
                    terms.tokens.insert(row.as_str().unwrap().to_owned());
                }
            }
            for key in ["file", "source"] {
                if let Some(file) = p["oracle"][key].as_str() {
                    terms.texts.insert(file.to_owned());
                }
            }
            for losses in p["knownLoss"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.values())
            {
                for loss in losses.as_array().unwrap() {
                    terms
                        .texts
                        .insert(loss["note"].as_str().unwrap().to_owned());
                }
            }
        }
        terms
    }

    fn hits(&self, text: &str) -> Vec<String> {
        let mut hits: Vec<String> = self
            .texts
            .iter()
            .filter(|term| text.contains(term.as_str()))
            .cloned()
            .collect();
        let tokens: std::collections::BTreeSet<&str> = text
            .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
            .collect();
        hits.extend(
            self.tokens
                .iter()
                .filter(|term| tokens.contains(term.as_str()))
                .cloned(),
        );
        hits
    }
}

#[test]
fn the_committed_files_name_no_private_deck() {
    let Some(path) = manifest_path() else { return };
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let terms = PrivateTerms::of(&manifest);
    assert!(
        terms.tokens.len() > 500 && terms.texts.len() > 100,
        "{} {}",
        terms.tokens.len(),
        terms.texts.len()
    );
    let files = committed_files();
    assert!(files.len() > 100, "{}", files.len());
    let hits: Vec<String> = files
        .iter()
        .flat_map(|(path, text)| {
            terms
                .hits(text)
                .into_iter()
                .map(move |hit| format!("{}: {hit}", path.display()))
        })
        .collect();
    assert!(hits.is_empty(), "{hits:#?}");
}
