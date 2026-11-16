//! Each transition and step of a deck gets a verdict: native, a declared fallback, an accepted fallback, accepted (it
//! played as expected but failed a check a review tracks), dropped, or mismatched. A corpus deck is held to its own
//! deck data; a probe deck to its expectations (`probes/out/motion-probe.expect.json`: per slide, the effect the
//! vocabulary plays with its params, or the fallback when it has none). A fallback played where an effect is
//! expected passes only with an entry of `bench/motion/fallback-allow.json` naming its native id, a failed check only
//! with an accepted check naming its native id and the failure's exact detail; an entry no item uses is stale, which
//! fails the lane as well.

use std::collections::BTreeSet;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;
use slidock_bench_schema::lanes::{MotionItem, MotionPairReport, MotionVerdict};
use slidock_bench_schema::{Frontend, PairKey, PresentationId, Sha256, SlideId};

use super::check::{Item, ItemRef, Played, Problem};
use super::deck::MotionDeck;
use crate::error::{BenchError, Result};

/// The probe decks the lane plays beside the corpus.
pub const PROBES: &str = "bench/motion/probes.json";
/// Reviewed fallbacks (and substitutions) of items that expect an effect.
pub const FALLBACK_ALLOW: &str = "bench/motion/fallback-allow.json";

/// A probe deck of the repository and its expectations.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionProbe {
    pub id: PresentationId,
    pub frontend: Frontend,
    /// Relative to the repository.
    pub file: String,
    pub sha256: Sha256,
    /// The expectations file, relative to the repository.
    pub expect: String,
    /// The deck's key in the expectations' `decks`.
    pub deck: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionProbes {
    pub version: u8,
    pub probes: Vec<MotionProbe>,
}

pub fn load_probes(path: &Path) -> Result<MotionProbes> {
    let text = std::fs::read_to_string(path).map_err(BenchError::io(path))?;
    let probes: MotionProbes = serde_json::from_str(&text)
        .map_err(|error| BenchError::failed(format!("{}: {error}", path.display())))?;
    if probes.version != 1 {
        return Err(BenchError::failed(format!(
            "{}: version is not 1",
            path.display()
        )));
    }
    Ok(probes)
}

/// An accepted fallback: an item that expects an effect and plays something else, reviewed.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllowEntry {
    /// `<presentation>:<frontend>`.
    pub deck: PairKey,
    /// The probe's slide key, or the corpus slide's id.
    pub slide: String,
    /// `transition` or `step <index>`.
    pub item: String,
    pub native: String,
    /// What plays: `fallback:<name>`, or another effect.
    pub played: String,
    pub reason: String,
    /// Where the fix is tracked.
    pub tracked: String,
}

/// A reviewed failed check: a defect the lane finds, accepted until the fix it names lands.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedCheck {
    pub deck: PairKey,
    /// The probe's slide key, or the corpus slide's id; `*` for every slide of the deck (one defect of the deck as a
    /// whole).
    pub slide: String,
    /// `transition` or `step <index>`.
    pub item: String,
    /// The item's native id.
    pub native: String,
    /// The check (`end-state`, `start-state`, `settled`, `reveal`, `monotonic`, `params`, `placement`, or a timeline
    /// field such as `missing` or `moves`).
    pub check: String,
    /// The failure exactly as the lane details it: a different failure of the same check is not accepted.
    pub detail: String,
    pub reason: String,
    /// Where the fix is tracked.
    pub tracked: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Allowlist {
    pub version: u8,
    /// Accepted fallbacks and substitutions.
    pub entries: Vec<AllowEntry>,
    /// Accepted failed checks.
    #[serde(default)]
    pub checks: Vec<AcceptedCheck>,
}

pub fn load_allowlist(path: &Path) -> Result<Allowlist> {
    let text = std::fs::read_to_string(path).map_err(BenchError::io(path))?;
    let list: Allowlist = serde_json::from_str(&text)
        .map_err(|error| BenchError::failed(format!("{}: {error}", path.display())))?;
    if list.version != 1 {
        return Err(BenchError::failed(format!(
            "{}: version is not 1",
            path.display()
        )));
    }
    Ok(list)
}

/// What a probe expects of one item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expectation {
    /// The deck's slide index.
    pub index: usize,
    pub key: String,
    /// The item, by its index among the deck's steps (dropped steps take none).
    pub item: Option<ItemRef>,
    pub native: String,
    /// The vocabulary effect; `None` where the vocabulary has none and the fallback plays.
    pub effect: Option<String>,
    /// The effect's params the probe expects (a subset of what the deck may write).
    pub params: Option<Value>,
    pub fallback: Option<String>,
    pub dropped: bool,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// The expectations of one deck of an expectations file, in deck order.
pub fn expectations(expect: &Value, deck: &str) -> std::result::Result<Vec<Expectation>, String> {
    let slides = expect
        .get("decks")
        .and_then(|d| d.get(deck))
        .and_then(|d| d.get("slideList"))
        .and_then(Value::as_array)
        .ok_or_else(|| format!("the expectations have no deck {deck}"))?;
    let mut out = Vec::new();
    for slide in slides {
        let index = slide
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| i.checked_sub(1))
            .ok_or("a slide without its 1-based index")?;
        let key = text(slide, "key").ok_or("a slide without its key")?;
        if let Some(transition) = slide.get("transition").filter(|t| !t.is_null()) {
            out.push(Expectation {
                index,
                key: key.clone(),
                item: Some(ItemRef::Transition),
                native: text(transition, "native").unwrap_or_default(),
                effect: text(transition, "effect"),
                params: transition.get("params").filter(|p| !p.is_null()).cloned(),
                fallback: text(transition, "fallback"),
                dropped: false,
            });
        }
        let mut position = 0u32;
        for step in slide
            .get("steps")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let dropped = step.get("dropped").and_then(Value::as_bool) == Some(true);
            out.push(Expectation {
                index,
                key: key.clone(),
                item: (!dropped).then_some(ItemRef::Step(position)),
                native: text(step, "native").unwrap_or_default(),
                effect: text(step, "effect"),
                params: step.get("params").filter(|p| !p.is_null()).cloned(),
                fallback: text(step, "fallback"),
                dropped,
            });
            if !dropped {
                position += 1;
            }
        }
    }
    Ok(out)
}

/// A params value in one spelling: hex colours in lower case, `horizontal`/`vertical` as `horz`/`vert`, a number
/// written as a string as the number, a `[x, y]` pair as `{x, y}`, a path's `d` as its list of commands and numbers.
fn spelled(key: &str, value: &Value) -> Value {
    match value {
        Value::String(text) if key == "d" => Value::Array(path_tokens(text)),
        Value::String(text) => match text.as_str() {
            "horizontal" => Value::from("horz"),
            "vertical" => Value::from("vert"),
            _ if text.starts_with('#') => Value::from(text.to_ascii_lowercase()),
            _ => match text.parse::<f64>() {
                Ok(n) if n.is_finite() => Value::from(n),
                _ => value.clone(),
            },
        },
        Value::Number(n) => n.as_f64().map_or_else(|| value.clone(), Value::from),
        Value::Array(items) if key == "d" => Value::Array(
            items
                .iter()
                .flat_map(|segment| match segment {
                    Value::Array(parts) => parts.iter().map(|p| spelled("", p)).collect(),
                    other => vec![spelled("", other)],
                })
                .collect(),
        ),
        Value::Array(items) => match items.as_slice() {
            [x, y] if x.is_number() && y.is_number() => {
                let mut pair = serde_json::Map::new();
                pair.insert("x".into(), spelled("", x));
                pair.insert("y".into(), spelled("", y));
                Value::Object(pair)
            }
            _ => Value::Array(items.iter().map(|v| spelled(key, v)).collect()),
        },
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), spelled(k, v)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

/// A path's `d` string as its commands (strings) and numbers.
fn path_tokens(d: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut number = String::new();
    let flush = |number: &mut String, out: &mut Vec<Value>| {
        if let Ok(n) = number.parse::<f64>() {
            out.push(Value::from(n));
        }
        number.clear();
    };
    for c in d.chars() {
        if c.is_ascii_alphabetic() && c != 'e' {
            flush(&mut number, &mut out);
            out.push(Value::from(c.to_string()));
        } else if c.is_ascii_digit() || c == '.' || c == 'e' || (c == '-' && number.is_empty()) {
            number.push(c);
        } else if c == '-' {
            flush(&mut number, &mut out);
            number.push(c);
        } else {
            flush(&mut number, &mut out);
        }
    }
    flush(&mut number, &mut out);
    out
}

/// Where a deck's params differ from what a probe expects: each expected key whose value the deck spells otherwise.
pub fn param_differences(want: &Value, have: Option<&Value>) -> Vec<String> {
    let Some(want) = want.as_object() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, expected) in want {
        let actual = have.and_then(|h| h.get(key));
        let same = actual.is_some_and(|actual| spelled(key, actual) == spelled(key, expected));
        if !same {
            out.push(format!(
                "{key}: the probe expects {expected}, the deck has {}",
                actual.map_or_else(|| "none".to_owned(), Value::to_string)
            ));
        }
    }
    out
}

/// Adds a `params` problem to each item of the slide at `index` that plays the effect its expectation names with other
/// params than it expects (a frontend's direction or axis, or a contract the two read differently).
pub fn params_problems(expected: &[Expectation], index: usize, items: &mut [Item]) {
    for expectation in expected.iter().filter(|e| e.index == index) {
        let (Some(item_ref), Some(effect), Some(want)) =
            (expectation.item, &expectation.effect, &expectation.params)
        else {
            continue;
        };
        let Some(item) = items.iter_mut().find(|i| i.item == item_ref) else {
            continue;
        };
        if item.effect.as_ref() != Some(effect) {
            continue;
        }
        let differences = param_differences(want, item.params.as_ref());
        if !differences.is_empty() {
            let detail = differences.join("; ");
            item.problems.push(Problem::with(
                "params",
                format!("{effect} params differ from the probe's: {detail}"),
                detail,
            ));
        }
    }
}

/// The items of one slide as the lane checked them.
pub struct SlideItems<'a> {
    pub index: usize,
    pub id: &'a SlideId,
    pub items: &'a [Item],
}

/// Gives each item its verdict.
pub struct Verdicts<'a> {
    pub pair: &'a PairKey,
    pub deck: &'a MotionDeck,
    pub allow: &'a Allowlist,
    /// The allowlist entries some item used, by position.
    pub used: BTreeSet<usize>,
    /// The accepted checks some item used, by position.
    pub used_checks: BTreeSet<usize>,
}

impl Verdicts<'_> {
    /// Moves each problem of a slide's items that an accepted check covers into the item's `accepted` reasons.
    pub fn accept(&mut self, slide: &str, items: &mut [Item]) {
        for item in items {
            let label = item.item.label();
            let mut kept = Vec::new();
            for problem in std::mem::take(&mut item.problems) {
                let found = self.allow.checks.iter().enumerate().find(|(_, entry)| {
                    &entry.deck == self.pair
                        && (entry.slide == slide || entry.slide == "*")
                        && entry.item == label
                        && entry.native == item.native
                        && entry.check == problem.check
                        && entry.detail == problem.detail
                });
                match found {
                    Some((position, entry)) => {
                        self.used_checks.insert(position);
                        item.accepted
                            .push(Problem::new(&problem.check, entry.reason.clone()));
                    }
                    None => kept.push(problem),
                }
            }
            item.problems = kept;
        }
    }

    /// Native or declared, or accepted where a review accepts its failed checks (a check it failed by playing
    /// nothing included); an accepted fallback keeps its verdict and adds the checks' reasons.
    fn reviewed(
        item: &Item,
        verdict: MotionVerdict,
        why: Option<String>,
    ) -> (MotionVerdict, Option<String>) {
        if item.accepted.is_empty() {
            return (verdict, why);
        }
        let mut reasons: Vec<&str> = item.accepted.iter().map(|p| p.text.as_str()).collect();
        reasons.dedup();
        let reasons = reasons.join("; ");
        let explained = matches!(verdict, MotionVerdict::Native | MotionVerdict::Declared)
            || item.played == Played::Nothing;
        match verdict {
            _ if explained => (MotionVerdict::Accepted, Some(reasons)),
            MotionVerdict::Allowed => (
                verdict,
                Some(why.map_or(reasons.clone(), |why| format!("{why}; {reasons}"))),
            ),
            _ => (verdict, why),
        }
    }

    fn allowed(
        &mut self,
        slide: &str,
        item: ItemRef,
        native: &str,
        played: &Played,
    ) -> Option<String> {
        let label = item.label();
        let played = played.label();
        let (position, entry) = self.allow.entries.iter().enumerate().find(|(_, entry)| {
            &entry.deck == self.pair
                && entry.slide == slide
                && entry.item == label
                && entry.native == native
                && entry.played == played
        })?;
        self.used.insert(position);
        Some(entry.reason.clone())
    }

    /// A corpus deck: each item against the deck's own data.
    pub fn corpus(&mut self, slides: &[SlideItems<'_>]) -> MotionPairReport {
        let mut report = MotionPairReport::default();
        for slide in slides {
            for item in slide.items {
                let (verdict, why) = if !item.problems.is_empty() {
                    (MotionVerdict::Mismatched, Some(problem_text(item)))
                } else {
                    match (&item.played, &item.effect) {
                        (Played::Native(_), _) => (MotionVerdict::Native, None),
                        (Played::Fallback(_), None) => (MotionVerdict::Declared, None),
                        (played, Some(effect)) => {
                            match self.allowed(slide.id.as_str(), item.item, &item.native, played) {
                                Some(reason) => (MotionVerdict::Allowed, Some(reason)),
                                None => (
                                    MotionVerdict::Mismatched,
                                    Some(format!(
                                        "the deck names {effect}; the player played {}",
                                        played.label()
                                    )),
                                ),
                            }
                        }
                        (Played::Nothing, None) => (
                            MotionVerdict::Mismatched,
                            Some("the player played nothing".to_owned()),
                        ),
                    }
                };
                let (verdict, why) = Self::reviewed(item, verdict, why);
                push(
                    &mut report,
                    slide.id,
                    None,
                    item,
                    item.effect.clone(),
                    verdict,
                    why,
                );
            }
        }
        report
    }

    /// A probe deck: each expectation against the item it names.
    pub fn probe(
        &mut self,
        slides: &[SlideItems<'_>],
        expected: &[Expectation],
    ) -> MotionPairReport {
        let mut report = MotionPairReport {
            probe: true,
            ..MotionPairReport::default()
        };
        let mut covered: BTreeSet<(usize, ItemRef)> = BTreeSet::new();
        for expectation in expected {
            let slide = slides.iter().find(|s| s.index == expectation.index);
            let id = match slide {
                Some(slide) => slide.id.clone(),
                None => match self
                    .deck
                    .slides
                    .get(expectation.index)
                    .map(|s| SlideId::new(s.id.clone()))
                {
                    Some(Ok(id)) => id,
                    _ => continue,
                },
            };
            let Some(item_ref) = expectation.item else {
                report.dropped += 1;
                report.items.push(MotionItem {
                    slide: id,
                    key: Some(expectation.key.clone()),
                    item: "dropped step".to_owned(),
                    native: expectation.native.clone(),
                    expected: expectation.effect.clone(),
                    played: "nothing".to_owned(),
                    verdict: MotionVerdict::Dropped,
                    why: None,
                });
                continue;
            };
            covered.insert((expectation.index, item_ref));
            let item = slide.and_then(|s| s.items.iter().find(|i| i.item == item_ref));
            let Some(item) = item else {
                let missing = Item {
                    item: item_ref,
                    native: expectation.native.clone(),
                    effect: None,
                    params: None,
                    fallback: String::new(),
                    played: Played::Nothing,
                    problems: Vec::new(),
                    accepted: Vec::new(),
                };
                push(
                    &mut report,
                    &id,
                    Some(&expectation.key),
                    &missing,
                    expectation.effect.clone(),
                    MotionVerdict::Mismatched,
                    Some("the deck has no such item".to_owned()),
                );
                continue;
            };
            let (verdict, why) = if !item.problems.is_empty() {
                (MotionVerdict::Mismatched, Some(problem_text(item)))
            } else {
                match (&expectation.effect, &item.played) {
                    (Some(effect), Played::Native(name)) if name == effect => {
                        (MotionVerdict::Native, None)
                    }
                    (None, Played::Fallback(fallback))
                        if Some(fallback) == expectation.fallback.as_ref() =>
                    {
                        (MotionVerdict::Declared, None)
                    }
                    (wanted, played) => {
                        match self.allowed(&expectation.key, item_ref, &item.native, played) {
                            Some(reason) => (MotionVerdict::Allowed, Some(reason)),
                            None => (
                                MotionVerdict::Mismatched,
                                Some(format!(
                                    "the probe expects {}; the player played {}",
                                    wanted.clone().unwrap_or_else(|| format!(
                                        "fallback:{}",
                                        expectation.fallback.clone().unwrap_or_default()
                                    )),
                                    played.label()
                                )),
                            ),
                        }
                    }
                }
            };
            let (verdict, why) = Self::reviewed(item, verdict, why);
            push(
                &mut report,
                &id,
                Some(&expectation.key),
                item,
                expectation.effect.clone(),
                verdict,
                why,
            );
        }
        for slide in slides {
            for item in slide.items {
                if !covered.contains(&(slide.index, item.item)) {
                    push(
                        &mut report,
                        slide.id,
                        None,
                        item,
                        item.effect.clone(),
                        MotionVerdict::Mismatched,
                        Some("the probe expects no such item".to_owned()),
                    );
                }
            }
        }
        report
    }

    /// The allowlist entries of this deck no item used: `(slide, item, what it accepts)`.
    pub fn stale(&self) -> Vec<(String, String, String)> {
        let fallbacks = self
            .allow
            .entries
            .iter()
            .enumerate()
            .filter(|(position, entry)| &entry.deck == self.pair && !self.used.contains(position))
            .map(|(_, e)| {
                (
                    e.slide.clone(),
                    e.item.clone(),
                    format!("played {}", e.played),
                )
            });
        let checks = self
            .allow
            .checks
            .iter()
            .enumerate()
            .filter(|(position, entry)| {
                &entry.deck == self.pair && !self.used_checks.contains(position)
            })
            .map(|(_, e)| {
                (
                    e.slide.clone(),
                    e.item.clone(),
                    format!("check {}: {}", e.check, e.detail),
                )
            });
        fallbacks.chain(checks).collect()
    }
}

fn problem_text(item: &Item) -> String {
    item.problems
        .iter()
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join("; ")
}

fn push(
    report: &mut MotionPairReport,
    slide: &SlideId,
    key: Option<&String>,
    item: &Item,
    expected: Option<String>,
    verdict: MotionVerdict,
    why: Option<String>,
) {
    match item.item {
        ItemRef::Transition => report.transitions += 1,
        ItemRef::Step(_) => report.steps += 1,
    }
    match verdict {
        MotionVerdict::Native => report.native += 1,
        MotionVerdict::Declared | MotionVerdict::Allowed => report.fallback_allowed += 1,
        MotionVerdict::Accepted => report.accepted += 1,
        MotionVerdict::Dropped => report.dropped += 1,
        MotionVerdict::Mismatched => report.mismatched += 1,
    }
    if verdict != MotionVerdict::Native {
        report.items.push(MotionItem {
            slide: slide.clone(),
            key: key.cloned(),
            item: item.item.label(),
            native: item.native.clone(),
            expected,
            played: item.played.label(),
            verdict,
            why,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(item: ItemRef, effect: Option<&str>, played: Played) -> Item {
        Item {
            item,
            native: "test:x".into(),
            effect: effect.map(str::to_owned),
            params: None,
            fallback: "dissolve".into(),
            played,
            problems: Vec::new(),
            accepted: Vec::new(),
        }
    }

    fn deck() -> MotionDeck {
        MotionDeck {
            size: super::super::deck::DeckSize { w: 960.0, h: 540.0 },
            slides: Vec::new(),
            index: Sha256::of("x"),
        }
    }

    fn pair() -> PairKey {
        PairKey::new(PresentationId::new("motion-probe").unwrap(), Frontend::Pptx)
    }

    fn entry(slide: &str, item: &str, played: &str) -> AllowEntry {
        AllowEntry {
            deck: pair(),
            slide: slide.into(),
            item: item.into(),
            native: "test:x".into(),
            played: played.into(),
            reason: "reviewed".into(),
            tracked: "issue".into(),
        }
    }

    #[test]
    fn probe_items_play_native_their_declared_fallback_or_an_accepted_one() {
        let expect = json!({"decks": {"d.pptx": {"slideList": [
            {"index": 1, "key": "a", "transition": {"native": "pptx:fade", "effect": "fade", "fallback": "dissolve"},
             "steps": [
                {"native": "pptx:entr.2", "effect": "flyIn", "fallback": "dissolve", "dropped": false},
                {"native": "pptx:entr.99", "effect": null, "fallback": "dissolve", "dropped": false},
                {"native": "pptx:entr.3", "effect": "blinds", "fallback": "dissolve", "dropped": true},
                {"native": "pptx:entr.4", "effect": "iris", "fallback": "dissolve", "dropped": false}
             ]},
            {"index": 2, "key": "b", "transition": {"native": "pptx:ripple", "effect": "ripple", "fallback": "dissolve"}, "steps": []}
        ]}}});
        let expected = expectations(&expect, "d.pptx").unwrap();
        assert_eq!(expected.len(), 6);
        assert_eq!(expected[3].item, None);
        assert_eq!(expected[4].item, Some(ItemRef::Step(2)));
        let a = vec![
            item(
                ItemRef::Transition,
                Some("fade"),
                Played::Native("fade".into()),
            ),
            item(
                ItemRef::Step(0),
                Some("flyIn"),
                Played::Native("flyIn".into()),
            ),
            item(ItemRef::Step(1), None, Played::Fallback("dissolve".into())),
            item(
                ItemRef::Step(2),
                Some("iris"),
                Played::Fallback("dissolve".into()),
            ),
        ];
        let b = vec![item(
            ItemRef::Transition,
            None,
            Played::Fallback("dissolve".into()),
        )];
        let (ida, idb) = (SlideId::new("pp-1").unwrap(), SlideId::new("pp-2").unwrap());
        let slides = [
            SlideItems {
                index: 0,
                id: &ida,
                items: &a,
            },
            SlideItems {
                index: 1,
                id: &idb,
                items: &b,
            },
        ];
        let deck = deck();
        let pair = pair();
        let allow = Allowlist {
            checks: Vec::new(),
            version: 1,
            entries: vec![
                entry("b", "transition", "fallback:dissolve"),
                entry("gone", "step 0", "fallback:dissolve"),
            ],
        };
        let mut verdicts = Verdicts {
            pair: &pair,
            deck: &deck,
            allow: &allow,
            used: BTreeSet::new(),
            used_checks: BTreeSet::new(),
        };
        let report = verdicts.probe(&slides, &expected);
        assert_eq!(
            (
                report.native,
                report.fallback_allowed,
                report.dropped,
                report.mismatched
            ),
            (2, 2, 1, 1)
        );
        // The iris step falls back without a review; ripple's fallback is reviewed.
        let mismatched: Vec<&str> = report
            .items
            .iter()
            .filter(|i| i.verdict == MotionVerdict::Mismatched)
            .map(|i| i.item.as_str())
            .collect();
        assert_eq!(mismatched, ["step 2"]);
        // An entry nothing uses is stale.
        assert_eq!(verdicts.stale().len(), 1);
        assert_eq!(verdicts.stale()[0].0, "gone");
    }

    #[test]
    fn a_failed_check_mismatches_unless_a_review_accepts_it() {
        let broken = || {
            let mut broken = item(
                ItemRef::Step(0),
                Some("flyIn"),
                Played::Native("flyIn".into()),
            );
            broken.problems.push(Problem::new(
                "end-state",
                "its stage's end state differs from the still render",
            ));
            broken
        };
        let id = SlideId::new("kn-1").unwrap();
        let deck = deck();
        let pair = pair();
        let accepted_as = |check: &str, detail: &str| AcceptedCheck {
            deck: pair.clone(),
            slide: "kn-1".into(),
            item: "step 0".into(),
            native: "test:x".into(),
            check: check.into(),
            detail: detail.into(),
            reason: "holds its end".into(),
            tracked: "player".into(),
        };
        let accepted =
            |check: &str| accepted_as(check, "its stage's end state differs from the still render");
        let allow = Allowlist {
            version: 1,
            entries: Vec::new(),
            checks: vec![accepted("reveal")],
        };
        let mut verdicts = Verdicts {
            pair: &pair,
            deck: &deck,
            allow: &allow,
            used: BTreeSet::new(),
            used_checks: BTreeSet::new(),
        };
        let mut items = [
            broken(),
            item(ItemRef::Step(1), None, Played::Fallback("dissolve".into())),
        ];
        verdicts.accept("kn-1", &mut items);
        let report = verdicts.corpus(&[SlideItems {
            index: 0,
            id: &id,
            items: &items,
        }]);
        assert_eq!(
            (report.native, report.fallback_allowed, report.mismatched),
            (0, 1, 1)
        );
        // The review accepts another check than the one that failed: it is stale.
        assert_eq!(verdicts.stale().len(), 1);
        // A review of the failed check accepts the item, with its reason.
        let allow = Allowlist {
            version: 1,
            entries: Vec::new(),
            checks: vec![accepted("end-state")],
        };
        let mut verdicts = Verdicts {
            pair: &pair,
            deck: &deck,
            allow: &allow,
            used: BTreeSet::new(),
            used_checks: BTreeSet::new(),
        };
        let mut items = [broken()];
        verdicts.accept("kn-1", &mut items);
        let report = verdicts.corpus(&[SlideItems {
            index: 0,
            id: &id,
            items: &items,
        }]);
        // It played natively: an accepted defect, not a fallback.
        assert_eq!(
            (report.fallback_allowed, report.accepted, report.mismatched),
            (0, 1, 0)
        );
        assert_eq!(report.items[0].verdict, MotionVerdict::Accepted);
        assert_eq!(report.items[0].why.as_deref(), Some("holds its end"));
        assert!(verdicts.stale().is_empty());
        // A review of another failure of the same check, or of another item's native id, accepts nothing.
        for check in [
            accepted_as("end-state", "a.3 opacity ends at 0.6, settles at 1"),
            AcceptedCheck {
                native: "test:y".into(),
                ..accepted("end-state")
            },
        ] {
            let allow = Allowlist {
                version: 1,
                entries: Vec::new(),
                checks: vec![check],
            };
            let mut verdicts = Verdicts {
                pair: &pair,
                deck: &deck,
                allow: &allow,
                used: BTreeSet::new(),
                used_checks: BTreeSet::new(),
            };
            let mut items = [broken()];
            verdicts.accept("kn-1", &mut items);
            let report = verdicts.corpus(&[SlideItems {
                index: 0,
                id: &id,
                items: &items,
            }]);
            assert_eq!(report.mismatched, 1);
            assert_eq!(verdicts.stale().len(), 1);
        }
        // A deck-wide review (`*`) accepts the failure on any slide.
        let allow = Allowlist {
            version: 1,
            entries: Vec::new(),
            checks: vec![AcceptedCheck {
                slide: "*".into(),
                ..accepted("end-state")
            }],
        };
        let mut verdicts = Verdicts {
            pair: &pair,
            deck: &deck,
            allow: &allow,
            used: BTreeSet::new(),
            used_checks: BTreeSet::new(),
        };
        let mut items = [broken()];
        verdicts.accept("kn-7", &mut items);
        assert!(items[0].problems.is_empty() && !items[0].accepted.is_empty());
        // A step that animated nothing, where a review accepts the missing animations.
        let allow = Allowlist {
            version: 1,
            entries: Vec::new(),
            checks: vec![accepted_as(
                "missing",
                "missing: expected animations, played none",
            )],
        };
        let mut verdicts = Verdicts {
            pair: &pair,
            deck: &deck,
            allow: &allow,
            used: BTreeSet::new(),
            used_checks: BTreeSet::new(),
        };
        let mut nothing = item(ItemRef::Step(0), Some("fade"), Played::Nothing);
        nothing.problems.push(Problem::new(
            "missing",
            "missing: expected animations, played none",
        ));
        let mut items = [nothing];
        verdicts.accept("kn-1", &mut items);
        let report = verdicts.corpus(&[SlideItems {
            index: 0,
            id: &id,
            items: &items,
        }]);
        assert_eq!((report.accepted, report.mismatched), (1, 0));
        // A deck that names an effect the player falls back from needs a review.
        let fell = [item(
            ItemRef::Step(0),
            Some("sparkle"),
            Played::Fallback("dissolve".into()),
        )];
        let report = verdicts.corpus(&[SlideItems {
            index: 0,
            id: &id,
            items: &fell,
        }]);
        assert_eq!(report.mismatched, 1);
    }

    #[test]
    fn a_fallback_entry_names_the_native_id_it_accepts() {
        let expect = json!({"decks": {"d.pptx": {"slideList": [
            {"index": 1, "key": "b", "transition": {"native": "test:x", "effect": "ripple", "fallback": "dissolve"}, "steps": []}
        ]}}});
        let expected = expectations(&expect, "d.pptx").unwrap();
        let fell = vec![item(
            ItemRef::Transition,
            None,
            Played::Fallback("dissolve".into()),
        )];
        let id = SlideId::new("pp-2").unwrap();
        let slides = [SlideItems {
            index: 0,
            id: &id,
            items: &fell,
        }];
        let (deck, pair) = (deck(), pair());
        for (native, mismatched) in [("test:x", 0), ("test:other", 1)] {
            let allow = Allowlist {
                version: 1,
                entries: vec![AllowEntry {
                    native: native.into(),
                    ..entry("b", "transition", "fallback:dissolve")
                }],
                checks: Vec::new(),
            };
            let mut verdicts = Verdicts {
                pair: &pair,
                deck: &deck,
                allow: &allow,
                used: BTreeSet::new(),
                used_checks: BTreeSet::new(),
            };
            let report = verdicts.probe(&slides, &expected);
            assert_eq!(report.mismatched, mismatched, "{native}");
            assert_eq!(verdicts.stale().len(), mismatched as usize);
        }
    }

    #[test]
    fn params_compare_in_one_spelling_and_differ_where_the_probe_reads_otherwise() {
        let same = |want: Value, have: Value| param_differences(&want, Some(&have)).is_empty();
        assert!(same(json!({"axis": "vertical"}), json!({"axis": "vert"})));
        assert!(same(
            json!({"to": "#E0442E"}),
            json!({"to": "#e0442e", "property": "fill"})
        ));
        assert!(same(json!({"from": 12}), json!({"from": "12"})));
        assert!(same(
            json!({"scale": [1.5, 1.5]}),
            json!({"scale": {"x": 1.5, "y": 1.5}, "hold": true})
        ));
        assert!(same(
            json!({"d": "M 0 0 L 128 -72.5 Z"}),
            json!({"d": [["M", 0, 0], ["L", 128, -72.5], ["Z"]]})
        ));
        assert!(!same(json!({"axis": "vertical"}), json!({"axis": "horz"})));
        assert!(!same(json!({"dir": "l"}), json!({"dir": "r"})));
        assert!(!same(json!({"dir": "l"}), json!({})));
        assert_eq!(
            param_differences(
                &json!({"inOut": "out"}),
                Some(&json!({"inOut": "in", "fade": false}))
            ),
            [r#"inOut: the probe expects "out", the deck has "in""#]
        );
        // A step that plays the probe's effect with another dir gets a params problem; another effect gets none.
        let expect = json!({"decks": {"d.pptx": {"slideList": [
            {"index": 1, "key": "a", "transition": null, "steps": [
                {"native": "pptx:entr.2", "effect": "flyIn", "params": {"dir": "l"}, "fallback": "dissolve"},
                {"native": "pptx:entr.3", "effect": "blinds", "params": {"orient": "horz"}, "fallback": "dissolve"}
            ]}
        ]}}});
        let expected = expectations(&expect, "d.pptx").unwrap();
        let mut items = [
            Item {
                params: Some(json!({"dir": "r", "distance": "offSlide"})),
                ..item(
                    ItemRef::Step(0),
                    Some("flyIn"),
                    Played::Native("flyIn".into()),
                )
            },
            Item {
                params: Some(json!({"orient": "vert"})),
                ..item(
                    ItemRef::Step(1),
                    Some("fade"),
                    Played::Native("fade".into()),
                )
            },
        ];
        params_problems(&expected, 0, &mut items);
        assert_eq!(items[0].problems.len(), 1);
        assert_eq!(items[0].problems[0].check, "params");
        assert_eq!(
            items[0].problems[0].detail,
            r#"dir: the probe expects "l", the deck has "r""#
        );
        assert!(items[1].problems.is_empty());
    }

    #[test]
    fn the_committed_probes_and_allowlist_read() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let probes = load_probes(&repo.join(PROBES)).unwrap();
        assert!(!probes.probes.is_empty());
        for probe in &probes.probes {
            assert_eq!(
                crate::hash::sha256_file(&repo.join(&probe.file)).unwrap(),
                probe.sha256.as_str()
            );
            let expect: Value =
                serde_json::from_str(&std::fs::read_to_string(repo.join(&probe.expect)).unwrap())
                    .unwrap();
            assert!(!expectations(&expect, &probe.deck).unwrap().is_empty());
        }
        let allow = load_allowlist(&repo.join(FALLBACK_ALLOW)).unwrap();
        assert!(
            allow
                .entries
                .iter()
                .all(|e| !e.reason.is_empty() && !e.tracked.is_empty())
        );
        // An accepted check names the item's native id and the failure exactly.
        assert!(allow.checks.iter().all(|c| !c.native.is_empty()
            && !c.detail.is_empty()
            && !c.reason.is_empty()
            && !c.tracked.is_empty()));
    }
}
