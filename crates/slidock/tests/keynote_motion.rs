//! A Keynote deck through `slidock::compile`: the registered Keynote frontend reads its native motion only on the
//! deck path, where each transition plays into the slide after the one Keynote stores it on and the motion codes
//! (KN1404–KN1409) are reported; the parity path keeps Keynote's placement and reports none of them. The input is
//! Keynote's own transition catalog (`probes/keynote/RUNBOOK.md`), read from
//! `$SLIDOCK_PRIVATE/probes-incoming/keynote` and pinned by sha256; the test skips, with its reason printed, when it
//! is absent.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test code"
)]

use serde_json::Value;
use slidock::{
    BundleError, BundleOptions, BundleSink, Frontends, Input, Options, compile, slidock_generator,
};
use slidock_io::{BytesReader, sha256_range};

const CATALOG_SHA256: &str = "70c791971c821335f20ab053fa9860106614c7604a0ea0e559a40f0cda22295e";

#[derive(Default)]
struct Discard;

impl BundleSink for Discard {
    fn write(&mut self, _: &str, _: &[u8]) -> Result<(), BundleError> {
        Ok(())
    }
}

fn catalog() -> Option<Vec<u8>> {
    let private = std::env::var("SLIDOCK_PRIVATE")
        .unwrap_or_else(|_| format!("{}/.slidock", std::env::var("HOME").unwrap()));
    let path =
        std::path::Path::new(&private).join("probes-incoming/keynote/transition-catalog.key");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!(
            "skipped: {} is absent (the attended Keynote step)",
            path.display()
        );
        return None;
    };
    let reader = BytesReader::new(bytes.clone());
    assert_eq!(
        sha256_range(&reader, 0, bytes.len() as u64).unwrap(),
        CATALOG_SHA256
    );
    Some(bytes)
}

/// The deck's slides and the codes of the compile's motion diagnostics.
fn deck(bytes: &[u8], native_motion: bool) -> (Vec<Value>, Vec<String>) {
    let reader = BytesReader::new(bytes.to_vec());
    let bundled = compile(
        &Input::new("transition-catalog.key", &reader),
        Frontends::default(),
        &Options {
            native_motion,
            generator: slidock_generator(),
            ..Options::default()
        },
        &BundleOptions::default(),
        &mut Discard,
    )
    .unwrap();
    let slides = serde_json::to_value(&bundled.deck.slides)
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    let motion = bundled
        .diagnostics
        .iter()
        .map(|d| d.code.code().to_owned())
        .filter(|code| {
            ["KN1404", "KN1405", "KN1406", "KN1407", "KN1408", "KN1409"].contains(&code.as_str())
        })
        .collect();
    (slides, motion)
}

#[test]
fn keynote_transitions_play_into_the_next_slide_and_report_motion_only_on_the_deck_path() {
    let Some(bytes) = catalog() else {
        return;
    };
    let (played, reported) = deck(&bytes, true);
    let (stored, quiet) = deck(&bytes, false);
    // Slide 1 stores the Magic Move, which plays into slide 2.
    assert_eq!(
        played[1]["transition"]["native"],
        "keynote:apple:magic-move-implied-motion-path"
    );
    assert_eq!(played[1]["transition"]["effect"], "magicMove");
    assert!(played[0]["transition"].is_null());
    assert_eq!(
        stored[0]["transition"]["native"],
        "keynote:apple:magic-move-implied-motion-path"
    );
    assert!(stored[0]["transition"].get("effect").is_none());
    assert!(!reported.is_empty());
    assert!(quiet.is_empty(), "{quiet:?}");
}
