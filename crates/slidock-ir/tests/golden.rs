//! The golden files (`--features golden`, tests/golden; skips when they are absent): every private IR fixture and every
//! resolved IR slidock recorded parses and re-prints byte for byte, the reports' diagnostics carry the registry's
//! severities and messages, and the fixtures read `null` and float spellings as the schema says.
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
mod support;

use std::path::Path;

use golden::Golden;
use serde::Serialize;
use serde::de::DeserializeOwned;
use slidock_ir::canonical::to_json;
use slidock_ir::canonical::to_json_pretty;
use slidock_ir::diagnostics::{Diagnostic, render_message};
use slidock_ir::verify::parse_presentation_json;
use slidock_ir::{parse, resolved, source};

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The private IR fixtures, source then resolved: `(path, phase)`.
fn fixtures(golden: &Golden) -> Vec<(std::path::PathBuf, &'static str)> {
    let mut out = Vec::new();
    for name in golden.ir_fixtures() {
        out.push((
            golden.fixture(&format!("ir/{name}.presentation-ir.json")),
            "source",
        ));
    }
    for phase in ["source", "resolved"] {
        for name in golden.fixture_names(&format!("presentation/{phase}")) {
            out.push((
                golden.fixture(&format!("presentation/{phase}/{name}")),
                phase,
            ));
        }
    }
    out
}

/// The resolved IR every recorded compile wrote, by output path.
fn recorded_irs(golden: &Golden) -> Vec<String> {
    let mut paths: Vec<String> = golden
        .ir_fixtures()
        .iter()
        .map(|name| format!("fixtures/{name}/presentation-ir.json"))
        .collect();
    paths.extend(
        golden
            .pairs()
            .iter()
            .map(|pair| format!("corpus/{}/presentation-ir.json", pair.name)),
    );
    paths
}

/// Parses `text` as `T` and re-prints it canonically, failing with the path of a structural error.
fn round_trip<T: DeserializeOwned + Serialize>(path: &Path, text: &str) -> T {
    let value: T = parse::from_str(text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let printed = to_json_pretty(&value).unwrap();
    if printed != text {
        let line = printed
            .lines()
            .zip(text.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        panic!(
            "{} does not re-print byte for byte; first difference at line {}:\n  printed: {:?}\n  golden:  {:?}",
            path.display(),
            line + 1,
            printed.lines().nth(line),
            text.lines().nth(line)
        );
    }
    value
}

/// Each diagnostic's severity and message are the registry's.
fn check_registry(path: &Path, diagnostics: &[Diagnostic]) {
    for d in diagnostics {
        assert_eq!(
            d.severity,
            d.code.severity(),
            "{}: {}",
            path.display(),
            d.code
        );
        let data = d.data.clone().unwrap_or_default();
        assert_eq!(
            render_message(d.code, &data),
            d.message,
            "{}: {}",
            path.display(),
            d.code
        );
    }
}

#[test]
fn every_golden_ir_round_trips_byte_for_byte() {
    let Some(golden) = Golden::open("slidock-ir round trips") else {
        return;
    };
    let mut count = 0;
    for (path, phase) in fixtures(&golden) {
        let text = read(&path);
        let presentation: source::Presentation = round_trip(&path, &text);
        assert_eq!(presentation.phase.as_str(), phase, "{}", path.display());
        if phase == "resolved" {
            round_trip::<resolved::Presentation>(&path, &text);
        }
        count += 1;
    }
    for output in recorded_irs(&golden) {
        let path = golden.root().join(&output);
        let text = golden.output_text(&output);
        round_trip::<resolved::Presentation>(&path, &text);
        round_trip::<source::Presentation>(&path, &text);
        let report_path = output.replace("presentation-ir.json", "report.json");
        let report: serde_json::Value =
            serde_json::from_slice(&golden.output(&report_path)).unwrap();
        let diagnostics: Vec<Diagnostic> =
            parse::from_value(report["diagnostics"].clone()).unwrap();
        check_registry(&golden.root().join(&report_path), &diagnostics);
        count += 1;
    }
    eprintln!("golden IRs round-tripped: {count}");
    assert_eq!(count, fixtures(&golden).len() + recorded_irs(&golden).len());
    assert!(count >= 28, "{count}");
}

/// The IR fixtures' members set to `null` are refused unless the schema takes `null` there, and their integers read the
/// same written as integral floats.
#[test]
fn ir_fixtures_read_null_and_float_spellings_as_the_schema_says() {
    let Some(golden) = Golden::open("slidock-ir spellings") else {
        return;
    };
    let mut count = 0;
    {
        for (path, _) in fixtures(&golden) {
            let value: serde_json::Value = serde_json::from_str(&read(&path)).unwrap();
            let resolved = value["phase"] == "resolved";
            for (at, input, accepted) in support::nulled(&value) {
                let at = support::show(&at);
                assert_eq!(
                    parse_presentation_json(&input).is_ok(),
                    accepted,
                    "{}: null at {at}",
                    path.display()
                );
                if resolved {
                    assert_eq!(
                        parse::from_value::<resolved::Presentation>(input).is_ok(),
                        accepted,
                        "{}: null at {at}, resolved schema",
                        path.display()
                    );
                }
            }
            let expected = to_json(&parse_presentation_json(&value).unwrap()).unwrap();
            for (at, input) in support::floated(&value) {
                let parsed = parse_presentation_json(&input).unwrap_or_else(|_| {
                    panic!("{}: {} as a float", path.display(), support::show(&at))
                });
                assert_eq!(
                    to_json(&parsed).unwrap(),
                    expected,
                    "{}",
                    support::show(&at)
                );
            }
            count += 1;
        }
    }
    eprintln!("IR fixtures swept: {count}");
    assert_eq!(count, 10);
}
