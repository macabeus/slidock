//! The frontend's goldens (`--features golden`, tests/golden; skips when they are absent): each synthetic presentation,
//! built in Rust, and the private `basic.key`, saved by Keynote 15.4, lower to the source IR, diagnostics and features
//! recorded (`slidock-keynote/frontend/<name>.json`), byte for byte; `basic.key` also compiles clean.
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

use golden::Golden;
use support::fixtures;
use support::harness::{check_fixture, golden_bytes, lower};

/// `basic.key`, saved by Keynote 15.4.
fn basic_key(golden: &Golden) -> Vec<u8> {
    std::fs::read(golden.fixture("keynote/basic.key")).unwrap()
}

#[test]
fn lowers_every_fixture_as_recorded() {
    let Some(golden) = Golden::open("slidock-keynote frontend") else {
        return;
    };
    let mut cases: Vec<(&str, Vec<u8>)> = fixtures::synthetic()
        .into_iter()
        .map(|(name, build)| (name, build()))
        .collect();
    cases.push(("basic-key", basic_key(&golden)));
    let mut failures = Vec::new();
    for (name, bytes) in &cases {
        let actual = golden_bytes(&lower(bytes));
        failures.extend(golden.record(
            &format!("slidock-keynote/frontend/{name}.json"),
            actual.as_bytes(),
        ));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    assert_eq!(cases.len(), 5);
}

#[test]
fn lowers_and_compiles_basic_key() {
    let Some(golden) = Golden::open("slidock-keynote basic.key") else {
        return;
    };
    check_fixture("basic-key", &basic_key(&golden));
}
