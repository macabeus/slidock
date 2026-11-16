//! Every synthetic presentation: analyze/lower parity, lowering each slide alone exactly as in the whole
//! presentation, the same bytes twice, and a clean compile to HTML.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use slidock_ir::ids::SlideId;
use support::{compile, fixtures, golden_bytes, lower, lower_with};

/// A slide with the layout it shows, as canonical JSON.
fn chain(presentation: &serde_json::Value, id: &str) -> String {
    let slide = presentation["slides"]
        .as_array()
        .unwrap()
        .iter()
        .find(|slide| slide["id"] == id)
        .cloned();
    let layout = slide
        .as_ref()
        .and_then(|slide| slide["layout"].as_str())
        .map(|layout| presentation["layouts"][layout].clone());
    slidock_canon::to_canonical_json(
        &serde_json::json!({ "slide": slide, "layout": layout }),
        slidock_canon::Layout::Compact,
    )
    .unwrap()
}

#[test]
fn lowers_the_slides_analyze_lists_and_the_same_bytes_twice() {
    for (name, build) in fixtures::all() {
        let first = lower(&build());
        let listed: Vec<(String, bool)> = first
            .analysis
            .slides
            .iter()
            .map(|slide| (slide.id.as_str().to_owned(), slide.hidden))
            .collect();
        let lowered: Vec<(String, bool)> = first
            .presentation
            .slides
            .iter()
            .map(|slide| (slide.id.as_str().to_owned(), slide.hidden))
            .collect();
        assert_eq!(lowered, listed, "{name}");
        assert_eq!(
            golden_bytes(&lower(&build()), None),
            golden_bytes(&first, None),
            "{name}"
        );
    }
}

#[test]
fn lowers_each_slide_alone_exactly_as_in_the_whole_presentation() {
    for (name, build) in fixtures::all() {
        let bytes = build();
        let whole = lower(&bytes);
        for slide in &whole.presentation.slides {
            let only = [SlideId::new(slide.id.as_str()).unwrap()];
            let alone = lower_with(&bytes, Some(&only), Default::default());
            let ids: Vec<&str> = alone
                .presentation
                .slides
                .iter()
                .map(|s| s.id.as_str())
                .collect();
            assert_eq!(ids, [slide.id.as_str()], "{name}");
            assert_eq!(
                chain(&alone.json, slide.id.as_str()),
                chain(&whole.json, slide.id.as_str()),
                "{name}/{}",
                slide.id.as_str()
            );
            assert_eq!(
                alone.lowered.features.get(slide.id.as_str()),
                whole.lowered.features.get(slide.id.as_str()),
                "{name}/{}",
                slide.id.as_str()
            );
        }
    }
}

#[test]
fn compiles_each_fixture_to_a_lint_clean_file_set() {
    for (name, build) in fixtures::all() {
        let out = compile(&build());
        assert_eq!(out.errors(), Vec::<&str>::new(), "{name}");
        assert_eq!(out.documents.unwrap().lint(), vec![], "{name}");
    }
}
