//! The motion probe deck (`probes/out/motion-probe.odp`) against its expectations
//! (`probes/out/motion-probe.expect.json`): every ODP slide's transition and steps, lowered with native motion, as
//! the expectations state them, and the slides the PPTX deck shares agreeing with the PPTX expectations on what
//! `conventions.crossFormat` says both formats share.
//!
//! [`CONFLICTS`] lists, field by field, where an expectation contradicts the IR contract or another merged table,
//! each with its reason; the test fails on any other mismatch, and on a listed conflict that no longer occurs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use slidock_ir::diagnostics::{DataValue, Diagnostic, Severity};

const PROBE: &[u8] = include_bytes!("../../../probes/out/motion-probe.odp");
const EXPECT: &str = include_str!("../../../probes/out/motion-probe.expect.json");

/// The duration an effect with an invalid one plays for (`timing::DEFAULT_DURATION`).
const DEFAULT_DURATION: u64 = 500;

/// Where an expectation and the frontend disagree on purpose: (slide key, target xml id, field, reason).
const CONFLICTS: &[(&str, &str, &str, &str)] = &[];

fn declared() -> BTreeSet<(String, String, String)> {
    CONFLICTS
        .iter()
        .map(|(key, shape, field, _)| (key.to_string(), shape.to_string(), field.to_string()))
        .collect()
}

fn expectations() -> Value {
    serde_json::from_str(EXPECT).unwrap()
}

/// A value token as the expectations spell it.
fn token(value: &Value) -> Value {
    match value {
        Value::String(s) => Value::String(
            match s.as_str() {
                "horz" => "horizontal",
                "vert" => "vertical",
                other => other,
            }
            .to_ascii_lowercase(),
        ),
        Value::Number(n) => Value::String(n.to_string().to_ascii_lowercase()),
        other => other.clone(),
    }
}

/// Whether an actual parameter is the expected one.
fn param_matches(key: &str, want: &Value, got: Option<&Value>) -> bool {
    let Some(got) = got else { return false };
    match key {
        "scale" => {
            let pair = Value::Array(vec![got["x"].clone(), got["y"].clone()]);
            support::js(&pair) == support::js(want)
        }
        "d" => {
            let printed = got
                .as_array()
                .map(|segments| {
                    segments
                        .iter()
                        .flat_map(|segment| segment.as_array().cloned().unwrap_or_default())
                        .map(|token| match token {
                            Value::String(s) => s,
                            other => support::js(&other).to_string(),
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            want.as_str() == Some(printed.as_str())
        }
        _ => token(want) == token(&support::js(got)),
    }
}

/// The diagnostics of one container, by the step or transition they name: (node, effect) → codes.
fn diagnostics_by_site(
    diagnostics: &[Diagnostic],
) -> BTreeMap<(String, Option<String>, String), Vec<String>> {
    let mut out: BTreeMap<_, Vec<String>> = BTreeMap::new();
    for diagnostic in diagnostics {
        let Some(container) = &diagnostic.container else {
            continue;
        };
        let effect = diagnostic
            .data
            .as_ref()
            .and_then(|data| data.get("effect"))
            .and_then(|value| match value {
                DataValue::Str(s) => Some(s.clone()),
                _ => None,
            });
        let Some(effect) = effect else { continue };
        out.entry((
            container.as_str().to_owned(),
            diagnostic.node.as_ref().map(|n| n.as_str().to_owned()),
            effect,
        ))
        .or_default()
        .push(diagnostic.code.code().to_owned());
    }
    out
}

struct Report {
    mismatches: BTreeSet<(String, String, String)>,
    details: Vec<String>,
    slides: usize,
    steps: usize,
    transitions: usize,
    matched: usize,
    fallback_with_diagnostic: usize,
}

impl Report {
    fn new() -> Report {
        Report {
            mismatches: BTreeSet::new(),
            details: Vec::new(),
            slides: 0,
            steps: 0,
            transitions: 0,
            matched: 0,
            fallback_with_diagnostic: 0,
        }
    }

    fn check(
        &mut self,
        key: &str,
        shape: &str,
        field: &str,
        ok: bool,
        detail: impl FnOnce() -> String,
    ) -> bool {
        if !ok {
            self.mismatches
                .insert((key.to_owned(), shape.to_owned(), field.to_owned()));
            self.details
                .push(format!("{key} {shape} {field}: {}", detail()));
        }
        ok
    }
}

/// Which fields to compare: all of them (the ODP expectations), or those `conventions.crossFormat` shares.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fields {
    All,
    Shared,
}

struct Actual<'a> {
    slide: &'a Value,
    container: String,
    elements: BTreeMap<String, String>,
    sites: &'a BTreeMap<(String, Option<String>, String), Vec<String>>,
}

impl Actual<'_> {
    fn codes(&self, node: Option<&str>, effect: &str) -> Vec<String> {
        let mut codes = self
            .sites
            .get(&(
                self.container.clone(),
                node.map(str::to_owned),
                effect.to_owned(),
            ))
            .cloned()
            .unwrap_or_default();
        codes.sort();
        codes.dedup();
        codes
    }
}

/// An element id by its native xml id, group children included.
fn elements_by_native(slide: &Value) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut stack: Vec<&Value> = slide["elements"].as_array().into_iter().flatten().collect();
    while let Some(element) = stack.pop() {
        if let Some(object) = element["origin"]["object"].as_str() {
            out.insert(
                object.to_owned(),
                element["id"].as_str().unwrap().to_owned(),
            );
        }
        stack.extend(element["children"].as_array().into_iter().flatten());
    }
    out
}

fn native_of(effect: &Value) -> String {
    effect["native"]["id"]
        .as_str()
        .or_else(|| effect["id"].as_str())
        .unwrap()
        .to_owned()
}

fn expected_code(value: &Value) -> Vec<String> {
    value["code"]
        .as_str()
        .map(|code| vec![code.to_owned()])
        .unwrap_or_default()
}

fn compare_transition(
    report: &mut Report,
    key: &str,
    want: &Value,
    actual: &Actual<'_>,
    fields: Fields,
) {
    let got = &actual.slide["transition"];
    if !report.check(
        key,
        "-",
        "transition",
        want.is_null() == got.is_null(),
        || format!("expected {want}, got {got}"),
    ) || want.is_null()
    {
        return;
    }
    report.transitions += 1;
    let mut ok = true;
    let native = native_of(&got["effect"]);
    if fields == Fields::All {
        ok &= report.check(
            key,
            "-",
            "transition.native",
            want["native"] == native,
            || format!("expected {}, got {native}", want["native"]),
        );
        ok &= report.check(
            key,
            "-",
            "transition.fallback",
            want["fallback"] == got["effect"]["fallback"],
            || {
                format!(
                    "expected {}, got {}",
                    want["fallback"], got["effect"]["fallback"]
                )
            },
        );
        let codes = actual.codes(None, &native);
        ok &= report.check(
            key,
            "-",
            "transition.diagnostic",
            codes == expected_code(&want["diagnostic"]),
            || format!("expected {}, got {codes:?}", want["diagnostic"]),
        );
        if !want["diagnostic"].is_null() && want["effect"].is_null() {
            report.fallback_with_diagnostic += 1;
        }
    }
    let effect = &got["effect"]["native"]["effect"];
    let effect = if effect.is_null() {
        Value::Null
    } else {
        effect.clone()
    };
    ok &= report.check(
        key,
        "-",
        "transition.effect",
        want["effect"] == effect,
        || format!("expected {}, got {effect}", want["effect"]),
    );
    if let Some(params) = want["params"].as_object() {
        for (name, value) in params {
            if fields == Fields::Shared && name == "soft" {
                continue;
            }
            let got_value = got["effect"]["native"]["params"].get(name);
            ok &= report.check(
                key,
                "-",
                &format!("transition.params.{name}"),
                param_matches(name, value, got_value),
                || format!("expected {value}, got {got_value:?}"),
            );
        }
    }
    let speed_slide = key.starts_with("tm-speed-");
    if fields == Fields::All || !speed_slide {
        ok &= report.check(
            key,
            "-",
            "transition.durationMs",
            want["durationMs"] == got["durationMs"],
            || format!("expected {}, got {}", want["durationMs"], got["durationMs"]),
        );
    }
    let advance = got["advance"].clone();
    let on_click = advance["onClick"].as_bool().unwrap_or(true);
    ok &= report.check(
        key,
        "-",
        "transition.advance",
        want["advance"]["onClick"] == on_click && want["advance"]["afterMs"] == advance["afterMs"],
        || format!("expected {}, got {advance}", want["advance"]),
    );
    if ok {
        report.matched += 1;
    }
}

fn clip_matches(want: &Value, tracks: &Value) -> bool {
    let name = want["effect"].as_str().unwrap_or_default();
    let tracks = tracks.as_array().cloned().unwrap_or_default();
    match name {
        "fade" => tracks.iter().any(|t| t["property"] == "opacity"),
        "flyIn" => {
            let dir = want["params"]["dir"].as_str().unwrap_or_default();
            tracks.iter().any(|t| {
                t["property"] == "translate" && {
                    let first = &t["frames"][0]["value"];
                    let (x, y) = (
                        first["x"].as_f64().unwrap_or(0.0),
                        first["y"].as_f64().unwrap_or(0.0),
                    );
                    match dir {
                        "d" => y < 0.0,
                        "u" => y > 0.0,
                        "r" => x < 0.0,
                        "l" => x > 0.0,
                        _ => false,
                    }
                }
            })
        }
        _ => tracks.iter().any(|t| {
            t["property"] == "clipPath"
                && t["reveal"]["effect"] == name
                && want["params"].as_object().is_none_or(|params| {
                    params
                        .iter()
                        .all(|(k, v)| param_matches(k, v, t["reveal"]["params"].get(k)))
                })
        }),
    }
}

fn compare_steps(
    report: &mut Report,
    key: &str,
    want: &[Value],
    actual: &Actual<'_>,
    fields: Fields,
) {
    let timeline: Vec<&Value> = actual.slide["timeline"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let mut kept = timeline.iter();
    for step in want {
        if step["skip"] == true {
            if step["dropped"] != true {
                kept.next();
            }
            continue;
        }
        report.steps += 1;
        let shape = step["target"]["xmlId"].as_str().unwrap_or("-").to_owned();
        let element = actual.elements.get(&shape).cloned();
        let native = step["native"].as_str().unwrap_or_default();
        let dropped = step["dropped"].as_bool() == Some(true);
        if dropped {
            let codes = actual.codes(element.as_deref(), native);
            let ok = fields == Fields::Shared
                || report.check(
                    key,
                    &shape,
                    "diagnostic",
                    codes == expected_code(&step["diagnostic"]),
                    || format!("expected {}, got {codes:?}", step["diagnostic"]),
                );
            if ok {
                report.matched += 1;
            }
            continue;
        }
        let Some(got) = kept.next() else {
            report.check(key, &shape, "step", false, || {
                "missing from the timeline".to_owned()
            });
            continue;
        };
        let mut ok = true;
        let mut check =
            |report: &mut Report, field: &str, good: bool, detail: &dyn Fn() -> String| {
                ok &= report.check(key, &shape, field, good, detail);
            };
        if fields == Fields::All {
            check(
                report,
                "target",
                element.as_deref() == got["target"]["element"].as_str(),
                &|| format!("expected {element:?}, got {}", got["target"]["element"]),
            );
        }
        let got_native = native_of(&got["effect"]);
        let effect = got["effect"]["native"]["effect"].clone();
        let tier = match effect.as_str() {
            None => Value::Null,
            Some("behaviors") => Value::from(2),
            Some(_) => Value::from(1),
        };
        let paragraphs = got["target"]["paragraphs"].clone();
        let background = got["target"]["background"].as_bool().unwrap_or(false);
        let timing = &got["timing"];
        let stagger = if timing["stagger"].is_null() {
            Value::Null
        } else {
            serde_json::json!({ "perUnitMs": timing["stagger"]["perUnit"]["ms"], "order": timing["stagger"]["order"] })
        };
        let easing = if timing["easing"].is_null() {
            Value::Null
        } else {
            timing["easing"]["accelDecel"].clone()
        };
        let after = if timing["after"].is_null() {
            Value::from("keep")
        } else {
            timing["after"].clone()
        };
        let duration = if step["durationMs"].is_null() {
            Value::from(DEFAULT_DURATION)
        } else {
            step["durationMs"].clone()
        };
        let delay = if step["delayMs"].is_null() {
            Value::from(0)
        } else {
            step["delayMs"].clone()
        };
        let codes = actual.codes(element.as_deref(), &got_native);
        if fields == Fields::All {
            check(report, "native", got_native == native, &|| {
                format!("expected {native}, got {got_native}")
            });
            check(
                report,
                "fallback",
                step["fallback"] == got["effect"]["fallback"],
                &|| {
                    format!(
                        "expected {}, got {}",
                        step["fallback"], got["effect"]["fallback"]
                    )
                },
            );
            // An approximation is reported (OD9413), as the expectation's diagnostic states.
            check(
                report,
                "diagnostic",
                codes == expected_code(&step["diagnostic"]),
                &|| format!("expected {}, got {codes:?}", step["diagnostic"]),
            );
            if !step["diagnostic"].is_null() && step["effect"].is_null() {
                report.fallback_with_diagnostic += 1;
            }
        }
        let checks: [(&str, Value, Value); 13] = [
            ("effect", step["effect"].clone(), effect.clone()),
            ("tier", step["tier"].clone(), tier),
            ("trigger", step["trigger"].clone(), got["trigger"].clone()),
            ("kind", step["kind"].clone(), got["kind"].clone()),
            (
                "delivery",
                step["delivery"].clone(),
                got["delivery"].clone(),
            ),
            ("stagger", step["stagger"].clone(), stagger),
            ("paragraphs", step["paragraphs"].clone(), paragraphs),
            (
                "background",
                Value::from(step["background"].as_bool() == Some(true)),
                Value::from(background),
            ),
            (
                "offsetMs",
                step["offsetMs"].clone(),
                timing["offsetMs"].clone(),
            ),
            ("delayMs", delay, got["delayMs"].clone()),
            ("durationMs", duration, got["durationMs"].clone()),
            ("easing", step["easing"].clone(), easing),
            ("after", step["after"].clone(), after),
        ];
        for (field, want_value, got_value) in checks {
            check(
                report,
                field,
                support::js(&want_value) == support::js(&got_value),
                &|| format!("expected {want_value}, got {got_value}"),
            );
        }
        let repeat = timing["repeat"].clone();
        check(
            report,
            "repeat",
            support::js(&step["repeat"]) == support::js(&repeat),
            &|| format!("expected {}, got {repeat}", step["repeat"]),
        );
        let reverse = timing["autoReverse"].as_bool().unwrap_or(false);
        check(
            report,
            "autoReverse",
            step["autoReverse"].as_bool() == Some(reverse),
            &|| format!("expected {}, got {reverse}", step["autoReverse"]),
        );
        if !step["hostileTiming"].is_null() && fields == Fields::All {
            check(
                report,
                "hostileTiming",
                codes.iter().any(|code| code == "OD9411"),
                &|| format!("no OD9411 for {}", step["hostileTiming"]),
            );
        }
        if effect == step["effect"]
            && let Some(params) = step["params"].as_object()
        {
            for (name, value) in params {
                let got_value = got["effect"]["native"]["params"].get(name);
                check(
                    report,
                    &format!("params.{name}"),
                    param_matches(name, value, got_value),
                    &|| format!("expected {value}, got {got_value:?}"),
                );
            }
        }
        if let Some(clip) = step.get("clip").filter(|clip| !clip.is_null()) {
            check(
                report,
                "clip",
                clip_matches(clip, &got["effect"]["native"]["params"]),
                &|| format!("expected {clip}, got {}", got["effect"]["native"]["params"]),
            );
        }
        if ok {
            report.matched += 1;
        }
    }
    let extra = kept.count();
    report.check(key, "-", "steps", extra == 0, || {
        format!("{extra} steps more than expected")
    });
}

/// Compares the lowered probe with one deck's expectations.
fn compare(deck: &str, fields: Fields) -> Report {
    let expect = expectations();
    let lowered = support::lower_motion(PROBE);
    let sites = diagnostics_by_site(&lowered.diagnostics);
    let odp: BTreeMap<&str, usize> = expect["decks"]["motion-probe.odp"]["slideList"]
        .as_array()
        .unwrap()
        .iter()
        .map(|slide| {
            (
                slide["key"].as_str().unwrap(),
                slide["index"].as_u64().unwrap() as usize - 1,
            )
        })
        .collect();
    let slides = lowered.json["slides"].as_array().unwrap();
    let mut report = Report::new();
    for want in expect["decks"][deck]["slideList"].as_array().unwrap() {
        let key = want["key"].as_str().unwrap();
        let Some(&index) = odp.get(key) else { continue };
        report.slides += 1;
        let slide = &slides[index];
        let actual = Actual {
            slide,
            container: slide["id"].as_str().unwrap().to_owned(),
            elements: elements_by_native(slide),
            sites: &sites,
        };
        compare_transition(&mut report, key, &want["transition"], &actual, fields);
        let odp_steps = expect["decks"]["motion-probe.odp"]["slideList"][index]["steps"]
            .as_array()
            .unwrap();
        let steps: Vec<Value> = match fields {
            Fields::All => want["steps"].as_array().unwrap().clone(),
            // The PPTX steps in the ODP deck's order, matched by their shape's label; an ODP-only step is skipped.
            Fields::Shared => {
                let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
                odp_steps
                    .iter()
                    .map(|odp| {
                        let label = odp["shape"].as_str().unwrap();
                        let n = seen.entry(label).or_default();
                        let pptx = want["steps"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .filter(|step| step["shape"] == label)
                            .nth(*n);
                        *n += 1;
                        match pptx {
                            Some(pptx) => {
                                let mut step = pptx.clone();
                                step["target"] = odp["target"].clone();
                                step
                            }
                            None => serde_json::json!({ "skip": true, "dropped": odp["dropped"] }),
                        }
                    })
                    .collect()
            }
        };
        compare_steps(&mut report, key, &steps, &actual, fields);
        if fields == Fields::All {
            let timeline = slide["timeline"].as_array().cloned().unwrap_or_default();
            let clicks = timeline
                .iter()
                .filter(|step| step["trigger"] == "click")
                .count()
                + steps
                    .iter()
                    .filter(|step| step["dropped"] == true && step["trigger"] == "click")
                    .count();
            report.check(
                key,
                "-",
                "clickGroups",
                want["clickGroups"] == clicks,
                || format!("expected {}, got {clicks}", want["clickGroups"]),
            );
            let auto = timeline
                .first()
                .is_some_and(|step| step["trigger"] != "click");
            report.check(key, "-", "autoGroup", want["autoGroup"] == auto, || {
                format!("expected {}, got {auto}", want["autoGroup"])
            });
        }
    }
    report
}

#[test]
fn every_odp_slide_meets_its_expectation() {
    let report = compare("motion-probe.odp", Fields::All);
    let declared = declared();
    let unexpected: Vec<&String> = report
        .details
        .iter()
        .filter(|detail| {
            let mut parts = detail.splitn(4, ' ');
            let key = parts.next().unwrap_or_default().to_owned();
            let shape = parts.next().unwrap_or_default().to_owned();
            let field = parts
                .next()
                .unwrap_or_default()
                .trim_end_matches(':')
                .to_owned();
            !declared.contains(&(key, shape, field))
        })
        .collect();
    let stale: Vec<_> = declared.difference(&report.mismatches).collect();
    println!(
        "slides {}, steps {}, transitions {}, matched {}, fallback with diagnostic {}, declared conflicts {}, mismatched {}",
        report.slides,
        report.steps,
        report.transitions,
        report.matched,
        report.fallback_with_diagnostic,
        report.mismatches.len() - unexpected.len().min(report.mismatches.len()),
        unexpected.len()
    );
    assert_eq!(report.slides, 178);
    assert_eq!(report.steps, 360);
    assert_eq!(report.transitions, 106);
    assert!(
        unexpected.is_empty(),
        "unexpected mismatches:\n{}",
        unexpected
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        stale.is_empty(),
        "declared conflicts that no longer occur: {stale:?}"
    );
}

#[test]
fn shared_slides_agree_with_the_pptx_expectations() {
    let report = compare("motion-probe.pptx", Fields::Shared);
    let declared = declared();
    let unexpected: Vec<&String> = report
        .details
        .iter()
        .filter(|detail| {
            let mut parts = detail.splitn(4, ' ');
            let key = parts.next().unwrap_or_default().to_owned();
            let shape = parts.next().unwrap_or_default().to_owned();
            let field = parts
                .next()
                .unwrap_or_default()
                .trim_end_matches(':')
                .to_owned();
            !declared.contains(&(key, shape, field))
        })
        .collect();
    println!(
        "shared slides {}, steps {}, transitions {}, matched {}",
        report.slides, report.steps, report.transitions, report.matched
    );
    assert_eq!(report.slides, 139);
    assert!(
        unexpected.is_empty(),
        "the ODP and PPTX probe slides disagree:\n{}",
        unexpected
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn the_probe_compiles_with_native_motion_without_errors() {
    let compiled = support::compile_motion(PROBE);
    let errors: Vec<&Diagnostic> = compiled
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error || d.code.code().starts_with("IR"))
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
    let presentation = compiled.presentation.unwrap();
    let lowered = support::lower_motion(PROBE);
    for (resolved, source) in presentation.slides.iter().zip(&lowered.presentation.slides) {
        assert_eq!(resolved.timeline, source.timeline, "{}", source.id);
        assert_eq!(resolved.transition, source.transition, "{}", source.id);
    }
    assert!(
        presentation
            .slides
            .iter()
            .any(|slide| !slide.timeline.is_empty())
    );
    // The session's native motion is what the frontend reads: off, the deck holds no motion.
    let parity = support::compile(PROBE).presentation.unwrap();
    assert!(
        parity
            .slides
            .iter()
            .all(|slide| slide.timeline.is_empty() && slide.transition.is_none())
    );
}

#[test]
fn every_motion_feature_the_frontend_detects_reaches_the_ir_or_is_reported() {
    // The loss check's frontend stage (slidock's loss.rs): a detected feature is in the source IR's census, or a
    // frontend diagnostic at its floor or above names it on its container.
    for native_motion in [false, true] {
        let lowered = if native_motion {
            support::lower_motion(PROBE)
        } else {
            support::lower(PROBE)
        };
        let presentation = &lowered.presentation;
        let mut lost = Vec::new();
        for container in presentation.containers() {
            let census = slidock_ir::features::census_container(container, presentation);
            let id = container.id();
            for feature in lowered
                .lowered
                .features
                .get(id.as_str())
                .into_iter()
                .flatten()
            {
                let named = lowered.diagnostics.iter().any(|d| {
                    d.feature == Some(*feature)
                        && d.container.as_ref().is_none_or(|c| c == id)
                        && d.severity.at_least(feature.floor())
                });
                if !census.contains(feature) && !named {
                    lost.push(format!("{} {}", id.as_str(), feature.as_str()));
                }
            }
        }
        assert!(lost.is_empty(), "native motion {native_motion}: {lost:?}");
    }
}

/// Whether a native id's preset or transition row approximates it: `odp:<preset-id>[/<sub-type>]` or
/// `odp:<type>[/<subtype>][/reverse]`.
fn approximate_row(native: &str) -> bool {
    let name = native.strip_prefix("odp:").unwrap();
    let mut parts = name.split('/');
    let id = parts.next().unwrap();
    let rest: Vec<&str> = parts.collect();
    for class in ["entrance", "exit", "emphasis"] {
        if id.starts_with(&format!("ooo-{class}-")) {
            return slidock_presets::odp::build(class, id, rest.first().copied())
                .is_some_and(|lookup| lookup.row.approximate);
        }
    }
    let reverse = rest.last() == Some(&"reverse");
    let subtype = rest.first().copied().filter(|s| *s != "reverse");
    slidock_presets::odp::transition(id, subtype, reverse, None)
        .is_some_and(|lookup| lookup.row.approximate)
}

/// Every step and transition that plays its fallback is reported once by the frontend (OD9402 for a transition,
/// OD9410 for a step) and once by the deck path (HTML6010), and every approximation that plays once (OD9413).
#[test]
fn every_fallback_and_approximation_is_reported_once() {
    let compiled = support::compile_motion(PROBE);
    let presentation = compiled.presentation.as_ref().unwrap();
    type Key = (String, Option<String>, String);
    let key = |d: &Diagnostic| -> Key {
        let effect = match d.data.as_ref().and_then(|data| data.get("effect")) {
            Some(DataValue::Str(s)) => s.clone(),
            _ => String::new(),
        };
        (
            d.container
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            d.node.as_ref().map(ToString::to_string),
            effect,
        )
    };
    let count = |diagnostics: &[Diagnostic], code: &str| {
        let mut out: BTreeMap<Key, usize> = BTreeMap::new();
        for d in diagnostics.iter().filter(|d| d.code.code() == code) {
            *out.entry(key(d)).or_default() += 1;
        }
        out
    };
    let mut fallback_steps: BTreeMap<Key, usize> = BTreeMap::new();
    let mut fallback_transitions: BTreeMap<Key, usize> = BTreeMap::new();
    let mut approximated: BTreeMap<Key, usize> = BTreeMap::new();
    for slide in &presentation.slides {
        let container = slide.id.to_string();
        for step in &slide.timeline {
            let node = Some(step.target.element.to_string());
            match &step.effect.native {
                None => {
                    fallback_steps.insert((container.clone(), node, step.effect.id.to_string()), 1);
                }
                Some(native)
                    if native.effect.name() != "behaviors"
                        && approximate_row(&native.id.to_string()) =>
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
                Some(native) if approximate_row(&native.id.to_string()) => {
                    approximated.insert((container.clone(), None, native.id.to_string()), 1);
                }
                Some(_) => {}
            }
        }
    }
    assert_eq!(count(&compiled.diagnostics, "OD9402"), fallback_transitions);
    // OD9410 also notes what tier 2 or a vocabulary effect leaves out; a fallback has exactly one.
    let unmapped = count(&compiled.diagnostics, "OD9410");
    for key in fallback_steps.keys() {
        assert_eq!(unmapped.get(key), Some(&1), "{key:?}");
    }
    assert_eq!(count(&compiled.diagnostics, "OD9413"), approximated);
    let emitted = slidock_html::emit_documents_shipping(
        presentation,
        &slidock_html::EmitOptions::new(slidock_html::Provenance {
            format: "odp".to_owned(),
            name: "motion-probe.odp".to_owned(),
            sha256: "0".repeat(64),
            bytes: PROBE.len() as u64,
        }),
        &slidock_html::FontStacks::new(),
        slidock_html::Motion::On,
    )
    .unwrap();
    let mut all = fallback_steps.clone();
    all.extend(fallback_transitions.clone());
    assert_eq!(count(&emitted.diagnostics, "HTML6010"), all);
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
    let lowered = support::lower_motion(PROBE);
    let mut seen = BTreeMap::new();
    for slide in lowered.json["slides"].as_array().unwrap() {
        for step in slide["timeline"].as_array().into_iter().flatten() {
            let native = step["effect"]["native"]["id"].as_str().unwrap_or_default();
            if ![
                "odp:ooo-emphasis-flash-bulb",
                "odp:ooo-emphasis-flicker",
                "odp:ooo-emphasis-blast",
            ]
            .contains(&native)
            {
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
                if native == "odp:ooo-emphasis-blast" {
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
        [
            "odp:ooo-emphasis-blast",
            "odp:ooo-emphasis-flash-bulb",
            "odp:ooo-emphasis-flicker"
        ]
    );
}

/// LibreOffice's `transitionfactorytab` rows that set `outInvertsSweep` (LibreOffice 25.2.3.2, as its SVG export's
/// `presentation_engine.js` holds them): in `smil:mode="out"` its `ClippingFunctor` runs these filters' sweep
/// backwards, so an exit plays the entrance its filter reads as, reversed. `barWipe` does not set it: the polygon is
/// subtracted instead, and the edge keeps its motion.
const OUT_INVERTS_SWEEP: [&str; 4] = ["irisWipe", "ellipseWipe", "fourBoxWipe", "barnDoorWipe"];

/// Every exit iris, split and wipe of the probe against what LibreOffice's engine plays for its filter: the entrance
/// the filter reads as (its type, sub-type and direction), played backwards. An iris's or a split's in/out and a
/// wipe's direction agree (`cargo xtask lo-engine` checks the iris and split clips against the engine's frames).
#[test]
fn exit_iris_split_and_wipe_play_as_libreoffice() {
    use slidock_ir::effects::{InOut, TransitionEffect};
    use std::io::Read as _;

    let mut content = String::new();
    zip::ZipArchive::new(std::io::Cursor::new(PROBE))
        .unwrap()
        .by_name("content.xml")
        .unwrap()
        .read_to_string(&mut content)
        .unwrap();
    let limits = slidock_io::XmlLimits::default();
    let document =
        slidock_io::parse_xml(&content, &limits, &mut slidock_io::XmlBudget::new(&limits)).unwrap();
    let names = ["box", "circle", "oval", "diamond", "plus", "split", "wipe"];
    // (preset id, sub-type) → (type, sub-type, reversed, closes), as the deck writes the preset's filter.
    let mut filters: BTreeMap<String, (String, String, bool, bool)> = BTreeMap::new();
    // Attributes by local name: the deck writes each once per element.
    let attr = |node: slidock_io::roxmltree::Node<'_, '_>, local: &str| {
        node.attributes()
            .find(|a| a.name() == local)
            .map(|a| a.value().to_owned())
    };
    for node in document.descendants().filter(|node| node.is_element()) {
        let Some(id) = attr(node, "preset-id").filter(|id| {
            attr(node, "preset-class").as_deref() == Some("exit")
                && names.iter().any(|name| *id == format!("ooo-exit-{name}"))
        }) else {
            continue;
        };
        let sub = attr(node, "preset-sub-type").unwrap_or_default();
        let filter = node
            .descendants()
            .find(|kid| kid.tag_name().name() == "transitionFilter")
            .unwrap();
        let read = (
            attr(filter, "type").unwrap(),
            attr(filter, "subtype").unwrap(),
            attr(filter, "direction").as_deref() == Some("reverse"),
            attr(filter, "mode").as_deref() == Some("out"),
        );
        let key = format!("odp:{id}/{sub}");
        assert_eq!(
            filters.entry(key.clone()).or_insert(read.clone()),
            &read,
            "{key}"
        );
    }
    let in_out = |value: InOut| json!(value);
    let mut agree = BTreeMap::<String, usize>::new();
    let lowered = support::lower_motion(PROBE);
    for slide in lowered.json["slides"].as_array().unwrap() {
        for step in slide["timeline"].as_array().into_iter().flatten() {
            let native = &step["effect"]["native"];
            let Some((ty, subtype, reverse, closes)) =
                native["id"].as_str().and_then(|id| filters.get(id))
            else {
                continue;
            };
            assert!(closes, "{native}");
            let row = slidock_presets::odp::transition(ty, Some(subtype), *reverse, None).unwrap();
            let (param, libreoffice) = match row.row.effect.unwrap() {
                TransitionEffect::Wipe(p) => ("dir", json!(p.dir)),
                TransitionEffect::Iris(p) => ("inOut", in_out(p.in_out)),
                TransitionEffect::Split(p) => ("inOut", in_out(p.in_out)),
                other => panic!("{native}: {other:?}"),
            };
            assert_eq!(
                OUT_INVERTS_SWEEP.contains(&ty.as_str()),
                param == "inOut",
                "{native}"
            );
            assert_eq!(
                native["params"][param], libreoffice,
                "{native}: LibreOffice plays {param} {libreoffice}"
            );
            *agree
                .entry(native["effect"].as_str().unwrap().to_owned())
                .or_default() += 1;
        }
    }
    println!("exits LibreOffice plays the same: {agree:?}");
    assert_eq!(
        agree,
        BTreeMap::from([
            ("iris".to_owned(), 8),
            ("split".to_owned(), 4),
            ("wipe".to_owned(), 4)
        ])
    );
}
