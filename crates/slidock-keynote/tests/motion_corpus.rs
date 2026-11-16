//! Keynote motion on the corpus (`--features golden`): every Keynote deck of the golden files, as they pin it, lowered
//! on the deck path against the expectations in `$SLIDOCK_PRIVATE/expect/keynote-motion/<presentation>.json`, which
//! the manifest pins too and which are derived from the decks' raw protobuf fields, never from this code. An
//! expectation's `deck` is the corpus presentation id. Per deck: each build and transition by effect id
//! with its vocabulary entry (or none: the fallback plays) and the IR's fallback, the steps by how they start, the
//! Magic Move pairs (each into the slide that carries it, from the shown slide before) with their matched and unmatched
//! elements, and the motion diagnostics. Skips when the golden files are absent.
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

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use slidock_core_api::Limits;
use slidock_io::FileReader;
use support::harness::{Lowered, lower_native_reader, lower_reader};

/// The expectations, which the manifest pins as inputs.
const EXPECTATIONS: &str = "expect/keynote-motion";

fn expectations(golden: &golden::Golden) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = golden
        .pinned_files(EXPECTATIONS)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| {
            let value: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            (value["deck"].as_str().unwrap().to_owned(), value)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Builds by (id, kind, by character as the deck plays them, native entry, fallback).
fn builds(lowered: &Lowered) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for slide in lowered.json["slides"].as_array().unwrap() {
        for step in slide["timeline"].as_array().into_iter().flatten() {
            let row = json!([
                step["effect"]["id"],
                step["kind"],
                step["delivery"] == "byChar",
                step["effect"]["native"]["effect"],
                step["effect"]["fallback"],
            ]);
            *out.entry(row.to_string()).or_default() += 1;
        }
    }
    out
}

fn expected_builds(expectation: &Value) -> BTreeMap<String, u64> {
    expectation["builds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            let id = format!(
                "keynote:{}",
                b["effect"].as_str().unwrap().replace(' ', "_")
            );
            let row = json!([id, b["kind"], b["byChar"], b["native"], b["fallback"]]);
            (row.to_string(), b["count"].as_u64().unwrap())
        })
        .collect()
}

/// Transitions by (id, native entry, params, easing).
fn transitions(lowered: &Lowered) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for slide in lowered.json["slides"].as_array().unwrap() {
        let transition = &slide["transition"];
        if transition.is_null() {
            continue;
        }
        let native = &transition["effect"]["native"];
        let row = json!([
            transition["effect"]["id"],
            native["effect"],
            native["params"],
            transition["easing"]
        ]);
        *out.entry(row.to_string()).or_default() += 1;
    }
    out
}

fn expected_transitions(expectation: &Value) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for t in expectation["transitions"].as_array().unwrap() {
        let id = format!(
            "keynote:{}",
            t["effect"].as_str().unwrap().replace(' ', "_")
        );
        let row = json!([id, t["native"], t["params"], t["easing"]]);
        *out.entry(row.to_string()).or_default() += t["count"].as_u64().unwrap();
    }
    out
}

/// Steps by how they start: on a click, after the step before, or with it (the same start plus its delay).
fn starts(lowered: &Lowered) -> Value {
    let (mut click, mut after, mut with) = (0, 0, 0);
    for slide in lowered.json["slides"].as_array().unwrap() {
        let mut previous: Option<(u64, u64)> = None;
        for step in slide["timeline"].as_array().into_iter().flatten() {
            let offset = step["timing"]["offsetMs"].as_u64().unwrap();
            let delay = step["delayMs"].as_u64().unwrap();
            let duration = step["durationMs"].as_u64().unwrap();
            match (step["trigger"].as_str().unwrap(), previous) {
                ("click", _) => click += 1,
                (_, Some((start, length))) if length > 0 && offset == start + delay => with += 1,
                _ => after += 1,
            }
            previous = Some((offset, duration));
        }
    }
    json!({ "click": click, "after": after, "with": with })
}

fn magic_move(lowered: &Lowered) -> Value {
    let slides = lowered.json["slides"].as_array().unwrap();
    let is_magic = |slide: &Value| slide["transition"]["effect"]["native"]["effect"] == "magicMove";
    let shown: Vec<usize> = (0..slides.len())
        .filter(|&i| slides[i]["hidden"] != json!(true))
        .collect();
    let keys = |slide: &Value| -> BTreeSet<String> {
        slide["elements"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["matchKey"].as_str().map(str::to_owned))
            .collect()
    };
    let (mut pairs, mut matched, mut unmatched, mut without) = (0, 0, 0, 0);
    for &j in &shown {
        if !is_magic(&slides[j]) {
            continue;
        }
        let Some(&i) = shown.iter().rev().find(|&&i| i < j) else {
            continue;
        };
        pairs += 1;
        let m = keys(&slides[i]).intersection(&keys(&slides[j])).count();
        let elements = |k: usize| slides[k]["elements"].as_array().unwrap().len();
        matched += m;
        unmatched += elements(i) + elements(j) - 2 * m;
        without += usize::from(m == 0);
    }
    json!({
        "transitions": slides.iter().filter(|s| is_magic(s)).count(),
        "pairs": pairs, "matched": matched, "unmatched": unmatched, "pairsWithoutMatch": without,
    })
}

/// The Magic Moves played into a slide that shares no match key with the shown slide before it but shares one with
/// the shown slide after it: a move out of the slide played into it (the motion lane's `placement` check). One
/// reported as matching nothing (KN1407, every object fades) is not counted: its slides share no content, wherever
/// it plays.
fn misplaced_magic_moves(lowered: &Lowered) -> usize {
    let unmatched: BTreeSet<&str> = lowered
        .diagnostics
        .iter()
        .filter(|d| d.code.code() == "KN1407")
        .filter_map(|d| d.container.as_ref().map(|c| c.as_str()))
        .collect();
    let slides = lowered.json["slides"].as_array().unwrap();
    let shown: Vec<&Value> = slides
        .iter()
        .filter(|s| s["hidden"] != json!(true))
        .collect();
    let keys = |slide: &Value| -> BTreeSet<String> {
        slide["elements"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["matchKey"].as_str().map(str::to_owned))
            .collect()
    };
    let shared = |a: &Value, b: &Value| keys(a).intersection(&keys(b)).count();
    (0..shown.len())
        .filter(|&j| shown[j]["transition"]["effect"]["native"]["effect"] == "magicMove")
        .filter(|&j| {
            let before = j.checked_sub(1).map_or(0, |i| shared(shown[i], shown[j]));
            let after = shown.get(j + 1).map_or(0, |next| shared(shown[j], next));
            let id = shown[j]["id"].as_str().unwrap_or_default();
            before == 0 && after > 0 && !unmatched.contains(id)
        })
        .count()
}

fn motion_diagnostics(lowered: &Lowered) -> Value {
    let mut out = serde_json::Map::new();
    for code in ["KN1404", "KN1405", "KN1406", "KN1407", "KN1408", "KN1409"] {
        let n = lowered.codes().iter().filter(|c| **c == code).count();
        out.insert(code.to_owned(), json!(n));
    }
    Value::Object(out)
}

#[test]
fn every_corpus_build_and_transition_plays_native_or_its_declared_fallback() {
    let Some(golden) = golden::Golden::open("slidock-keynote motion corpus") else {
        return;
    };
    let expectations = expectations(&golden);
    let decks: Vec<&str> = expectations.iter().map(|(deck, _)| deck.as_str()).collect();
    assert_eq!(
        decks,
        ["keynote-basic", "keynote-talk", "retro", "tables-keynote"]
    );
    let mut failures = Vec::new();
    let (mut misplaced_stored, mut misplaced_played) = (0, 0);
    for (deck, expectation) in &expectations {
        let path = golden.input(&format!("corpus:{deck}.keynote"));
        let reader = FileReader::open(&path)
            .unwrap_or_else(|e| panic!("{} is missing: {e}", path.display()));
        let lowered = lower_native_reader(&reader, None);
        // Read where Keynote stores them (the parity path), the Magic Moves morph the wrong pair; played into the
        // next shown slide, none does.
        misplaced_stored += misplaced_magic_moves(&lower_reader(&reader, None, Limits::default()));
        misplaced_played += misplaced_magic_moves(&lowered);
        let checks = [
            (
                "builds",
                json!(builds(&lowered)),
                json!(expected_builds(expectation)),
            ),
            (
                "transitions",
                json!(transitions(&lowered)),
                json!(expected_transitions(expectation)),
            ),
            ("steps", starts(&lowered), expectation["steps"].clone()),
            (
                "magicMove",
                magic_move(&lowered),
                expectation["magicMove"].clone(),
            ),
            (
                "motionDiagnostics",
                motion_diagnostics(&lowered),
                expectation["motionDiagnostics"].clone(),
            ),
        ];
        for (what, actual, expected) in checks {
            if actual != expected {
                failures.push(format!(
                    "{deck} {what}:\n  slidock  {actual}\n  expected {expected}"
                ));
            }
        }
        // Every build and transition is accounted for: a vocabulary entry, or the fallback with KN1402/KN1403.
        let unmapped_builds: u64 = builds(&lowered)
            .iter()
            .filter(|(row, _)| serde_json::from_str::<Value>(row).unwrap()[3].is_null())
            .map(|(_, n)| n)
            .sum();
        let kn1402 = lowered.codes().iter().filter(|c| **c == "KN1402").count() as u64;
        if unmapped_builds > kn1402 {
            failures.push(format!(
                "{deck}: {unmapped_builds} builds play their fallback, {kn1402} reported"
            ));
        }
        eprintln!(
            "{deck}: {} steps, {} transitions, magic move {}",
            builds(&lowered).values().sum::<u64>(),
            transitions(&lowered).values().sum::<u64>(),
            magic_move(&lowered)
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!((misplaced_stored, misplaced_played), (25, 0));
}
