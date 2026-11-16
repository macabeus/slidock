//! The attended Google Slides probe (`probes/google/RUNBOOK.md`): one Google deck exported as PPTX and as ODP,
//! private, read from `$SLIDOCK_PRIVATE/probes-incoming/google/` and pinned by SHA-256 in
//! `probes/attended/google-motion-probe.expect.json`. The ODP export, lowered with native motion, must read every
//! labelled shape and transition as the expectations say, which slidock-pptx's twin test holds the PPTX export to:
//! both formats agree per shape, or the expectations explain why Google's writers differ. Skips, saying why, when
//! the private files are absent.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::path::PathBuf;

use serde_json::Value;

const FORMAT: &str = "odp";
const MOTION_PREFIX: &str = "OD94";

/// The export lowered with native motion, as JSON, and its diagnostic codes.
fn lowered(bytes: &[u8]) -> (Value, Vec<String>) {
    let lowered = support::lower_motion(bytes);
    let codes = lowered
        .diagnostics
        .iter()
        .map(|d| d.code.code().to_owned())
        .collect();
    (lowered.json.clone(), codes)
}

const EXPECT: &str = include_str!("../../../probes/attended/google-motion-probe.expect.json");

/// The private folder (`$SLIDOCK_PRIVATE`, else `~/.slidock`).
fn private() -> Option<PathBuf> {
    std::env::var_os("SLIDOCK_PRIVATE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".slidock")))
}

/// The export's bytes, checked against the SHA-256 the expectations pin; `None`, with the reason printed, when the
/// private files are absent.
fn export(expect: &Value) -> Option<Vec<u8>> {
    let file = &expect["files"][FORMAT];
    let name = file["name"].as_str().unwrap();
    let Some(path) = private().map(|dir| dir.join("probes-incoming/google").join(name)) else {
        eprintln!("skipped: neither SLIDOCK_PRIVATE nor HOME is set, so {name} cannot be found");
        return None;
    };
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!(
            "skipped: {} is absent (the attended Google Slides export, probes/google/RUNBOOK.md)",
            path.display()
        );
        return None;
    };
    assert_eq!(
        slidock_canon::sha256_hex(&bytes),
        file["sha256"].as_str().unwrap(),
        "{} is not the export the expectations pin",
        path.display()
    );
    Some(bytes)
}

/// The text of an element: its paragraphs' runs, paragraphs joined with `|`.
fn text_of(element: &Value) -> String {
    let paragraphs = element["text"]["paragraphs"].as_array();
    paragraphs
        .into_iter()
        .flatten()
        .map(|paragraph| {
            paragraph["inlines"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|inline| inline["text"].as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn elements<'v>(list: &'v Value, out: &mut Vec<&'v Value>) {
    for element in list.as_array().into_iter().flatten() {
        out.push(element);
        elements(&element["children"], out);
    }
}

/// Whether two values are the same, numbers to 1e-9 and an absent value as null.
fn same(want: &Value, got: &Value) -> bool {
    match (want, got) {
        (Value::Number(a), Value::Number(b)) => {
            (a.as_f64().unwrap() - b.as_f64().unwrap()).abs() < 1e-9
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, a)| b.get(key).is_some_and(|b| same(a, b)))
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b))
        }
        _ => want == got,
    }
}

/// `base` with the members of `over` in place of its own.
fn merged(base: &Value, over: &Value) -> Value {
    let mut out = base.clone();
    if let (Some(out), Some(over)) = (out.as_object_mut(), over.as_object()) {
        for (key, value) in over {
            out.insert(key.clone(), value.clone());
        }
    }
    out
}

/// A lowered transition or step's effect: the vocabulary effect, its params and the native id, or the opaque id and
/// the fallback.
fn effect_of(effect: &Value) -> (Value, Value, Value, Value) {
    match effect.get("native") {
        Some(native) => (
            native["effect"].clone(),
            native.get("params").cloned().unwrap_or(Value::Null),
            native["id"].clone(),
            Value::Null,
        ),
        None => (
            Value::Null,
            Value::Null,
            effect["id"].clone(),
            effect["fallback"].clone(),
        ),
    }
}

/// The fields of a step both formats read the same way (native ids and diagnostic codes are each format's own).
const STEP_FIELDS: [&str; 8] = [
    "shape",
    "kind",
    "effect",
    "params",
    "trigger",
    "delivery",
    "paragraphs",
    "durationMs",
];

/// What the comparison found.
#[derive(Default, Debug)]
struct Tally {
    transitions: usize,
    /// Transitions both formats read as the same effect (or the same fallback).
    transitions_agreeing: usize,
    /// Transitions whose durations Google's writers give differently.
    durations_differing: usize,
    steps: usize,
    steps_agreeing: usize,
    explained: Vec<String>,
    problems: Vec<String>,
    diagnostics: Vec<String>,
}

fn compare(presentation: &Value, expect: &Value) -> Tally {
    let mut tally = Tally::default();
    let slides = presentation["slides"].as_array().unwrap();
    let expected = expect["slides"].as_array().unwrap();
    if slides.len() != expected.len() {
        tally.problems.push(format!(
            "{} slides, {} expected",
            slides.len(),
            expected.len()
        ));
    }
    for (slide, want) in slides.iter().zip(expected) {
        let index = want["index"].as_u64().unwrap();
        let mut problem = |what: String| tally.problems.push(format!("slide {index}: {what}"));
        let got = &slide["transition"];
        match want["transition"].as_object() {
            None => {
                if !got.is_null() {
                    problem(format!("a transition, none expected: {got}"));
                }
            }
            Some(_) => {
                let transition = &want["transition"];
                let own = &transition[FORMAT];
                let wanted = merged(transition, own);
                tally.transitions += 1;
                let (pptx, odp) = (
                    merged(transition, &transition["pptx"]),
                    merged(transition, &transition["odp"]),
                );
                let differs = ["effect", "params", "fallback"]
                    .iter()
                    .any(|field| !same(&pptx[*field], &odp[*field]))
                    || pptx["native"].is_null() != odp["native"].is_null();
                if !same(&pptx["durationMs"], &odp["durationMs"]) {
                    tally.durations_differing += 1;
                }
                if differs || !same(&pptx["durationMs"], &odp["durationMs"]) {
                    assert!(
                        !transition["explained"].as_str().unwrap_or("").is_empty(),
                        "slide {index}: an unexplained difference"
                    );
                }
                if differs {
                    let explained = transition["explained"].as_str().unwrap_or("");
                    assert!(
                        !explained.is_empty(),
                        "slide {index}: an unexplained difference"
                    );
                    tally.explained.push(format!("slide {index} transition"));
                } else {
                    tally.transitions_agreeing += 1;
                }
                if wanted["native"].is_null() {
                    if !got.is_null() {
                        problem(format!("a transition, none expected in {FORMAT}: {got}"));
                    }
                } else {
                    let (effect, params, native, fallback) = effect_of(&got["effect"]);
                    for (field, value) in [
                        ("effect", effect),
                        ("params", params),
                        ("native", native),
                        ("durationMs", got["durationMs"].clone()),
                    ] {
                        if !same(&wanted[field], &value) {
                            problem(format!(
                                "transition {field}: {value}, expected {}",
                                wanted[field]
                            ));
                        }
                    }
                    if wanted["effect"].is_null() {
                        if !same(&wanted["fallback"], &fallback) {
                            problem(format!("transition fallback: {fallback}"));
                        }
                        tally
                            .diagnostics
                            .push(wanted["diagnostic"].as_str().unwrap().to_owned());
                    }
                }
            }
        }
        let mut shapes = Vec::new();
        elements(&slide["elements"], &mut shapes);
        let steps = slide["timeline"].as_array().cloned().unwrap_or_default();
        let wanted_steps = want["steps"].as_array().unwrap();
        if steps.len() != wanted_steps.len() {
            problem(format!(
                "{} steps, {} expected",
                steps.len(),
                wanted_steps.len()
            ));
        }
        for (i, (step, wanted)) in steps.iter().zip(wanted_steps).enumerate() {
            tally.steps += 1;
            let own = &wanted[FORMAT];
            let (pptx, odp) = (
                merged(wanted, &wanted["pptx"]),
                merged(wanted, &wanted["odp"]),
            );
            let differs = STEP_FIELDS
                .iter()
                .any(|field| !same(&pptx[*field], &odp[*field]));
            if differs {
                assert!(
                    !wanted["explained"].as_str().unwrap_or("").is_empty(),
                    "slide {index} step {i}: an unexplained difference"
                );
                tally.explained.push(format!("slide {index} step {i}"));
            } else {
                tally.steps_agreeing += 1;
            }
            let wanted = merged(wanted, own);
            let target = step["target"]["element"].as_str().unwrap();
            let shape = shapes
                .iter()
                .find(|element| element["id"] == target)
                .map_or_else(String::new, |element| text_of(element));
            let (effect, params, native, _) = effect_of(&step["effect"]);
            let paragraphs = step["target"]
                .get("paragraphs")
                .cloned()
                .unwrap_or(Value::Null);
            for (field, value) in [
                ("shape", Value::String(shape)),
                ("kind", step["kind"].clone()),
                ("effect", effect),
                ("params", params),
                ("trigger", step["trigger"].clone()),
                ("delivery", step["delivery"].clone()),
                ("paragraphs", paragraphs),
                ("durationMs", step["durationMs"].clone()),
                ("native", native),
            ] {
                let want_value = if field == "native" {
                    &wanted["native"][FORMAT]
                } else {
                    &wanted[field]
                };
                if !same(want_value, &value) {
                    problem(format!("step {i} {field}: {value}, expected {want_value}"));
                }
            }
        }
    }
    tally
}

/// The motion codes a frontend reports, sorted.
fn motion_codes(codes: impl Iterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = codes
        .filter(|code| code.starts_with(MOTION_PREFIX))
        .collect();
    out.sort();
    out
}

/// The Google export lowers, slide by slide, to what the expectations say both formats read: the same effect,
/// direction, trigger, delivery, paragraph range and duration per shape, a native effect or a declared fallback per
/// transition and step, and no other motion diagnostic. Where Google's two writers disagree, the expectations name
/// what this format reads and why.
#[test]
fn the_google_export_plays_what_both_formats_agree_on() {
    let expect: Value = serde_json::from_str(EXPECT).unwrap();
    let Some(bytes) = export(&expect) else {
        return;
    };
    let (presentation, codes) = lowered(&bytes);
    let mut tally = compare(&presentation, &expect);
    let mut wanted = std::mem::take(&mut tally.diagnostics);
    wanted.sort();
    eprintln!(
        "{FORMAT}: transitions {} ({} agree with the other format on the effect, {} differ in duration), steps {} \
         ({} agree), explained differences {:?}",
        tally.transitions,
        tally.transitions_agreeing,
        tally.durations_differing,
        tally.steps,
        tally.steps_agreeing,
        tally.explained
    );
    assert!(tally.problems.is_empty(), "{:#?}", tally.problems);
    assert_eq!(motion_codes(codes.into_iter()), wanted);
    assert_eq!((tally.transitions, tally.steps), (9, 31));
    assert_eq!(
        (
            tally.transitions_agreeing,
            tally.steps_agreeing,
            tally.explained.len()
        ),
        (4, 23, 13)
    );
}
