//! The golden files (`--features golden`, tests/golden; skips when they are absent): the manager and verify-source on
//! the private full and minimal presentations, in both verify modes, with identity passes, a lying pass and a throwing
//! one, print the canonical JSON slidock recorded (presentation and sorted diagnostics, every message and the failure
//! hashes included), under `slidock-core/`, byte for byte.
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
use serde_json::{Value, json};
use slidock_core::{RunResult, VerifyMode, verify_source_presentation};
use slidock_core_api::{ContainerOrder, Pass, PassError, Unverified, sort_diagnostics};
use slidock_ir::verify::Invariant;
use support::*;

/// A private presentation fixture marked as source (a resolved one read as source).
fn fixture(golden: &Golden, phase: &str, name: &str) -> Value {
    let path = golden.fixture(&format!("presentation/{phase}/{name}.presentation-ir.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut value: Value = serde_json::from_str(&text).unwrap();
    value["phase"] = json!("source");
    value
}

fn thrower(layout: bool) -> Pass {
    slide_pass("thrower", move |view, _| {
        let hit = if layout {
            view.container.kind() == "layout"
        } else {
            view.container.id().as_str() == "ir-s2"
        };
        if hit {
            return Err(PassError::failed("boom"));
        }
        Ok(view.unchanged())
    })
}

const MODES: [VerifyMode; 2] = [VerifyMode::Each, VerifyMode::Boundaries];

/// Each verify mode's run against `slidock-core/<name>.json`: both modes print the same bytes.
fn check(golden: &Golden, name: &str, passes: impl Fn() -> Vec<Pass>, presentation: &Value) {
    let printed: Vec<String> = MODES
        .iter()
        .map(|verify| {
            let result: RunResult = run(passes(), &typed(presentation.clone()), *verify);
            bytes(&result)
        })
        .collect();
    assert_eq!(printed[0], printed[1], "{name}: the verify modes differ");
    let problem = golden.record(&format!("slidock-core/{name}.json"), printed[0].as_bytes());
    assert!(problem.is_none(), "{}", problem.unwrap_or_default());
}

#[test]
fn identity_passes_resolve_the_fixtures_as_recorded() {
    let Some(golden) = Golden::open("slidock-core identity") else {
        return;
    };
    check(
        &golden,
        "identity-minimal",
        || vec![claim_all(&[]), identity("identity")],
        &fixture(&golden, "resolved", "minimal"),
    );
    check(
        &golden,
        "identity-full",
        || vec![claim_all(&[]), identity("identity")],
        &fixture(&golden, "resolved", "full"),
    );
}

#[test]
fn failures_isolate_and_report_as_recorded() {
    let Some(golden) = Golden::open("slidock-core failures") else {
        return;
    };
    let full = fixture(&golden, "resolved", "full");
    check(
        &golden,
        "liar-on-ir-s2",
        || vec![claim_all(&[Invariant::TextCascaded]), cascade_liar("ir-s2")],
        &full,
    );
    check(
        &golden,
        "thrower-on-ir-s2",
        || vec![claim_all(&[]), thrower(false)],
        &full,
    );
    let copy = copy_slide(&full, "ir-s2", "ir-s3", false);
    check(
        &golden,
        "layout-thrower",
        || vec![claim_all(&[]), thrower(true)],
        &copy,
    );
    check(
        &golden,
        "liar-on-ir-s1-with-a-copy",
        || vec![claim_all(&[Invariant::TextCascaded]), cascade_liar("ir-s1")],
        &copy,
    );
}

/// verify-source's presentation with `[code, container, node, via]` of each sorted diagnostic, and the presentation
/// alone, as canonical JSON.
fn verified(golden: &Golden, mutate: impl Fn(&mut Value)) -> (String, String) {
    let mut input = fixture(golden, "source", "full");
    mutate(&mut input);
    let verified = verify_source_presentation(&Unverified::Json(input), None);
    let order = ContainerOrder::of(verified.presentation.as_ref());
    let keys: Vec<Value> = sort_diagnostics(verified.diagnostics, &order)
        .iter()
        .map(|d| {
            json!([
                d.code,
                d.container,
                d.node,
                d.data.as_ref().and_then(|data| data.get("via"))
            ])
        })
        .collect();
    let canonical = |value: &Value| slidock_ir::canonical::to_json(value).unwrap();
    (
        canonical(&json!({ "presentation": verified.presentation, "diagnostics": keys })),
        canonical(&json!(verified.presentation)),
    )
}

/// A named change to the source fixture.
type Mutation = (&'static str, fn(&mut Value));

#[test]
fn verify_source_fails_containers_as_recorded() {
    let Some(golden) = Golden::open("slidock-core verify-source") else {
        return;
    };
    let cases: [Mutation; 4] = [
        ("clean", |_| {}),
        ("frame off grid", |p| {
            p["slides"][1]["elements"][0]["frame"]["w"] = json!(0.0001)
        }),
        ("layout and background", |p| {
            p["layouts"]["ir-layout"]["bogus"] = json!(true);
            p["slides"][0]["background"] = json!({ "kind": "nonsense" });
        }),
        ("duplicate slide id", |p| {
            p["slides"][1]["id"] = json!("ir-s1")
        }),
    ];
    for (name, mutate) in cases {
        let (printed, _) = verified(&golden, mutate);
        let problem = golden.record(
            &format!("slidock-core/verify-source-{}.json", name.replace(' ', "-")),
            printed.as_bytes(),
        );
        assert!(problem.is_none(), "{}", problem.unwrap_or_default());
    }
    // A broken colour fails its master with the first structural error only; the presentation verify-source hands on is
    // recorded.
    let (_, presentation) = verified(&golden, |p| {
        p["themes"]["ir-theme"]["colors"]["dk1"] = json!({ "kind": "rgb" })
    });
    let problem = golden.record(
        "slidock-core/verify-source-broken-colour.json",
        presentation.as_bytes(),
    );
    assert!(problem.is_none(), "{}", problem.unwrap_or_default());
}
