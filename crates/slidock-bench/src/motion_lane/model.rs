//! What a slide's timeline says should play, worked out from the deck data alone,
//! independently of the player: the click groups, each step's start inside its group and how long its animations
//! last, and which build units show after each group.
//!
//! - Group 0 is the run of steps before the first `click` step; every `click` step opens a group, and the with- and
//!   after-previous steps after it join it.
//! - A step starts at its absolute `offsetMs` when the compiler gave one, else with the previous step's start (with
//!   previous) or at its end (after previous), plus its delay. Its end counts its repeats (one round of an endless
//!   one), its auto-reverse and its stagger.
//! - A unit (an element, or a paragraph of one) whose first step is an entrance starts hidden; an entrance shows it,
//!   an exit or a step's `after: hide` hides it.
//! - A tier-2 set ([`switches`]) switches at its begin, as an appear does: it does not move, and a step whose tracks
//!   are all sets ([`only_switches`]) does not move at all.

use std::collections::BTreeMap;

use serde_json::Value;

use super::deck::{DeckStep, number};

/// The longest time the deck format holds (slidock-ir `Ms::MAX`).
pub const MS_MAX: f64 = 3_600_000.0;
/// The most repeats a step plays (slidock-ir `REPEAT_COUNT`).
pub const REPEAT_MAX: f64 = 1000.0;

/// A time in the deck: finite, not negative, at most [`MS_MAX`]; anything else reads as 0.
pub fn ms(value: &Value) -> f64 {
    match number(value) {
        Some(ms) if ms.is_finite() && (0.0..=MS_MAX).contains(&ms) => ms,
        _ => 0.0,
    }
}

/// A step's place and timing inside its click group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scheduled {
    pub group: u32,
    pub start: f64,
    /// Its end: every repeat (one round of an endless repeat), auto-reverse and stagger.
    pub end: f64,
    /// One iteration.
    pub duration: f64,
}

/// The click group of each step.
pub fn groups(steps: &[DeckStep]) -> Vec<u32> {
    let mut group = 0u32;
    steps
        .iter()
        .map(|step| {
            if step.trigger == "click" {
                group += 1;
            }
            group
        })
        .collect()
}

/// How many times one round plays, and whether the step repeats without end.
fn rounds(step: &DeckStep) -> (f64, bool) {
    match &step.repeat {
        None => (1.0, false),
        Some(Value::String(until)) if until == "untilNextClick" || until == "untilEndOfSlide" => {
            (1.0, true)
        }
        Some(Value::Object(object)) if object.len() == 1 => {
            match object.get("count").and_then(Value::as_f64) {
                Some(count) if count.is_finite() && count > 0.0 && count <= REPEAT_MAX => {
                    (count, false)
                }
                _ => (1.0, false),
            }
        }
        Some(_) => (1.0, false),
    }
}

/// The stagger between consecutive units, in ms, and the widest slot `units` units reach in its order.
fn stagger(step: &DeckStep, duration: f64, units: u32) -> f64 {
    let Some(Value::Object(stagger)) = &step.stagger else {
        return 0.0;
    };
    let per_unit = match stagger.get("perUnit") {
        Some(Value::Object(amount)) if amount.len() == 1 => {
            match (
                amount.get("ms").and_then(Value::as_f64),
                amount.get("fraction").and_then(Value::as_f64),
            ) {
                (Some(ms), _) if ms.is_finite() && (0.0..=MS_MAX).contains(&ms) => ms,
                (_, Some(fraction)) if fraction.is_finite() && (0.0..=1.0).contains(&fraction) => {
                    fraction * duration
                }
                _ => return 0.0,
            }
        }
        _ => return 0.0,
    };
    let n = f64::from(units.max(1));
    let widest = match stagger.get("order") {
        Some(Value::String(order)) if order == "fromCenter" || order == "fromEdges" => {
            ((n - 1.0) / 2.0).floor()
        }
        Some(Value::String(order)) if order == "forward" || order == "backward" => n - 1.0,
        Some(Value::Object(random)) if random.contains_key("random") => n - 1.0,
        _ => return 0.0,
    };
    widest * per_unit
}

/// Each step's group, start and end. `units(i)` is how many units step `i` delivered (1 when built whole).
pub fn schedule(steps: &[DeckStep], units: impl Fn(usize) -> u32) -> Vec<Scheduled> {
    let groups = groups(steps);
    let mut out: Vec<Scheduled> = Vec::with_capacity(steps.len());
    for (index, step) in steps.iter().enumerate() {
        let group = groups.get(index).copied().unwrap_or_default();
        let previous = out.last().filter(|previous| previous.group == group);
        let delay = ms(&step.delay_ms);
        let mut duration = ms(&step.duration_ms);
        let start = match &step.offset_ms {
            Some(offset) => ms(offset),
            None => match step.trigger.as_str() {
                "withPrevious" => previous.map_or(0.0, |p| p.start) + delay,
                "afterPrevious" => previous.map_or(0.0, |p| p.end) + delay,
                _ => delay,
            },
        };
        let (count, endless) = rounds(step);
        let reverse = if step.auto_reverse == Some(true) {
            2.0
        } else {
            1.0
        };
        let n = units(index).max(1);
        let span = if step.effect.as_deref() == Some("typewriter") {
            let per_unit = duration / f64::from(n);
            duration = 0.0;
            f64::from(n - 1) * per_unit
        } else {
            stagger(step, duration, n)
        };
        let active = if endless {
            duration * reverse
        } else {
            duration * count * reverse
        };
        out.push(Scheduled {
            group,
            start,
            end: start + active + span,
            duration,
        });
    }
    out
}

/// How long each of a step's animations lasts: 0 for a switch (an appear, a typewriter's units, a media cue).
pub fn animation_ms(step: &DeckStep, scheduled: &Scheduled, native: bool) -> f64 {
    let plays = if native {
        step.effect.as_deref()
    } else {
        Some(step.fallback.as_str())
    };
    if step.kind == "media" || matches!(plays, Some("appear" | "typewriter")) {
        0.0
    } else {
        scheduled.duration
    }
}

/// Whether a tier-2 track is a set (PPTX `p:set`, ODF `anim:set`, the IR's `SetTrack`): a switch of one property at
/// its begin (`{"property": "set", "beginMs", "durationMs", "to": {"property", "value"}}`), not a change over its span.
/// Any other track is held to moving.
pub fn is_set(track: &Value) -> bool {
    track.get("property").and_then(Value::as_str) == Some("set")
}

/// The tier-2 tracks of a step the player plays natively (`behaviors`); `None` for any other step.
fn tracks(step: &DeckStep, native: bool) -> Option<&[Value]> {
    if !native || step.effect.as_deref() != Some("behaviors") {
        return None;
    }
    match &step.params {
        Some(Value::Array(tracks)) if !tracks.is_empty() => Some(tracks),
        _ => None,
    }
}

/// Whether a step only switches: it plays its own tier-2 tracks, and every one is a set. `moves` does not apply to it.
pub fn only_switches(step: &DeckStep, native: bool) -> bool {
    tracks(step, native).is_some_and(|tracks| tracks.iter().all(is_set))
}

/// The pose key the player animates a tier-2 property on (`packages/player/src/effects/properties.ts`): `opacity`,
/// and the registered `--sd-anim-*` property of a colour or font.
pub fn property_key(property: &str) -> Option<&'static str> {
    Some(match property {
        "opacity" => "opacity",
        "text" => "--sd-anim-color",
        "fill" => "--sd-anim-fill",
        "line" => "--sd-anim-line",
        "bold" => "--sd-anim-bold",
        "italic" => "--sd-anim-italic",
        "underline" => "--sd-anim-underline",
        _ => return None,
    })
}

/// The value a switch shows once switched, as its pose key computes it.
#[derive(Clone, Debug, PartialEq)]
pub enum Switched {
    /// A keyword: `bold` / `normal`, `italic` / `normal`, `underline` / `none`.
    Word(&'static str),
    /// A colour, `#rrggbb` in the deck; computed as `rgb(r, g, b)`.
    Color([u8; 3]),
    /// An opacity.
    Number(f64),
}

impl Switched {
    /// Whether a computed value shows this: the same keyword, colour channels within 1, a number within a thousandth.
    pub fn shows(&self, value: &str) -> bool {
        let value = value.trim();
        match self {
            Switched::Word(word) => value == *word,
            Switched::Color(want) => color(value)
                .is_some_and(|seen| seen.iter().zip(want).all(|(a, b)| a.abs_diff(*b) <= 1)),
            Switched::Number(want) => value
                .parse::<f64>()
                .is_ok_and(|seen| (seen - want).abs() <= 1e-3),
        }
    }
}

impl std::fmt::Display for Switched {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Switched::Word(word) => f.write_str(word),
            Switched::Color([r, g, b]) => write!(f, "#{r:02x}{g:02x}{b:02x}"),
            Switched::Number(n) => write!(f, "{n}"),
        }
    }
}

/// An opaque colour written `#rrggbb`, `rgb(r, g, b)`, `rgb(r g b)` or `rgba(r, g, b, 1)`.
fn color(value: &str) -> Option<[u8; 3]> {
    if let Some(hex) = value.strip_prefix('#') {
        if hex.len() != 6 || !hex.is_ascii() {
            return None;
        }
        let channel = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        return Some([channel(0)?, channel(2)?, channel(4)?]);
    }
    let inner = value
        .strip_prefix("rgba(")
        .or_else(|| value.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let parts: Vec<&str> = inner
        .split([',', ' ', '/'])
        .filter(|part| !part.is_empty())
        .collect();
    let channel = |part: &&str| -> Option<u8> {
        let n: f64 = part.parse().ok()?;
        (n.is_finite() && (0.0..=255.0).contains(&n)).then(|| n.round() as u8)
    };
    match parts.as_slice() {
        [r, g, b] => Some([channel(r)?, channel(g)?, channel(b)?]),
        [r, g, b, a] if a.parse::<f64>().is_ok_and(|a| (a - 1.0).abs() <= 1e-3) => {
            Some([channel(r)?, channel(g)?, channel(b)?])
        }
        _ => None,
    }
}

/// One switch of a step: from its begin (ms from the step's start, per unit), the pose `key` shows `to`.
#[derive(Clone, Debug, PartialEq)]
pub struct Switch {
    pub begin: f64,
    pub key: &'static str,
    pub to: Switched,
}

/// A set track as a [`Switch`]; `Err` names a set whose begin, property or value the lane cannot read (it fails
/// closed: a mismatch, never a pass).
fn switch_of(track: &Value) -> Result<Switch, String> {
    let unknown = || format!("an unknown switch {track}");
    let begin = track.get("beginMs").map(ms).ok_or_else(unknown)?;
    let to = track.get("to").ok_or_else(unknown)?;
    let property = to
        .get("property")
        .and_then(Value::as_str)
        .ok_or_else(unknown)?;
    let key = property_key(property).ok_or_else(unknown)?;
    let value = to.get("value").ok_or_else(unknown)?;
    let word =
        |on: &'static str, off: &'static str| value.as_bool().map(|b| if b { on } else { off });
    let to = match property {
        "opacity" => value
            .as_f64()
            .filter(|n| (0.0..=1.0).contains(n))
            .map(Switched::Number),
        "text" | "fill" | "line" => value
            .as_str()
            .filter(|hex| hex.starts_with('#'))
            .and_then(color)
            .map(Switched::Color),
        "bold" => word("bold", "normal").map(Switched::Word),
        "italic" => word("italic", "normal").map(Switched::Word),
        "underline" => word("underline", "none").map(Switched::Word),
        _ => None,
    }
    .ok_or_else(unknown)?;
    Ok(Switch { begin, key, to })
}

/// The switches of a native tier-2 step that the lane checks as switches: its sets of a property no other track of
/// the step animates (a property a track moves is held to moving). `None` when the step has none; `Err` names a set
/// it cannot read.
pub fn switches(step: &DeckStep, native: bool) -> Option<Result<Vec<Switch>, String>> {
    let tracks = tracks(step, native)?;
    let moved: Vec<&str> = tracks
        .iter()
        .filter(|track| !is_set(track))
        .filter_map(|track| track.get("property").and_then(Value::as_str))
        .collect();
    let mut out = Vec::new();
    for track in tracks.iter().filter(|track| is_set(track)) {
        let property = track
            .get("to")
            .and_then(|to| to.get("property"))
            .and_then(Value::as_str);
        if property.is_some_and(|p| moved.contains(&p)) {
            continue;
        }
        match switch_of(track) {
            Ok(switch) => out.push(switch),
            Err(why) => return Some(Err(why)),
        }
    }
    (!out.is_empty()).then_some(Ok(out))
}

/// Where one of a step's own tier-2 tracks plays inside it, as the player times its animation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackSpan {
    /// From the step's start.
    pub begin: f64,
    pub length: f64,
    /// A set: its animation may also last 0 ms (it switches at its begin and holds).
    pub set: bool,
}

/// Where a step's own tier-2 tracks (`behaviors`, played natively) play inside it. `None` for any other step, which
/// plays over its own duration.
pub fn track_spans(step: &DeckStep, native: bool) -> Option<Vec<TrackSpan>> {
    let spans: Vec<TrackSpan> = tracks(step, native)?
        .iter()
        .filter_map(|track| {
            Some(TrackSpan {
                begin: track.get("beginMs").map(ms)?,
                length: track.get("durationMs").map(ms)?,
                set: is_set(track),
            })
        })
        .collect();
    (!spans.is_empty()).then_some(spans)
}

/// Whether the player animates anything for a step: its effect, or a fallback that has frames (an entrance or exit
/// that appears or dissolves), or an exit's hide.
pub fn animates(step: &DeckStep, native: bool) -> bool {
    if native || step.kind == "media" {
        return true;
    }
    match step.kind.as_str() {
        "entrance" => step.fallback != "none",
        "exit" => true,
        _ => matches!(&step.after, Some(Value::String(after)) if after == "hide"),
    }
}

/// A build unit: an element, or one paragraph of it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Unit {
    pub element: String,
    pub paragraph: Option<u32>,
}

/// Paragraphs one step addresses at most; a wider range is the whole element (the player's bound).
const PARAGRAPHS_PER_STEP: u32 = 10_000;

/// The units a step builds; none for a background build, which the lane does not measure.
pub fn step_units(step: &DeckStep) -> Vec<Unit> {
    let target = &step.target;
    if target.is_background() {
        return Vec::new();
    }
    match target.paragraphs {
        Some((first, last)) if last >= first && last - first < PARAGRAPHS_PER_STEP => (first
            ..=last)
            .map(|p| Unit {
                element: target.element.clone(),
                paragraph: Some(p),
            })
            .collect(),
        _ => vec![Unit {
            element: target.element.clone(),
            paragraph: None,
        }],
    }
}

fn hides_after(step: &DeckStep) -> bool {
    matches!(&step.after, Some(Value::String(after)) if after == "hide")
}

/// Whether each unit the timeline builds shows once groups `0..=group` have played. A paragraph shows only when its
/// element does as well.
pub fn reveal(steps: &[DeckStep], group: u32) -> BTreeMap<Unit, bool> {
    let groups = groups(steps);
    let mut own: BTreeMap<Unit, bool> = BTreeMap::new();
    for step in steps {
        for unit in step_units(step) {
            own.entry(unit).or_insert(step.kind != "entrance");
        }
    }
    for (step, g) in steps.iter().zip(&groups) {
        if *g > group {
            break;
        }
        for unit in step_units(step) {
            let shows = match step.kind.as_str() {
                "entrance" => true,
                "exit" => false,
                _ => own.get(&unit).copied().unwrap_or(true),
            };
            own.insert(unit, shows && !hides_after(step));
        }
    }
    own.iter()
        .map(|(unit, shows)| {
            let element = Unit {
                element: unit.element.clone(),
                paragraph: None,
            };
            let parent = match unit.paragraph {
                Some(_) => own.get(&element).copied().unwrap_or(true),
                None => true,
            };
            (unit.clone(), *shows && parent)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion_lane::deck::Target;
    use serde_json::json;

    pub fn step(trigger: &str, kind: &str, element: &str, delay: f64, duration: f64) -> DeckStep {
        DeckStep {
            target: Target {
                element: element.to_owned(),
                paragraphs: None,
                background: None,
            },
            trigger: trigger.to_owned(),
            kind: kind.to_owned(),
            delivery: Some("all".into()),
            delay_ms: json!(delay),
            duration_ms: json!(duration),
            native: format!("test:{kind}"),
            fallback: "dissolve".into(),
            effect: Some("fade".into()),
            params: None,
            offset_ms: None,
            repeat: None,
            auto_reverse: None,
            stagger: None,
            after: None,
        }
    }

    #[test]
    fn steps_chain_inside_their_click_group() {
        let mut steps = vec![
            step("afterPrevious", "entrance", "a", 100.0, 500.0),
            step("click", "entrance", "b", 0.0, 500.0),
            step("withPrevious", "entrance", "c", 200.0, 300.0),
            step("afterPrevious", "emphasis", "c", 50.0, 400.0),
            step("click", "exit", "a", 0.0, 1000.0),
        ];
        steps[3].repeat = Some(json!({"count": 2}));
        steps[3].auto_reverse = Some(true);
        let s = schedule(&steps, |_| 1);
        assert_eq!(
            s.iter().map(|s| s.group).collect::<Vec<_>>(),
            [0, 1, 1, 1, 2]
        );
        assert_eq!(s[0].start, 100.0);
        assert_eq!(s[1].start, 0.0);
        assert_eq!(s[2].start, 200.0);
        assert_eq!(s[2].end, 500.0);
        // After the previous step's end, plus its delay; two rounds, each forward then back.
        assert_eq!(s[3].start, 550.0);
        assert_eq!(s[3].end, 550.0 + 400.0 * 4.0);
        // A click step starts its own group at its delay.
        assert_eq!(s[4].start, 0.0);
    }

    #[test]
    fn an_offset_wins_and_bad_times_read_as_zero() {
        let mut steps = vec![
            step("click", "entrance", "a", 0.0, 500.0),
            step("afterPrevious", "entrance", "b", 0.0, 500.0),
        ];
        steps[1].offset_ms = Some(json!(1250));
        let s = schedule(&steps, |_| 1);
        assert_eq!(s[1].start, 1250.0);
        assert_eq!(ms(&json!(-1)), 0.0);
        assert_eq!(ms(&json!(3_600_001)), 0.0);
        assert_eq!(ms(&json!("x")), 0.0);
        assert_eq!(ms(&json!(3_600_000)), MS_MAX);
    }

    #[test]
    fn a_stagger_widens_the_end_by_its_widest_slot() {
        let mut steps = vec![
            step("click", "entrance", "a", 0.0, 500.0),
            step("afterPrevious", "entrance", "b", 0.0, 100.0),
        ];
        steps[0].stagger = Some(json!({"perUnit": {"ms": 50}, "order": "forward"}));
        let s = schedule(&steps, |i| if i == 0 { 5 } else { 1 });
        assert_eq!(s[0].end, 500.0 + 4.0 * 50.0);
        assert_eq!(s[1].start, 700.0);
        steps[0].stagger = Some(json!({"perUnit": {"fraction": 0.5}, "order": "fromCenter"}));
        let s = schedule(&steps, |i| if i == 0 { 5 } else { 1 });
        assert_eq!(s[0].end, 500.0 + 2.0 * 250.0);
        // A typewriter types its units one after another over its duration.
        steps[0].stagger = None;
        steps[0].effect = Some("typewriter".into());
        let s = schedule(&steps, |i| if i == 0 { 4 } else { 1 });
        assert_eq!(s[0].duration, 0.0);
        assert_eq!(s[0].end, 375.0);
    }

    #[test]
    fn an_endless_repeat_counts_one_round() {
        let mut steps = vec![step("click", "emphasis", "a", 0.0, 400.0)];
        steps[0].repeat = Some(json!("untilNextClick"));
        assert_eq!(schedule(&steps, |_| 1)[0].end, 400.0);
        steps[0].repeat = Some(json!({"count": 1e9}));
        assert_eq!(schedule(&steps, |_| 1)[0].end, 400.0);
    }

    #[test]
    fn switches_last_no_time() {
        let mut appear = step("click", "entrance", "a", 0.0, 500.0);
        appear.effect = Some("appear".into());
        let s = schedule(std::slice::from_ref(&appear), |_| 1)[0];
        assert_eq!(animation_ms(&appear, &s, true), 0.0);
        let mut fade = step("click", "entrance", "a", 0.0, 500.0);
        assert_eq!(animation_ms(&fade, &s, true), 500.0);
        fade.fallback = "appear".into();
        assert_eq!(animation_ms(&fade, &s, false), 0.0);
        let media = step("click", "media", "v", 0.0, 1000.0);
        assert_eq!(animation_ms(&media, &s, true), 0.0);
    }

    #[test]
    fn only_entrances_and_exits_fall_back_to_frames() {
        let mut emphasis = step("click", "emphasis", "a", 0.0, 500.0);
        assert!(animates(&emphasis, true));
        assert!(!animates(&emphasis, false));
        emphasis.after = Some(json!("hide"));
        assert!(animates(&emphasis, false));
        let mut entrance = step("click", "entrance", "a", 0.0, 500.0);
        assert!(animates(&entrance, false));
        entrance.fallback = "none".into();
        assert!(!animates(&entrance, false));
        let mut exit = step("click", "exit", "a", 0.0, 500.0);
        exit.fallback = "none".into();
        assert!(animates(&exit, false));
    }

    #[test]
    fn a_step_only_switches_when_every_track_it_plays_is_a_set() {
        let set = json!({"property": "set", "beginMs": 0, "durationMs": 500,
            "to": {"property": "bold", "value": true}});
        // A track of equal frames is a change over its span that changes nothing: it moves, and fails `moves`.
        let constant = json!({"property": "bold", "beginMs": 0, "durationMs": 500,
            "frames": [{"offset": 0, "value": true}, {"offset": 1, "value": true}]});
        let mut emphasis = step("click", "emphasis", "a", 0.0, 500.0);
        emphasis.effect = Some("behaviors".into());
        emphasis.params = Some(json!([set]));
        assert!(only_switches(&emphasis, true));
        // Played by its fallback, it plays no set.
        assert!(!only_switches(&emphasis, false));
        emphasis.params = Some(json!([set, set]));
        assert!(only_switches(&emphasis, true));
        emphasis.params = Some(json!([constant]));
        assert!(!only_switches(&emphasis, true));
        // A set beside a track that moves: the step moves.
        emphasis.params = Some(json!([set, constant]));
        assert!(!only_switches(&emphasis, true));
        emphasis.params = Some(json!([]));
        assert!(!only_switches(&emphasis, true));
        emphasis.params = Some(json!({"property": "set"}));
        assert!(!only_switches(&emphasis, true));
        // Only tier 2 carries tracks.
        emphasis.effect = Some("colorChange".into());
        emphasis.params = Some(json!([set]));
        assert!(!only_switches(&emphasis, true));
    }

    #[test]
    fn a_set_switches_its_property_key_to_its_value() {
        let set = |property: &str, value: Value| {
            json!({"property": "set", "beginMs": 250, "durationMs": 0,
                "to": {"property": property, "value": value}})
        };
        let mut emphasis = step("click", "emphasis", "a", 0.0, 500.0);
        emphasis.effect = Some("behaviors".into());
        emphasis.params = Some(json!([
            set("bold", json!(true)),
            set("underline", json!(false)),
            set("text", json!("#E0442E")),
            set("opacity", json!(0.5)),
        ]));
        let read = switches(&emphasis, true).and_then(Result::ok).unwrap();
        let seen: Vec<(f64, &str, String)> = read
            .iter()
            .map(|s| (s.begin, s.key, s.to.to_string()))
            .collect();
        assert_eq!(
            seen,
            [
                (250.0, "--sd-anim-bold", "bold".to_owned()),
                (250.0, "--sd-anim-underline", "none".to_owned()),
                (250.0, "--sd-anim-color", "#e0442e".to_owned()),
                (250.0, "opacity", "0.5".to_owned()),
            ]
        );
        assert!(read[2].to.shows("rgb(224, 68, 46)"));
        assert!(read[2].to.shows("rgb(225 68 46 / 1)"));
        assert!(!read[2].to.shows("rgb(0, 0, 0)"));
        assert!(!read[2].to.shows("rgba(224, 68, 46, 0.5)"));
        assert!(read[3].to.shows("0.5"));
        assert!(!read[3].to.shows("1"));
        assert!(!read[0].to.shows("normal"));
        // A set beside a track of the same property is held to moving with it; one of another property is a switch.
        emphasis.params = Some(json!([
            set("bold", json!(true)),
            set("text", json!("#e0442e")),
            {"property": "text", "beginMs": 0, "durationMs": 500,
                "frames": [{"offset": 0, "value": "#000000"}, {"offset": 1, "value": "#e0442e"}]},
        ]));
        let keys: Vec<&str> = switches(&emphasis, true)
            .and_then(Result::ok)
            .unwrap()
            .iter()
            .map(|s| s.key)
            .collect();
        assert_eq!(keys, ["--sd-anim-bold"]);
        // Fails closed on what it cannot read: an unknown property, a value of the wrong kind, a colour by name.
        for bad in [
            set("font", json!("Arial")),
            set("bold", json!("yes")),
            set("opacity", json!(2)),
            set("fill", json!("red")),
            json!({"property": "set", "durationMs": 0, "to": {"property": "bold", "value": true}}),
        ] {
            emphasis.params = Some(json!([bad]));
            assert!(
                matches!(switches(&emphasis, true), Some(Err(why)) if why.starts_with("an unknown switch")),
                "{bad}"
            );
        }
        // No sets, or not played natively: nothing to check as a switch.
        emphasis.params = Some(
            json!([{"property": "opacity", "beginMs": 0, "durationMs": 500,
            "frames": [{"offset": 0, "value": 0}, {"offset": 1, "value": 1}]}]),
        );
        assert_eq!(switches(&emphasis, true), None);
        emphasis.params = Some(json!([set("bold", json!(true))]));
        assert_eq!(switches(&emphasis, false), None);
    }

    #[test]
    fn units_show_from_their_entrance_and_hide_from_their_exit() {
        let mut steps = vec![
            step("click", "entrance", "a", 0.0, 500.0),
            step("click", "emphasis", "b", 0.0, 500.0),
            step("click", "exit", "a", 0.0, 500.0),
            step("click", "entrance", "c", 0.0, 500.0),
        ];
        steps[1].after = Some(json!("hide"));
        steps[3].target.paragraphs = Some((1, 2));
        let a = Unit {
            element: "a".into(),
            paragraph: None,
        };
        let b = Unit {
            element: "b".into(),
            paragraph: None,
        };
        let c1 = Unit {
            element: "c".into(),
            paragraph: Some(1),
        };
        let at = |group| reveal(&steps, group);
        assert_eq!((at(0)[&a], at(0)[&b], at(0)[&c1]), (false, true, false));
        assert_eq!((at(1)[&a], at(1)[&b]), (true, true));
        assert_eq!((at(2)[&a], at(2)[&b]), (true, false));
        assert_eq!((at(3)[&a], at(3)[&c1]), (false, false));
        assert!(at(4)[&c1]);
        // A paragraph shows only while its element does.
        steps.push(step("click", "exit", "c", 0.0, 500.0));
        assert!(!reveal(&steps, 5)[&c1]);
        // A background build is not measured.
        steps[0].target.background = Some(true);
        assert!(step_units(&steps[0]).is_empty());
    }
}
