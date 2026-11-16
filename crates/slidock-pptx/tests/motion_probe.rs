//! The motion probe deck (`probes/out/motion-probe.pptx`) against its expectations (`motion-probe.expect.json`,
//! format `slidock-motion-probe-expect@1`): every slide's transition and steps as native motion lowers them. Each
//! expected transition and step is matched (vocabulary effect, params, trigger, timing…), plays its fallback with
//! the expected diagnostic (dropped steps included), or is a mismatch; the test fails on any mismatch.
//!
//! A transition's IR `fallback` keeps the parity path's rule (the PPTX frontend's own fallbacks), so an expectation's
//! `fallback` is compared with the vocabulary's canonical fallback of the effect it names.
//!
//! Where the expectations and the shared contract (the IR's vocabulary, the preset table) disagree, the frontend
//! follows the contract; [`DISAGREEMENTS`] lists each case and its reason, and the test fails on any other
//! mismatch and on a listed case that no longer occurs. The table's `approximate` flag is not part of the IR: its
//! differences from the expectations are printed, and fail the test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json};
use slidock_ir::formats::IdPrefix;
use slidock_ir::ids::{IdPart, node_id};
use slidock_ir::source::Slide;

fn probes() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../probes/out")
}

/// The duration a step plays when its own does not read (the frontend's declared default).
const DEFAULT_STEP_DURATION: u32 = 500;

const MOTION_CODES: [&str; 6] = ["PX2402", "PX2403", "PX2410", "PX2411", "PX2412", "PX2413"];

/// The expected effects the frontend plays otherwise because the shared contract says so, with the fields that
/// differ and why.
const DISAGREEMENTS: &[(&[&str], &[&str], &str)] = &[];

/// The disagreement a problem on `native`'s `field` is, if any.
fn listed(native: &str, field: &str) -> Option<usize> {
    DISAGREEMENTS
        .iter()
        .position(|(natives, fields, _)| natives.contains(&native) && fields.contains(&field))
}

/// What a comparison found.
#[derive(Default, Debug)]
struct Tally {
    slides: usize,
    transitions: usize,
    steps: usize,
    matched: usize,
    fallback_with_diagnostic: usize,
    /// Where the preset table's `approximate` flag (not part of the IR) disagrees with the expectations.
    table_flags: Vec<String>,
    /// Steps whose every difference [`DISAGREEMENTS`] explains.
    disagreeing_steps: usize,
    /// Problems [`DISAGREEMENTS`] explains, per entry.
    disagreements: BTreeMap<usize, usize>,
    mismatched: Vec<String>,
}

/// A JSON number as the expectations print it (`128` for 128.0).
fn number(value: &Value) -> String {
    let x = value.as_f64().unwrap();
    if x.fract() == 0.0 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

/// A motion path as `M 0 0 L 128 0 …`.
fn path_text(d: &Value) -> String {
    d.as_array()
        .unwrap()
        .iter()
        .flat_map(|segment| {
            let segment = segment.as_array().unwrap();
            let mut words = vec![segment[0].as_str().unwrap().to_owned()];
            words.extend(segment[1..].iter().map(number));
            words
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// An expectation token in the IR's spelling: orientations and axes are `horz`/`vert` there.
fn token(text: &str) -> String {
    match text {
        "horizontal" => "horz".to_owned(),
        "vertical" => "vert".to_owned(),
        _ if text.starts_with('#') => text.to_ascii_lowercase(),
        _ => text.to_owned(),
    }
}

/// Whether an actual value equals an expected one: numbers to 1e-9, tokens in the IR's spelling, arrays and
/// objects member by member.
fn same(want: &Value, got: &Value) -> bool {
    match (want, got) {
        (Value::Number(_), Value::Number(_)) => {
            (want.as_f64().unwrap() - got.as_f64().unwrap()).abs() < 1e-9
        }
        (Value::String(a), Value::String(b)) => token(a) == *b,
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, a)| b.get(key).is_some_and(|b| same(a, b)))
        }
        _ => want == got,
    }
}

/// Whether the actual params hold every expected one, in the expectations' value forms.
fn params_hold(expected: &Value, actual: &Value) -> Result<(), String> {
    let Some(expected) = expected.as_object() else {
        return Ok(());
    };
    for (key, want) in expected {
        let got = actual.get(key).unwrap_or(&Value::Null);
        let ok = match key.as_str() {
            "scale" => same(&want[0], &got["x"]) && same(&want[1], &got["y"]),
            "d" => want.as_str() == Some(path_text(got).as_str()),
            _ => same(want, got),
        };
        if !ok {
            return Err(format!("params.{key}: expected {want}, got {got}"));
        }
    }
    Ok(())
}

/// The motion diagnostics on a slide, as JSON.
fn slide_diagnostics(lowered: &support::Lowered, slide: &Slide) -> Vec<Value> {
    lowered
        .diagnostics
        .iter()
        .filter(|d| d.container.as_ref() == Some(&slide.id))
        .filter(|d| MOTION_CODES.contains(&d.code.code()))
        .map(|d| serde_json::to_value(d).unwrap())
        .collect()
}

fn data<'v>(diagnostic: &'v Value, key: &str) -> &'v Value {
    &diagnostic["data"][key]
}

/// Takes the first diagnostic with `code` (and the given data) out of `pool`.
fn take(
    pool: &mut Vec<Value>,
    code: &str,
    effect: &str,
    spid: Option<&str>,
    what: Option<&str>,
) -> Option<Value> {
    let at = pool.iter().position(|d| {
        d["code"] == code
            && data(d, "effect") == effect
            && spid.is_none_or(|spid| data(d, "spid") == spid)
            && what.is_none_or(|what| data(d, "what") == what)
    })?;
    Some(pool.remove(at))
}

/// The expected native id's preset row approximates the effect.
fn step_approximate(native: &str) -> bool {
    let name = native.strip_prefix("pptx:").unwrap();
    let (class, rest) = name.split_once('.').unwrap();
    let (id, subtype) = rest.split_once('/').unwrap_or((rest, "0"));
    slidock_presets::pptx::build(class, id.parse().unwrap(), subtype.parse().unwrap())
        .is_some_and(|lookup| lookup.row.approximate)
}

fn transition_row(native: &str) -> Option<slidock_presets::TransitionRow> {
    let name = native.strip_prefix("pptx:").unwrap();
    let element = name.split('/').next().unwrap();
    let prst = name.split_once('/').map(|(_, prst)| prst);
    slidock_presets::pptx::transition(element, |attr| (attr == "prst").then_some(prst).flatten())
        .map(|lookup| lookup.row)
}

fn compare_transition(
    expected: &Value,
    slide: &Slide,
    pool: &mut Vec<Value>,
    tally: &mut Tally,
) -> Vec<String> {
    let mut problems = Vec::new();
    let Some(actual) = &slide.transition else {
        if !expected.is_null() {
            problems.push("transition: missing".to_owned());
        }
        return problems;
    };
    if expected.is_null() {
        problems.push("transition: unexpected".to_owned());
        return problems;
    }
    tally.transitions += 1;
    let actual = serde_json::to_value(actual).unwrap();
    let native = expected["native"].as_str().unwrap();
    let got_native = actual["effect"]["native"]["id"]
        .as_str()
        .or(actual["effect"]["id"].as_str())
        .unwrap();
    if got_native != native {
        problems.push(format!(
            "transition native: expected {native}, got {got_native}"
        ));
    }
    let got_effect = &actual["effect"]["native"]["effect"];
    if &expected["effect"] != got_effect {
        problems.push(format!(
            "transition effect: expected {}, got {got_effect}",
            expected["effect"]
        ));
    } else if let Err(problem) =
        params_hold(&expected["params"], &actual["effect"]["native"]["params"])
    {
        problems.push(format!("transition {problem}"));
    }
    let row = transition_row(native);
    let approximate = row.is_some_and(|row| row.approximate);
    if expected["approximate"] != json!(approximate) {
        tally
            .table_flags
            .push(format!("{native}: approximate {}", expected["approximate"]));
    }
    let canonical = match row.and_then(|row| row.effect) {
        Some(_) => serde_json::to_value(row.unwrap().canonical_fallback).unwrap(),
        None => actual["effect"]["fallback"].clone(),
    };
    if expected["fallback"] != canonical {
        problems.push(format!(
            "transition fallback: expected {}, got {canonical}",
            expected["fallback"]
        ));
    }
    if expected["durationMs"] != actual["durationMs"] {
        problems.push(format!(
            "transition durationMs: expected {}, got {}",
            expected["durationMs"], actual["durationMs"]
        ));
    }
    let advance = &actual["advance"];
    let want = &expected["advance"];
    if want["onClick"] != advance["onClick"]
        || want["afterMs"] != *advance.get("afterMs").unwrap_or(&Value::Null)
    {
        problems.push(format!(
            "transition advance: expected {want}, got {advance}"
        ));
    }
    let name = native.strip_prefix("pptx:").unwrap();
    if let Some(code) = expected["diagnostic"]["code"].as_str()
        && (code != "PX2411" || expected["hostileTiming"].is_null())
        && take(pool, code, name, None, None).is_none()
    {
        problems.push(format!("transition diagnostic {code}: missing"));
    }
    if let Some(fields) = expected["hostileTiming"].as_object() {
        for (field, want) in fields {
            match take(pool, "PX2411", name, None, Some(field)) {
                Some(found)
                    if data(&found, "native") == &want["native"]
                        && data(&found, "read") == &want["read"] => {}
                found => problems.push(format!(
                    "transition hostile {field}: expected {want}, got {found:?}"
                )),
            }
        }
    }
    if problems.is_empty() {
        if expected["effect"].is_null() {
            tally.fallback_with_diagnostic += 1;
        } else {
            tally.matched += 1;
        }
    }
    problems
}

/// Whether the tier-2 tracks play the expected clip.
fn clip_holds(clip: &Value, tracks: &Value) -> bool {
    let tracks = tracks.as_array().unwrap();
    let effect = clip["effect"].as_str().unwrap();
    tracks.iter().any(
        |track| match (track["property"].as_str().unwrap(), effect) {
            ("opacity", "fade") => true,
            ("clipPath", _) => {
                track["reveal"]["effect"] == effect
                    && params_hold(&clip["params"], &track["reveal"]["params"]).is_ok()
            }
            ("translate", "flyIn") => {
                let frames = track["frames"].as_array().unwrap();
                let (first, last) = (&frames[0]["value"], &frames[frames.len() - 1]["value"]);
                let dx = last["x"].as_f64().unwrap() - first["x"].as_f64().unwrap();
                let dy = last["y"].as_f64().unwrap() - first["y"].as_f64().unwrap();
                let moves = match clip["params"]["dir"].as_str().unwrap() {
                    "l" => dx < 0.0 && dy == 0.0,
                    "r" => dx > 0.0 && dy == 0.0,
                    "u" => dy < 0.0 && dx == 0.0,
                    _ => dy > 0.0 && dx == 0.0,
                };
                moves && clip["params"]["distance"] == "slight"
            }
            _ => false,
        },
    )
}

#[allow(clippy::too_many_lines, reason = "one field after another")]
fn compare_step(
    expected: &Value,
    actual: &Value,
    slide: &Slide,
    pool: &mut Vec<Value>,
    tally: &mut Tally,
) -> Vec<String> {
    let mut problems = Vec::new();
    let native = expected["native"].as_str().unwrap();
    let mut check = |field: &str, want: &Value, got: &Value| {
        if !same(want, got) {
            problems.push(format!("{field}: expected {want}, got {got}"));
        }
    };
    let spid = expected["target"]["spid"].as_u64().unwrap().to_string();
    let element = node_id(
        IdPrefix::Pp,
        &[IdPart::Str(slide.id.body()), IdPart::Str(&spid)],
    )
    .unwrap();
    check(
        "target",
        &json!(element.as_str()),
        &actual["target"]["element"],
    );
    check(
        "paragraphs",
        &expected["paragraphs"],
        actual["target"].get("paragraphs").unwrap_or(&Value::Null),
    );
    check(
        "background",
        &expected["background"],
        &json!(actual["target"]["background"] == true),
    );
    check("trigger", &expected["trigger"], &actual["trigger"]);
    check("kind", &expected["kind"], &actual["kind"]);
    let got_native = actual["effect"]["native"]["id"]
        .as_str()
        .or(actual["effect"]["id"].as_str())
        .unwrap();
    check("native", &json!(native), &json!(got_native));
    let got_effect = &actual["effect"]["native"]["effect"];
    check("effect", &expected["effect"], got_effect);
    if expected["effect"] == *got_effect {
        if expected["effect"] != "behaviors" {
            if let Err(problem) =
                params_hold(&expected["params"], &actual["effect"]["native"]["params"])
            {
                problems.push(problem);
            }
        } else if !expected["clip"].is_null()
            && !clip_holds(&expected["clip"], &actual["effect"]["native"]["params"])
        {
            problems.push(format!(
                "clip: expected {}, got {}",
                expected["clip"], actual["effect"]["native"]["params"]
            ));
        }
    }
    let mut check = |field: &str, want: &Value, got: &Value| {
        if !same(want, got) {
            problems.push(format!("{field}: expected {want}, got {got}"));
        }
    };
    let tier = match got_effect.as_str() {
        None => Value::Null,
        Some("behaviors") => json!(2),
        Some(_) => json!(1),
    };
    check("tier", &expected["tier"], &tier);
    if tier == json!(1) && expected["approximate"] != json!(step_approximate(native)) {
        tally
            .table_flags
            .push(format!("{native}: approximate {}", expected["approximate"]));
    }
    check(
        "fallback",
        &expected["fallback"],
        &actual["effect"]["fallback"],
    );
    check("delivery", &expected["delivery"], &actual["delivery"]);
    let timing = actual.get("timing").cloned().unwrap_or(json!({}));
    let stagger = match timing.get("stagger") {
        None => Value::Null,
        Some(stagger) => {
            let mut out = json!({ "order": stagger["order"] });
            if let Some(ms) = stagger["perUnit"].get("ms") {
                out["perUnitMs"] = ms.clone();
            } else {
                out["perUnitPct"] = json!(stagger["perUnit"]["fraction"].as_f64().unwrap() * 100.0);
            }
            out
        }
    };
    check("stagger", &expected["stagger"], &stagger);
    check(
        "offsetMs",
        &expected["offsetMs"],
        timing.get("offsetMs").unwrap_or(&Value::Null),
    );
    check("delayMs", &expected["delayMs"], &actual["delayMs"]);
    let duration = if expected["durationMs"].is_null() {
        json!(DEFAULT_STEP_DURATION)
    } else {
        expected["durationMs"].clone()
    };
    check("durationMs", &duration, &actual["durationMs"]);
    let easing = match timing.get("easing") {
        Some(easing) => easing["accelDecel"].clone(),
        None => Value::Null,
    };
    check("easing", &expected["easing"], &easing);
    check(
        "repeat",
        &expected["repeat"],
        timing.get("repeat").unwrap_or(&Value::Null),
    );
    check(
        "autoReverse",
        &expected["autoReverse"],
        &json!(timing["autoReverse"] == true),
    );
    check(
        "after",
        &expected["after"],
        timing.get("after").unwrap_or(&json!("keep")),
    );
    if let Some(code) = expected["diagnostic"]["code"].as_str()
        && (code != "PX2411" || expected["hostileTiming"].is_null())
        && take(pool, code, native, Some(&spid), None).is_none()
    {
        problems.push(format!("diagnostic {code}: missing"));
    }
    if let Some(fields) = expected["hostileTiming"].as_object() {
        for (field, want) in fields {
            match take(pool, "PX2411", native, Some(&spid), Some(field)) {
                Some(found) => {
                    if data(&found, "native") != &want["native"]
                        || data(&found, "read") != &want["read"]
                        || data(&found, "limit") != &want["limit"]
                    {
                        problems.push(format!(
                            "hostile {field}: expected {want}, got {}",
                            found["data"]
                        ));
                    }
                }
                None => problems.push(format!("hostile {field}: no PX2411")),
            }
        }
    }
    if problems.is_empty() {
        if expected["effect"].is_null() {
            tally.fallback_with_diagnostic += 1;
        } else {
            tally.matched += 1;
        }
    }
    problems
}

/// The expected click groups that keep a step: a group whose steps are all dropped leaves no click. Interactive
/// steps are outside the main sequence.
fn surviving_groups(steps: &[Value]) -> usize {
    let mut groups: Vec<bool> = Vec::new();
    for step in steps {
        if step["trigger"] == "onShapeClick" {
            continue;
        }
        if step["trigger"] == "click" {
            groups.push(false);
        }
        if step["dropped"] == false
            && let Some(last) = groups.last_mut()
        {
            *last = true;
        }
    }
    groups.into_iter().filter(|kept| *kept).count()
}

fn run() -> Tally {
    let bytes = std::fs::read(probes().join("motion-probe.pptx")).unwrap();
    let expect: Value =
        serde_json::from_slice(&std::fs::read(probes().join("motion-probe.expect.json")).unwrap())
            .unwrap();
    assert_eq!(expect["format"], "slidock-motion-probe-expect@1");
    let deck = &expect["decks"]["motion-probe.pptx"];
    let lowered = support::lower_native(&bytes);
    let presentation = lowered.presentation();
    let mut tally = Tally::default();
    let slides = deck["slideList"].as_array().unwrap();
    assert_eq!(deck["slides"], json!(slides.len()));
    assert_eq!(presentation.slides.len(), slides.len());
    for (expected, slide) in slides.iter().zip(&presentation.slides) {
        tally.slides += 1;
        let key = expected["key"].as_str().unwrap();
        let mut pool = slide_diagnostics(&lowered, slide);
        let mut problems =
            compare_transition(&expected["transition"], slide, &mut pool, &mut tally);
        let steps = expected["steps"].as_array().unwrap();
        let mut actual = slide
            .timeline
            .iter()
            .map(|step| serde_json::to_value(step).unwrap());
        for (n, step) in steps.iter().enumerate() {
            tally.steps += 1;
            if step["dropped"] == true {
                let native = step["native"].as_str().unwrap();
                let spid = step["target"]["spid"].as_u64().unwrap().to_string();
                let code = step["diagnostic"]["code"].as_str().unwrap();
                match take(&mut pool, code, native, Some(&spid), None) {
                    Some(_) => tally.fallback_with_diagnostic += 1,
                    None => problems.push(format!("step {n} ({native}): dropped without {code}")),
                }
                continue;
            }
            match actual.next() {
                Some(got) => {
                    let native = step["native"].as_str().unwrap();
                    let found = compare_step(step, &got, slide, &mut pool, &mut tally);
                    let mut explained = !found.is_empty();
                    for problem in found {
                        let field = problem.split(':').next().unwrap_or("");
                        match listed(native, field) {
                            Some(entry) => *tally.disagreements.entry(entry).or_default() += 1,
                            None => {
                                explained = false;
                                problems.push(format!("step {n} ({native}): {problem}"));
                            }
                        }
                    }
                    if explained {
                        tally.disagreeing_steps += 1;
                    }
                }
                None => problems.push(format!("step {n}: missing")),
            }
        }
        if actual.next().is_some() {
            problems.push("more steps than expected".to_owned());
        }
        let clicks = slide
            .timeline
            .iter()
            .filter(|s| s.trigger == slidock_ir::timeline::Trigger::Click)
            .count();
        let groups = surviving_groups(steps);
        if clicks != groups {
            problems.push(format!("click groups: expected {groups}, got {clicks}"));
        }
        if let Some(first) = slide.timeline.first() {
            let auto = first.trigger != slidock_ir::timeline::Trigger::Click;
            if json!(auto) != expected["autoGroup"] {
                problems.push(format!("autoGroup: expected {}", expected["autoGroup"]));
            }
        }
        for extra in pool {
            let code = extra["code"].as_str().unwrap_or("");
            let effect = data(&extra, "effect").as_str().unwrap_or("");
            match listed(effect, &format!("diagnostic {code}")) {
                Some(entry) => *tally.disagreements.entry(entry).or_default() += 1,
                None => problems.push(format!("unexpected diagnostic {code} on {effect}")),
            }
        }
        tally.mismatched.extend(
            problems
                .into_iter()
                .map(|problem| format!("{key}: {problem}")),
        );
    }
    tally
}

#[test]
fn every_pptx_slide_meets_its_expectations() {
    let tally = run();
    let summary: BTreeMap<&str, usize> = [
        ("slides", tally.slides),
        ("transitions", tally.transitions),
        ("steps", tally.steps),
        ("matched", tally.matched),
        ("fallbackWithDiagnostic", tally.fallback_with_diagnostic),
        ("tableApproximateFlags", tally.table_flags.len()),
        ("mismatched", tally.mismatched.len()),
        ("contractDisagreeingSteps", tally.disagreeing_steps),
    ]
    .into_iter()
    .collect();
    println!("{summary:?}");
    for problem in &tally.mismatched {
        println!("MISMATCH {problem}");
    }
    for flag in &tally.table_flags {
        println!("TABLE {flag}");
    }
    for (entry, count) in &tally.disagreements {
        println!("CONTRACT {count} x {}", DISAGREEMENTS[*entry].2);
    }
    assert!(
        tally.mismatched.is_empty(),
        "{} mismatches",
        tally.mismatched.len()
    );
    assert!(tally.table_flags.is_empty(), "{:?}", tally.table_flags);
    let stale: Vec<&str> = (0..DISAGREEMENTS.len())
        .filter(|entry| !tally.disagreements.contains_key(entry))
        .map(|entry| DISAGREEMENTS[entry].2)
        .collect();
    assert!(
        stale.is_empty(),
        "listed disagreements that no longer occur: {stale:?}"
    );
    assert_eq!(tally.slides, 211);
    assert_eq!(tally.transitions, 141);
    assert_eq!(tally.steps, 344);
}

/// Every step and transition that plays its fallback is reported once by the frontend (PX2402 for a transition,
/// PX2410 for a step) and once by the deck path (HTML6010), and every approximation that plays once (PX2413).
#[test]
fn every_fallback_and_approximation_is_reported_once() {
    use slidock_ir::diagnostics::Diagnostic;
    let bytes = std::fs::read(probes().join("motion-probe.pptx")).unwrap();
    let compiled = support::compile_motion(&bytes, "motion-probe.pptx", true);
    let presentation = compiled.presentation.as_ref().unwrap();
    let key = |d: &Diagnostic| {
        let effect = d
            .data
            .as_ref()
            .and_then(|data| data.get("effect"))
            .map(|effect| effect.to_js_string())
            .unwrap_or_default();
        (
            d.container.as_ref().map(ToString::to_string),
            d.node.as_ref().map(ToString::to_string),
            effect,
        )
    };
    let count = |diagnostics: &[Diagnostic], code: &str| {
        let mut out: BTreeMap<_, usize> = BTreeMap::new();
        for d in diagnostics.iter().filter(|d| d.code.code() == code) {
            *out.entry(key(d)).or_default() += 1;
        }
        out
    };
    let mut fallback_steps = BTreeMap::new();
    let mut fallback_transitions = BTreeMap::new();
    let mut approximated = BTreeMap::new();
    for slide in &presentation.slides {
        let container = Some(slide.id.to_string());
        for step in &slide.timeline {
            let node = Some(step.target.element.to_string());
            match &step.effect.native {
                None => {
                    fallback_steps.insert((container.clone(), node, step.effect.id.to_string()), 1);
                }
                Some(native)
                    if native.effect.name() != "behaviors"
                        && step_approximate(&native.id.to_string()) =>
                {
                    approximated.insert((container.clone(), node, native.id.to_string()), 1);
                }
                Some(_) => {}
            }
        }
        if let Some(transition) = &slide.transition {
            match &transition.effect.native {
                None => {
                    fallback_transitions.insert(
                        (container.clone(), None, transition.effect.id.to_string()),
                        1,
                    );
                }
                Some(native)
                    if transition_row(&native.id.to_string())
                        .is_some_and(|row| row.approximate) =>
                {
                    approximated.insert((container.clone(), None, native.id.to_string()), 1);
                }
                Some(_) => {}
            }
        }
    }
    let ids = |map: &BTreeMap<(Option<String>, Option<String>, String), usize>| {
        map.iter()
            .map(|((c, n, e), v)| {
                (
                    c.clone(),
                    n.clone(),
                    e.trim_start_matches("pptx:").to_owned(),
                    *v,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(&count(&compiled.diagnostics, "PX2402")),
        ids(&fallback_transitions)
    );
    // PX2410 also notes a tier-2 step that plays without part of its behaviours; a fallback has exactly one.
    let plays_fallback = |d: &Diagnostic| {
        d.data
            .as_ref()
            .and_then(|data| data.get("play"))
            .is_some_and(|play| {
                ["appear", "dissolve", "none"].contains(&play.to_js_string().as_str())
            })
    };
    let unmapped: Vec<Diagnostic> = compiled
        .diagnostics
        .iter()
        .filter(|d| plays_fallback(d))
        .cloned()
        .collect();
    assert_eq!(ids(&count(&unmapped, "PX2410")), ids(&fallback_steps));
    let partial: Vec<String> = compiled
        .diagnostics
        .iter()
        .filter(|d| d.code.code() == "PX2410" && !plays_fallback(d))
        .map(|d| key(d).2)
        .collect();
    // Every behaviour of the probe's tier-2 steps plays: a behaviour's own reversal folds into its track.
    assert_eq!(partial, Vec::<String>::new());
    assert_eq!(
        ids(&count(&compiled.diagnostics, "PX2413")),
        ids(&approximated)
    );
    let emitted = slidock_html::emit_documents_shipping(
        presentation,
        &slidock_html::EmitOptions::new(slidock_html::Provenance {
            format: "pptx".to_owned(),
            name: "motion-probe.pptx".to_owned(),
            sha256: "0".repeat(64),
            bytes: bytes.len() as u64,
        }),
        &slidock_html::FontStacks::new(),
        slidock_html::Motion::On,
    )
    .unwrap();
    let mut all = fallback_steps.clone();
    all.extend(fallback_transitions.clone());
    assert_eq!(ids(&count(&emitted.diagnostics, "HTML6010")), ids(&all));
    println!(
        "fallback steps {}, fallback transitions {}, approximations {}",
        fallback_steps.len(),
        fallback_transitions.len(),
        approximated.len()
    );
    assert!(
        !fallback_steps.is_empty() && !fallback_transitions.is_empty() && !approximated.is_empty()
    );
}

/// Flash Bulb (a pulse that reverses on its own, a fade out to half and back under a time filter) and Flicker
/// (colours to white and back) end at rest: every track of theirs ends on its first frame. The probe's Blast is its
/// own held scale (a tier-2 transform emphasis that keeps its end), which stays held: the case the player and the lane
/// check a held emphasis with.
#[test]
fn flash_bulb_and_flicker_end_at_rest_and_the_held_scale_holds() {
    let bytes = std::fs::read(probes().join("motion-probe.pptx")).unwrap();
    let lowered = support::lower_native(&bytes);
    let mut seen = BTreeMap::new();
    for slide in &lowered.presentation().slides {
        for step in &slide.timeline {
            let step = serde_json::to_value(step).unwrap();
            let native = step["effect"]["native"]["id"].as_str().unwrap_or_default();
            if !["pptx:emph.26", "pptx:emph.27", "pptx:emph.14"].contains(&native) {
                continue;
            }
            let tracks = step["effect"]["native"]["params"].as_array().unwrap();
            assert!(!tracks.is_empty(), "{step}");
            for track in tracks {
                let frames = track["frames"].as_array().unwrap();
                let (first, last) = (
                    &frames.first().unwrap()["value"],
                    &frames.last().unwrap()["value"],
                );
                if native == "pptx:emph.14" {
                    assert_eq!(track["property"], "scale", "{track}");
                    assert_eq!(*last, json!({ "x": 1.1, "y": 1.1 }), "{track}");
                } else {
                    assert_eq!(first, last, "{native}: {track}");
                }
            }
            *seen.entry(native.to_owned()).or_insert(0) += 1;
        }
    }
    assert_eq!(
        seen.keys().collect::<Vec<_>>(),
        ["pptx:emph.14", "pptx:emph.26", "pptx:emph.27"]
    );
}
