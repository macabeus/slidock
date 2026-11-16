//! Every synthetic Keynote fixture: the slides lowered are the ones analyze lists, lowering is deterministic and
//! lowers each slide alone exactly as in the whole presentation, and the compile is error-free and lint-clean.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use support::fixtures;
use support::harness::check_fixture;

#[test]
fn lowers_and_compiles_every_synthetic_fixture() {
    let fixtures = fixtures::synthetic();
    assert_eq!(fixtures.len(), 4);
    for (name, build) in fixtures {
        check_fixture(name, &build());
    }
}
