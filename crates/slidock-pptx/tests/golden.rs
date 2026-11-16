//! Every synthetic presentation: its golden source IR (`tests/golden/<name>.json`, rewritten with
//! `UPDATE_GOLDEN=1`), analyze/lower agreement, `only` lowering the same bytes as the whole presentation, and a
//! clean compile to a lint-clean file set.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::path::PathBuf;

use slidock_core_api::Limits;
use slidock_ir::source::Presentation;
use support::fixtures::FIXTURES;
use support::{compile_bytes, golden_bytes, lint, lower, lower_with};

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.json"))
}

/// A slide with the layout, master and theme it shows, as canonical JSON.
fn chain(presentation: &Presentation, id: &str) -> String {
    let slide = presentation
        .slides
        .iter()
        .find(|slide| slide.id.as_str() == id);
    let layout = slide
        .and_then(|slide| slide.layout.as_ref())
        .and_then(|layout| presentation.layouts.get(layout));
    let master = layout
        .and_then(|layout| layout.master.as_ref())
        .and_then(|master| presentation.masters.get(master));
    let theme = master
        .and_then(|master| master.theme.as_ref())
        .and_then(|theme| presentation.themes.get(theme));
    slidock_canon::to_canonical_json(
        &(slide, layout, master, theme),
        slidock_canon::Layout::Compact,
    )
    .unwrap()
}

#[test]
fn lowers_each_fixture_to_its_golden_source_ir() {
    let update = std::env::var("UPDATE_GOLDEN").is_ok_and(|value| value == "1");
    let mut failures = Vec::new();
    for (name, build) in FIXTURES {
        let actual = golden_bytes(&lower(&build()));
        let path = golden_path(name);
        if update {
            std::fs::write(&path, &actual).unwrap();
        }
        let wanted = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        if actual != wanted {
            failures.push(name);
        }
    }
    assert!(failures.is_empty(), "goldens differ: {failures:?}");
}

#[test]
fn lowers_the_slides_analyze_lists_and_the_same_bytes_twice() {
    for (name, build) in FIXTURES {
        let first = lower(&build());
        let slides: Vec<(String, bool)> = first
            .presentation()
            .slides
            .iter()
            .map(|slide| (slide.id.as_str().to_owned(), slide.hidden))
            .collect();
        let analyzed: Vec<(String, bool)> = first
            .analysis
            .slides
            .iter()
            .map(|slide| (slide.id.as_str().to_owned(), slide.hidden))
            .collect();
        assert_eq!(slides, analyzed, "{name}");
        assert_eq!(
            golden_bytes(&lower(&build())),
            golden_bytes(&first),
            "{name}"
        );
    }
}

#[test]
fn lowers_each_slide_alone_exactly_as_in_the_whole_presentation() {
    for (name, build) in FIXTURES {
        let bytes = build();
        let whole = lower(&bytes);
        for slide in &whole.presentation().slides {
            let only = [slide.id.clone()];
            let alone = lower_with(&bytes, Some(&only), &Limits::default()).unwrap();
            let ids: Vec<&str> = alone
                .presentation()
                .slides
                .iter()
                .map(|slide| slide.id.as_str())
                .collect();
            assert_eq!(ids, [slide.id.as_str()], "{name}");
            assert_eq!(
                chain(alone.presentation(), slide.id.as_str()),
                chain(whole.presentation(), slide.id.as_str()),
                "{name} {}",
                slide.id
            );
            assert_eq!(
                alone.lowered.features.get(slide.id.as_str()),
                whole.lowered.features.get(slide.id.as_str()),
                "{name} {}",
                slide.id
            );
        }
    }
}

#[test]
fn compiles_each_fixture_to_a_lint_clean_file_set() {
    for (name, build) in FIXTURES {
        let compiled = compile_bytes(&build(), &format!("{name}.pptx"));
        assert_eq!(
            compiled.errors(),
            Vec::<&slidock_ir::diagnostics::Diagnostic>::new(),
            "{name}"
        );
        assert_eq!(lint(&compiled), Vec::<String>::new(), "{name}");
    }
}
