//! The golden files (`--features golden`, tests/golden; skips when they are absent): the sample document set composes
//! as recorded (`slidock-format/compose/`), every corpus document set `slidock compile` recorded lints clean and
//! composes every slide as recorded (`slidock-format/composed/<pair>/`), and every IR fixture's set lints clean.
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

use golden::Golden;
use slidock_format::paths::file_id;
use slidock_format::{
    MediaUrls, PresentationFiles, compose_slide, lint_presentation, read_manifest,
};

/// A recorded document set as the lint reads it: documents and the manifest as text, media present only.
fn file_set(golden: &Golden, prefix: &str) -> PresentationFiles {
    golden
        .output_paths(prefix)
        .into_iter()
        .filter(|path| path != "presentation-ir.json" && path != "report.json")
        .map(|path| {
            let text = (path.ends_with(".html") || path.ends_with(".json"))
                .then(|| golden.output_text(&format!("{prefix}/{path}")));
            (path, text)
        })
        .collect()
}

#[test]
fn reproduces_the_compose_goldens() {
    let Some(golden) = Golden::open("slidock-format compose") else {
        return;
    };
    let files = common::sample_files();
    for (id, name) in [("ir-s1", "ir-s1.html"), ("ir-S2", "ir-s2.html")] {
        let composed = compose_slide(&files, id, &MediaUrls::Unchanged).unwrap();
        let problem = golden.record(
            &format!("slidock-format/compose/{name}"),
            composed.as_bytes(),
        );
        assert!(problem.is_none(), "{}", problem.unwrap_or_default());
    }
}

#[test]
fn composes_every_corpus_slide_as_recorded() {
    let Some(golden) = Golden::open("slidock-format corpus") else {
        return;
    };
    let mut slides = 0;
    let mut problems = Vec::new();
    for pair in golden.pairs() {
        let name = &pair.name;
        let files = file_set(&golden, &format!("corpus/{name}"));
        assert_eq!(lint_presentation(&files), [], "{name}");
        let manifest = read_manifest(files["presentation.json"].as_deref().unwrap()).unwrap();
        for slide in &manifest.slides {
            let composed = compose_slide(&files, &slide.id, &MediaUrls::Unchanged).unwrap();
            let path = format!("slidock-format/composed/{name}/{}.html", file_id(&slide.id));
            problems.extend(golden.record(&path, composed.as_bytes()));
            slides += 1;
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    assert!(slides > 100, "only {slides} slides composed");
}

#[test]
fn lints_every_ir_fixture_set_clean() {
    let Some(golden) = Golden::open("slidock-format lint") else {
        return;
    };
    let mut linted = 0;
    for name in golden.ir_fixtures() {
        let files = file_set(&golden, &format!("fixtures/{name}"));
        let errors: Vec<_> = lint_presentation(&files)
            .into_iter()
            .filter(|problem| problem.is_error())
            .collect();
        assert_eq!(errors, [], "{name}");
        linted += 1;
    }
    assert!(linted >= 5, "only {linted} IR fixture sets linted");
}
