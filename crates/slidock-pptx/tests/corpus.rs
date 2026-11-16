//! The corpus against the golden files (`--features golden`, tests/golden; skips when they are absent):
//! every PPTX pair, read from `$SLIDOCK_PRIVATE/corpus` (or the repository's probes) by range, compiled through this
//! frontend, a `slidock-core` Session over the standard pass pipeline with the built-in catalog and the static lane's
//! font set over it, and `slidock-html`'s documents, gives what `slidock compile` recorded: the resolved IR, every
//! document and medium, and the report, byte for byte.
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

#[path = "../../../tests/golden/compile.rs"]
mod golden_compile;

use golden::Golden;
use slidock_core::{FrontendRegistry, PassRegistry, Pipeline, Session, SessionOptions};
use slidock_core_api::Input;
use slidock_io::FileReader;
use slidock_pptx::PptxFrontend;

#[test]
fn every_pptx_corpus_pair_compiles_as_recorded() {
    let Some(golden) = Golden::open("slidock-pptx corpus") else {
        return;
    };
    let pairs = golden_compile::pairs_of(&golden, "pptx");
    assert!(
        pairs.len() >= 3,
        "the corpus holds three PPTX sources: {pairs:?}"
    );
    let catalog = golden_compile::font_catalog(&golden);
    let mut registry = PassRegistry::new();
    for pass in slidock_passes::passes() {
        registry.register(pass).unwrap();
    }
    let pipeline = Pipeline::standard(registry).unwrap();
    let frontends = FrontendRegistry::new(vec![Box::new(PptxFrontend)]).unwrap();
    let mut failures = Vec::new();
    let mut compared = 0;
    for pair in &pairs {
        let mut options = SessionOptions::new(&frontends, &pipeline);
        options.fonts = &catalog;
        let source = golden.source(pair);
        let reader = FileReader::open(&source).unwrap();
        let file = source.file_name().unwrap().to_str().unwrap().to_owned();
        let mut session = Session::open(&Input::new(&file, &reader), options)
            .unwrap_or_else(|error| panic!("{}: {error:?}", pair.name));
        let finished = session.finish().unwrap();
        let (differences, files) =
            golden_compile::compare_pair(&golden, pair, &session, &finished, &reader);
        eprintln!("{}: {files} files, {} differ", pair.name, differences.len());
        failures.extend(differences);
        compared += files;
    }
    eprintln!(
        "{} PPTX pairs, {compared} files compared byte for byte, {} differ",
        pairs.len(),
        failures.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
