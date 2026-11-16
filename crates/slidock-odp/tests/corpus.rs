//! The golden files (`--features golden`, tests/golden; skips when they are absent): the frontend lowers each synthetic
//! presentation as recorded (`slidock-odp/frontend/<name>.json`), and every ODP corpus pair, compiled
//! natively through the passes and the HTML backend with the static lane's font set, gives what `slidock compile`
//! recorded: the resolved IR, every document and medium, and the report, byte for byte.
#![cfg(feature = "golden")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

#[path = "../../../tests/golden/support.rs"]
mod golden;

#[path = "../../../tests/golden/compile.rs"]
mod golden_compile;

use golden::Golden;
use slidock_core::{Session, SessionOptions};
use slidock_core_api::Input;
use slidock_io::FileReader;
use support::{fixtures, golden_bytes, lower};

#[test]
fn lowers_every_fixture_as_recorded() {
    let Some(golden) = Golden::open("slidock-odp frontend") else {
        return;
    };
    let mut failures = Vec::new();
    for (name, build) in fixtures::all() {
        let actual = golden_bytes(&lower(&build()), None);
        failures.extend(golden.record(
            &format!("slidock-odp/frontend/{name}.json"),
            actual.as_bytes(),
        ));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// Native motion changes nothing on the corpus pairs, which hold no transition or build: the same presentation and the
/// same diagnostics as the parity path.
#[test]
fn native_motion_lowers_the_corpus_pairs_as_the_parity_path_does() {
    let Some(golden) = Golden::open("slidock-odp native motion") else {
        return;
    };
    for pair in golden_compile::pairs_of(&golden, "odp") {
        if pair.input.starts_with("repo:") {
            continue;
        }
        let bytes = std::fs::read(golden.source(&pair)).unwrap();
        let parity = support::lower(&bytes);
        let native = support::lower_motion(&bytes);
        assert_eq!(parity.json, native.json, "{}", pair.name);
        assert_eq!(parity.diagnostics, native.diagnostics, "{}", pair.name);
        assert!(
            parity
                .diagnostics
                .iter()
                .all(|d| !d.code.code().starts_with("OD94")),
            "{}: the corpus pair holds motion",
            pair.name
        );
    }
}

#[test]
fn every_odp_corpus_pair_compiles_as_recorded() {
    let Some(golden) = Golden::open("slidock-odp corpus") else {
        return;
    };
    let pairs = golden_compile::pairs_of(&golden, "odp");
    assert!(
        pairs.len() >= 3,
        "the corpus holds three ODP sources: {pairs:?}"
    );
    let catalog = golden_compile::font_catalog(&golden);
    let (registry, pipeline) = support::setup();
    let mut failures = Vec::new();
    let mut compared = 0;
    for pair in &pairs {
        let source = golden.source(pair);
        let reader = FileReader::open(&source).unwrap();
        let name = source.file_name().unwrap().to_str().unwrap().to_owned();
        let mut options = SessionOptions::new(&registry, &pipeline);
        options.fonts = &catalog;
        let mut session = Session::open(&Input::new(&name, &reader), options)
            .unwrap_or_else(|e| panic!("{}: {e:?}", pair.name));
        let finished = session.finish().unwrap();
        let (differences, files) =
            golden_compile::compare_pair(&golden, pair, &session, &finished, &reader);
        eprintln!("{}: {files} files, {} differ", pair.name, differences.len());
        failures.extend(differences);
        compared += files;
    }
    eprintln!(
        "{} ODP pairs, {compared} files compared, {} differ",
        pairs.len(),
        failures.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
