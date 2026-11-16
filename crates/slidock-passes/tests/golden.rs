//! The golden files (`--features golden`, tests/golden; skips when they are absent): every private IR fixture, lowered
//! as the IR frontend lowers it and run through every pass by slidock-core's `PassManager`, gives the resolved IR and
//! diagnostics slidock recorded (`slidock-passes/resolved/<name>.json`) byte for byte, in both verify modes; its
//! resolved IR is the one `slidock compile` wrote, its pass diagnostics are the report's, every media file the compile
//! wrote is planned so, and a second run, in reverse order under a clock that never reads the same twice, gives the
//! same bytes.
#![cfg(feature = "golden")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde::Serialize;
use serde_json::Value;
use slidock_core::{CompileResult, VerifyMode};
use slidock_ir::diagnostics::{Diagnostic, Severity, Stage};
use slidock_ir::resolved;
use support::fixtures::{compile_fixture, compile_fixture_clocked, golden, ir_fixture_names};

/// What the golden holds: the resolved presentation and the sorted diagnostics.
#[derive(Serialize)]
struct Resolved<'a> {
    presentation: Option<&'a resolved::Presentation>,
    diagnostics: &'a [Diagnostic],
}

fn bytes(result: &CompileResult) -> String {
    let resolved = Resolved {
        presentation: result.presentation.as_ref(),
        diagnostics: &result.diagnostics,
    };
    slidock_ir::canonical::to_json_pretty(&resolved).unwrap()
}

/// The resolved presentation as `slidock compile --emit ir` dumps it: with the IR's `$schema`.
fn dump(result: &CompileResult) -> String {
    slidock_ir::canonical::dump_presentation(result.presentation.as_ref().unwrap()).unwrap()
}

/// Each diagnostic as canonical JSON, sorted: the multiset a list holds.
fn diagnostic_set<'a>(diagnostics: impl IntoIterator<Item = &'a Value>) -> Vec<String> {
    let mut set: Vec<String> = diagnostics
        .into_iter()
        .map(|d| slidock_canon::to_canonical_json(d, slidock_canon::Layout::Compact).unwrap())
        .collect();
    set.sort();
    set
}

#[test]
fn every_ir_fixture_resolves_as_recorded_byte_for_byte() {
    let Some(golden) = golden() else {
        return;
    };
    let names = ir_fixture_names();
    let mut failures = Vec::new();
    for name in &names {
        let result = compile_fixture(name, VerifyMode::Each);
        let path = format!("slidock-passes/resolved/{name}.json");
        failures.extend(golden.record(&path, bytes(&result).as_bytes()));
        // The CLI's compile of the same fixture: its resolved IR, and its report's diagnostics but the backend's.
        let ir = format!("fixtures/{name}/presentation-ir.json");
        failures.extend(golden.compare(&ir, dump(&result).as_bytes()));
        let report: Value =
            serde_json::from_slice(&golden.output(&format!("fixtures/{name}/report.json")))
                .unwrap();
        let reported: Vec<&Value> = report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["stage"] != "backend")
            .collect();
        let ours: Vec<Value> = result
            .diagnostics
            .iter()
            .map(|d| serde_json::to_value(d).unwrap())
            .collect();
        if diagnostic_set(reported) != diagnostic_set(&ours) {
            failures.push(format!("{name}: the pass diagnostics are not the report's"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    assert_eq!(names.len(), 6, "the private fixtures hold six IR fixtures");
}

#[test]
fn every_ir_fixture_resolves_with_no_error_and_alike_in_both_verify_modes() {
    if golden().is_none() {
        return;
    }
    for name in ir_fixture_names() {
        let each = compile_fixture(&name, VerifyMode::Each);
        let errors: Vec<&Diagnostic> = each
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(errors.is_empty(), "{name}: {errors:?}");
        assert!(each.presentation.is_some(), "{name}");
        assert_eq!(
            bytes(&compile_fixture(&name, VerifyMode::Boundaries)),
            bytes(&each),
            "{name}"
        );
    }
}

#[test]
fn is_deterministic_a_second_run_in_reverse_order_under_a_moving_clock_gives_the_same_bytes() {
    if golden().is_none() {
        return;
    }
    let names = ir_fixture_names();
    let first: Vec<String> = names
        .iter()
        .map(|name| bytes(&compile_fixture(name, VerifyMode::Each)))
        .collect();
    let now = std::cell::Cell::new(0.0);
    let clock = || {
        now.set(now.get() + 1.0);
        now.get()
    };
    for (name, expected) in names.iter().zip(&first).rev() {
        let again = compile_fixture_clocked(name, VerifyMode::Each, Some(&clock));
        assert_eq!(&bytes(&again), expected, "{name}");
    }
    assert!(now.get() > 0.0, "the compiler read the clock");
}

/// Every medium the compile wrote is planned so: a copy under its own hash and type, a transcode as a PNG named by its
/// source's hash; a planned item it does not print (a video with no poster) writes no file.
#[test]
fn media_plans_give_the_files_the_compile_wrote() {
    let Some(golden) = golden() else {
        return;
    };
    for name in ir_fixture_names() {
        let result = compile_fixture(&name, VerifyMode::Each);
        let presentation = serde_json::to_value(result.presentation.as_ref().unwrap()).unwrap();
        let planned: Vec<(String, String, String)> = presentation["media"]
            .as_object()
            .unwrap()
            .values()
            .filter(|entry| entry["plan"]["action"] != "omit")
            .map(|entry| {
                let plan = &entry["plan"];
                (
                    plan["output"]["file"].as_str().unwrap().to_owned(),
                    plan["action"].as_str().unwrap().to_owned(),
                    entry["sha256"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        let written = golden
            .outputs_under(&format!("fixtures/{name}"))
            .into_iter()
            .filter(|(path, _)| path.starts_with("media/"));
        let mut count = 0;
        for (path, bytes) in written {
            let file = path.strip_prefix("media/").unwrap();
            let (_, action, sha256) = planned
                .iter()
                .find(|(planned_file, _, _)| planned_file.ends_with(file))
                .unwrap_or_else(|| panic!("{name}: {file} is not planned"));
            assert!(file.starts_with(sha256.as_str()), "{name}: {file}");
            if action == "copy" {
                assert_eq!(&slidock_canon::sha256_hex(&bytes), sha256, "{name}: {file}");
            } else {
                assert!(file.ends_with(".png"), "{name}: {file} is a transcode");
            }
            count += 1;
        }
        assert!(count > 0 || name != "media-timeline", "{name}");
        assert!(
            result.diagnostics.iter().all(|d| d.stage != Stage::Backend),
            "{name}"
        );
    }
}
