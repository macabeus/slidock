//! A walked slide against its deck data: the timeline that played against the one the deck says ([`super::model`]),
//! the transition into it, and each stage's frames. The result is the slide's [`TimelineFidelity`], its stage and
//! transition rows, and, per step and transition, what played and what went wrong ([`Item`]), which the verdicts
//! ([`super::verdict`]) read.
//!
//! Animations are attributed to steps by target: the player plays a group's steps in timeline order, each on its own
//! units, so the creation order walks the steps. Two steps of one group on the same target that start together cannot
//! be told apart; the later one counts as played with the earlier one's animations.
//!
//! Besides what played when, each step and transition that animates is held to how it moved:
//! - **moves**: paused at a quarter, the middle or three quarters of its span, some property it animates is at neither
//!   its start nor its end value, and, where its frames change at all, some such frame differs from both its first and
//!   its last frame (a jump cut, or keyframes that change nothing, fails). A tier-2 set (a PPTX `p:set` or ODF
//!   `anim:set`, [`model::switches`]) is a switch at its begin instead, as an appear is ([`moved::switches`]): before
//!   it the property shows what it showed, from it the set's value, which holds; a step whose tracks are all sets
//!   ([`model::only_switches`]) is not held to `moves`, and in a step that also moves, its sets' keys do not count
//!   as moving;
//! - **direction**: an effect whose `dir` is the direction the content moves shows it on the side it
//!   comes from at its middle (an entrance) or the side it goes to (an exit), by its box or, for a clip, by where its
//!   frame changed;
//! - **end state**: what each animated node shows at the end (its computed animated properties, whether it shows)
//!   equals what it shows once the stage settled; only then may the end frame differ from the still render by more than
//!   the antialias bound, as a composited frame rasterizes;
//! - **start state**: at its first frame every unit shows as it did before the stage (an entrance's are hidden).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use slidock_bench_schema::lanes::{
    ANTIALIAS_CHANNEL, ANTIALIAS_PIXELS, DeliveryCounts, EffectCounts, EffectPlay, EffectRef,
    FrameCheck, FrameMatch, FrameStrip, MotionRowMetrics, RevealCheck, RevealMiss, StepCounts,
    StepPlay, StepTiming, TimelineFidelity, TimelineField, TimelineMismatch, TransitionPlay,
};
use slidock_bench_schema::results::MotionOutcome;
use slidock_bench_schema::{RowStep, Sha256};

use super::deck::{DeckSlide, DeckStep, DeckTransition};
use super::drive::{Animation, Batch, NodeProbe, SlideSeen, StageSeen};
use super::model::{self, MS_MAX, Scheduled};
use crate::images::Change;

/// Times within this many ms are the same time (WAAPI reads them back as doubles).
const EPSILON_MS: f64 = 0.5;
/// A channel within this of another is the same (a composited frame's noise).
pub const NOISE_CHANNEL: u8 = 8;
/// A change of at most this many pixels is no change.
const NOISE_PIXELS: u32 = 16;
/// A displacement of at most this many pixels is none.
const STILL_PX: f64 = 0.5;
/// Build effects whose `dir` is the direction their content moves.
const DIRECTED_BUILDS: [&str; 6] = [
    "flyIn",
    "floatIn",
    "peek",
    "wipe",
    "fadeAndMove",
    "driftAndScale",
];

/// What played for a step or a transition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Played {
    /// The vocabulary effect the deck names.
    Native(String),
    /// The canonical fallback.
    Fallback(String),
    Nothing,
}

impl Played {
    pub fn label(&self) -> String {
        match self {
            Played::Native(effect) => effect.clone(),
            Played::Fallback(fallback) => format!("fallback:{fallback}"),
            Played::Nothing => "nothing".to_owned(),
        }
    }
}

/// A transition, or a step by its index in the slide's timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ItemRef {
    Transition,
    Step(u32),
}

impl ItemRef {
    pub fn label(self) -> String {
        match self {
            ItemRef::Transition => "transition".to_owned(),
            ItemRef::Step(index) => format!("step {index}"),
        }
    }
}

/// A check an item failed: its name (`end-state`, `start-state`, `settled`, `reveal`, `monotonic`, `params`,
/// `placement`, or a [`TimelineField`] as JSON names it), what went wrong, and the failure's exact signature, which an
/// accepted check of `bench/motion/fallback-allow.json` must name (renderer independent: no pixel counts).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub check: String,
    pub text: String,
    pub detail: String,
}

impl Problem {
    /// A problem whose text is its signature.
    pub fn new(check: &str, text: impl Into<String>) -> Problem {
        let text = text.into();
        Problem {
            check: check.to_owned(),
            detail: text.clone(),
            text,
        }
    }

    pub fn with(check: &str, text: impl Into<String>, detail: impl Into<String>) -> Problem {
        Problem {
            check: check.to_owned(),
            text: text.into(),
            detail: detail.into(),
        }
    }
}

/// A step or the transition of a slide, as it played.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub item: ItemRef,
    pub native: String,
    /// The effect the deck names.
    pub effect: Option<String>,
    /// Its params as the deck writes them.
    pub params: Option<Value>,
    pub fallback: String,
    pub played: Played,
    /// Checks it failed; empty when it played as the deck says.
    pub problems: Vec<Problem>,
    /// Failed checks `bench/motion/fallback-allow.json` accepts, each with its reason as text.
    pub accepted: Vec<Problem>,
}

/// A stage or transition row: where it is, its class, its metrics, its frame, and the checks it failed.
pub type Row = (
    RowStep,
    MotionOutcome,
    MotionRowMetrics,
    Sha256,
    Vec<String>,
);

/// One slide checked.
#[derive(Clone, Debug, PartialEq)]
pub struct SlideCheck {
    pub fidelity: TimelineFidelity,
    /// Stage rows by step, then transition rows by per-mille.
    pub rows: Vec<Row>,
    /// The settled frame of each stage.
    pub positions: Vec<(u32, Sha256)>,
    pub items: Vec<Item>,
}

fn timing(start: f64, duration: f64) -> StepTiming {
    StepTiming {
        start_ms: slidock_bench_metrics::quantise(start),
        duration_ms: slidock_bench_metrics::quantise(duration),
    }
}

fn same(a: f64, b: f64) -> bool {
    (a - b).abs() <= EPSILON_MS
}

fn ms_text(value: f64) -> String {
    slidock_canon::to_js_string(slidock_bench_metrics::quantise(value))
}

/// A settled frame against its reference: the same PNG, or not.
fn check(expected: &Sha256, actual: &Sha256) -> FrameCheck {
    FrameCheck {
        status: if expected == actual {
            FrameMatch::Identical
        } else {
            FrameMatch::Differs
        },
        expected: expected.clone(),
        actual: actual.clone(),
    }
}

/// A paused frame against its reference: composited content may antialias differently (see [`FrameMatch`]).
fn check_paused(input: &SlideInput<'_>, expected: &Sha256, actual: &Sha256) -> FrameCheck {
    let mut checked = check(expected, actual);
    if checked.status == FrameMatch::Differs
        && ((input.frames.antialiased)(expected, actual)
            || (input.frames.matches)(expected, actual))
    {
        checked.status = FrameMatch::Antialiased;
    }
    checked
}

/// A settled frame against its still render: the same PNG, or a large image rasterized a few pixels apart within the
/// player's antialias bound (the container's software raster); never the looser `match` of the static lane's metrics.
fn check_settled(input: &SlideInput<'_>, expected: &Sha256, actual: &Sha256) -> FrameCheck {
    let mut checked = check(expected, actual);
    if checked.status == FrameMatch::Differs && (input.frames.antialiased)(expected, actual) {
        checked.status = FrameMatch::Antialiased;
    }
    checked
}

fn failed(check: Option<&FrameCheck>) -> bool {
    check.is_some_and(|c| !c.status.passes())
}

/// The steps and the transition the player reported falling back on (RT0006), and its other complaints.
struct Reported {
    fallback_steps: BTreeSet<u32>,
    fallback_transition: bool,
    other: Vec<(String, String)>,
    counts: BTreeMap<String, u32>,
}

fn reported(seen: &SlideSeen, slide_id: &str) -> Reported {
    let mut out = Reported {
        fallback_steps: BTreeSet::new(),
        fallback_transition: false,
        other: Vec::new(),
        counts: BTreeMap::new(),
    };
    let prefix = format!("slide {slide_id} ");
    for diagnostic in &seen.diagnostics {
        let Some(rest) = diagnostic.message.strip_prefix(&prefix) else {
            continue;
        };
        *out.counts.entry(diagnostic.code.clone()).or_default() += 1;
        match diagnostic.code.as_str() {
            "RT0006" if rest.starts_with("transition:") => out.fallback_transition = true,
            "RT0006" => {
                let index = rest
                    .strip_prefix("step ")
                    .and_then(|r| r.split(':').next())
                    .and_then(|n| n.parse::<u32>().ok());
                if let Some(index) = index {
                    out.fallback_steps.insert(index);
                }
            }
            // A slide shown before its fonts and images loaded: its frames may show it unloaded.
            "RT0001" | "RT0007" | "RT0008" | "RT0009" | "RT0005" => {
                out.other.push((diagnostic.code.clone(), rest.to_owned()));
            }
            _ => {}
        }
    }
    out
}

/// Whether a step's target covers an animation's target.
pub fn covers(step: &DeckStep, animation: &Animation) -> bool {
    if animation.element.as_deref() != Some(step.target.element.as_str()) {
        return false;
    }
    match (step.target.paragraphs, animation.paragraph) {
        (None, _) => true,
        (Some((first, last)), Some(p)) => (first..=last).contains(&p),
        (Some(_), None) => false,
    }
}

/// The first round of a step's moving animations (its hides left out): from the earliest start to the latest end.
pub fn interval(animations: &[&Animation]) -> Option<(f64, f64)> {
    let moving = || animations.iter().filter(|a| !a.is_hide());
    let start = moving().map(|a| a.delay).reduce(f64::min)?;
    let end = moving().map(|a| a.delay + a.duration).reduce(f64::max)?;
    Some((start, end))
}

/// The animations of a group's batches, each attributed to a step of the group (by index into `steps`), or not.
fn attribute(
    steps: &[DeckStep],
    in_group: &[usize],
    expected: &[Scheduled],
    animations: &[&Animation],
) -> Vec<Option<usize>> {
    let mut at = 0usize;
    animations
        .iter()
        .map(|animation| {
            let candidates: Vec<usize> = in_group
                .iter()
                .enumerate()
                .skip(at)
                .filter(|(_, index)| steps.get(**index).is_some_and(|s| covers(s, animation)))
                .map(|(position, _)| position)
                .collect();
            let first = *candidates.first()?;
            // A later step on the same target takes the animation once it starts at or after that step's start.
            let mut chosen = first;
            for position in candidates.iter().skip(1) {
                let (Some(&now), Some(&later)) = (in_group.get(chosen), in_group.get(*position))
                else {
                    break;
                };
                let (Some(now), Some(later)) = (expected.get(now), expected.get(later)) else {
                    break;
                };
                if later.start > now.start + EPSILON_MS
                    && animation.delay + EPSILON_MS >= later.start
                {
                    chosen = *position;
                } else {
                    break;
                }
            }
            at = chosen;
            in_group.get(chosen).copied()
        })
        .collect()
}

/// Each animation of group `group`'s batches attributed to a step of `steps` (by index), as [`check_slide`] attributes
/// them: first with whole targets, then with the delivered unit counts.
pub fn owners(steps: &[DeckStep], group: u32, animations: &[&Animation]) -> Vec<Option<usize>> {
    let groups = model::groups(steps);
    let in_group: Vec<usize> = (0..steps.len())
        .filter(|i| groups.get(*i) == Some(&group))
        .collect();
    let first = attribute(steps, &in_group, &model::schedule(steps, |_| 1), animations);
    let units = |step: usize| -> u32 {
        let nodes: BTreeSet<u64> = animations
            .iter()
            .zip(&first)
            .filter(|(a, owner)| **owner == Some(step) && !a.is_hide())
            .map(|(a, _)| a.node)
            .collect();
        nodes.len() as u32
    };
    attribute(steps, &in_group, &model::schedule(steps, units), animations)
}

/// What a step's animations show: its start and duration, and the units it animated.
#[derive(Clone, Debug, Default)]
struct Observed {
    animations: Vec<Animation>,
    group: Option<u32>,
}

impl Observed {
    fn moving(&self) -> impl Iterator<Item = &Animation> {
        self.animations.iter().filter(|a| !a.is_hide())
    }

    fn start(&self) -> Option<f64> {
        self.moving().map(|a| a.delay).reduce(f64::min)
    }

    fn duration(&self) -> Option<f64> {
        self.moving().map(|a| a.duration).reduce(f64::max)
    }

    fn units(&self) -> u32 {
        let nodes: BTreeSet<u64> = self.moving().map(|a| a.node).collect();
        nodes.len() as u32
    }
}

/// Whether the deck's transition makes the player animate: an effect other than a plain cut, or a fallback other
/// than none, over a valid duration.
pub fn transition_animates(transition: &DeckTransition) -> bool {
    let duration = transition.duration_ms.as_f64().unwrap_or(-1.0);
    let valid = duration.is_finite() && duration > 0.0 && duration <= MS_MAX;
    let plays = match transition.effect.as_deref() {
        Some("none") => false,
        Some("cut") => {
            transition
                .params
                .as_ref()
                .and_then(|p| p.get("throughBlack"))
                .and_then(Value::as_bool)
                == Some(true)
        }
        Some(_) => true,
        None => transition.fallback != "none",
    };
    valid && plays
}

/// Two stored frames' SSIM and mean ΔE00, or `None` when they cannot be read.
pub type ScoreFn<'a> = dyn Fn(&Sha256, &Sha256) -> Option<(f64, f64)> + 'a;

/// Where two stored frames differ inside a region (the whole frame when `None`) by more than a channel tolerance.
pub type ChangeFn<'a> = dyn Fn(&Sha256, &Sha256, Option<[f64; 4]>, u8) -> Option<Change> + 'a;

/// Stored frames compared by content.
pub struct Frames<'a> {
    /// SSIM and mean ΔE00 at the metric size, as the static lane scores.
    pub scores: &'a ScoreFn<'a>,
    /// Whether two frames differ only by antialiasing (within the player's own bound).
    pub antialiased: &'a dyn Fn(&Sha256, &Sha256) -> bool,
    /// Whether the static lane's metrics class one frame a `match` of the other.
    pub matches: &'a dyn Fn(&Sha256, &Sha256) -> bool,
    pub change: &'a ChangeFn<'a>,
    /// Whether two frames differ only by edges a device pixel apart (within the antialias bound once each may be), as
    /// a composited layer at a fractional offset draws them.
    pub displaced: &'a dyn Fn(&Sha256, &Sha256) -> bool,
}

/// How the lane reads a slide's walk.
pub struct SlideInput<'a> {
    pub slide: &'a DeckSlide,
    pub seen: &'a SlideSeen,
    /// The first slide the show starts on: it is not entered by a transition.
    pub first: bool,
    /// The shown slides before and after it.
    pub previous: Option<&'a DeckSlide>,
    pub next: Option<&'a DeckSlide>,
    /// Compares stored frames.
    pub frames: &'a Frames<'a>,
}

/// Checks one walked slide.
pub fn check_slide(input: &SlideInput<'_>) -> SlideCheck {
    let SlideInput { slide, seen, .. } = input;
    let steps = &slide.timeline;
    let report = reported(seen, &slide.id);
    let mut mismatches: Vec<TimelineMismatch> = Vec::new();
    let mismatch = |step: Option<u32>, field, expected: String, actual: String| TimelineMismatch {
        step,
        field,
        expected,
        actual,
    };
    for (code, message) in &report.other {
        mismatches.push(mismatch(
            None,
            TimelineField::Player,
            String::new(),
            format!("{code} {message}"),
        ));
    }
    let groups = model::groups(steps);
    let native_of = |index: usize| -> bool {
        steps.get(index).is_some_and(|s| s.effect.is_some())
            && !report.fallback_steps.contains(&(index as u32))
    };
    // Attribute each stage's animations to its group's steps, first with whole targets, then with the delivered
    // unit counts.
    let stage_animations: Vec<Vec<&Animation>> = seen
        .stages
        .iter()
        .map(|stage| {
            stage
                .batches
                .iter()
                .flat_map(|batch| batch.animations.iter())
                .filter(|a| !a.whole && a.role == "current")
                .collect()
        })
        .collect();
    let mut observed: Vec<Observed> = vec![Observed::default(); steps.len()];
    let mut extras: Vec<(u32, &Animation)> = Vec::new();
    let mut expected = model::schedule(steps, |_| 1);
    for pass in 0..2 {
        if pass == 1 {
            expected = model::schedule(steps, |i| observed.get(i).map_or(1, Observed::units));
            observed = vec![Observed::default(); steps.len()];
            extras.clear();
        }
        for (k, animations) in stage_animations.iter().enumerate() {
            let k = k as u32;
            let in_group: Vec<usize> = (0..steps.len())
                .filter(|i| groups.get(*i) == Some(&k))
                .collect();
            for (animation, owner) in animations
                .iter()
                .zip(attribute(steps, &in_group, &expected, animations))
            {
                match owner.and_then(|i| observed.get_mut(i).map(|o| (i, o))) {
                    Some((_, o)) => {
                        o.animations.push((*animation).clone());
                        o.group = Some(k);
                    }
                    None => extras.push((k, animation)),
                }
            }
        }
    }
    // Two steps of a group on one target starting together: the later one played with the earlier one.
    for index in 1..steps.len() {
        let (Some(previous), Some(step)) = (steps.get(index - 1), steps.get(index)) else {
            continue;
        };
        let shared = previous.target == step.target
            && groups.get(index - 1) == groups.get(index)
            && expected
                .get(index - 1)
                .zip(expected.get(index))
                .is_some_and(|(a, b)| same(a.start, b.start));
        if shared
            && observed.get(index).is_some_and(|o| o.animations.is_empty())
            && let Some(earlier) = observed.get(index - 1).cloned()
            && let Some(slot) = observed.get_mut(index)
        {
            *slot = earlier;
        }
    }
    let mut plays = Vec::with_capacity(steps.len());
    let mut effects: BTreeMap<String, EffectCounts> = BTreeMap::new();
    let mut delivery = DeliveryCounts::default();
    let mut items = Vec::new();
    let mut played_steps = 0u32;
    for (index, step) in steps.iter().enumerate() {
        let i = index as u32;
        let Some(want) = expected.get(index).copied() else {
            continue;
        };
        let native = native_of(index);
        let seen_step = observed.get(index).cloned().unwrap_or_default();
        let animates = model::animates(step, native);
        let mut problems = Vec::new();
        let mut step_mismatches = Vec::new();
        let played = if seen_step.animations.is_empty() && animates {
            // Played in another group?
            let elsewhere = extras
                .iter()
                .find(|(_, a)| covers(step, a))
                .map(|(g, _)| *g);
            match elsewhere {
                Some(group) => step_mismatches.push(mismatch(
                    Some(i),
                    TimelineField::Group,
                    want.group.to_string(),
                    group.to_string(),
                )),
                None => step_mismatches.push(mismatch(
                    Some(i),
                    TimelineField::Missing,
                    "animations".into(),
                    "none".into(),
                )),
            }
            Played::Nothing
        } else if native {
            Played::Native(step.effect.clone().unwrap_or_default())
        } else {
            Played::Fallback(step.fallback.clone())
        };
        if !animates && seen_step.moving().next().is_some() {
            step_mismatches.push(mismatch(
                Some(i),
                TimelineField::Extra,
                "nothing".into(),
                format!("{} animations", seen_step.moving().count()),
            ));
        }
        // Tier-2 tracks carry their own begin and length inside the step.
        let tracks = model::track_spans(step, native);
        let first_begin = tracks
            .iter()
            .flatten()
            .map(|span| span.begin)
            .reduce(f64::min)
            .unwrap_or(0.0);
        if let Some(start) = seen_step.start()
            && !same(start, want.start + first_begin)
        {
            step_mismatches.push(mismatch(
                Some(i),
                TimelineField::Start,
                ms_text(want.start + first_begin),
                ms_text(start),
            ));
        }
        let duration_ms = model::animation_ms(step, &want, native);
        // A set may last its track's length or none: it switches at its begin and holds either way.
        let lengths: Vec<f64> = match &tracks {
            Some(spans) => spans
                .iter()
                .flat_map(|span| std::iter::once(span.length).chain(span.set.then_some(0.0)))
                .collect(),
            None => vec![duration_ms],
        };
        if seen_step.moving().next().is_some()
            && !seen_step
                .moving()
                .any(|a| lengths.iter().any(|length| same(a.duration, *length)))
        {
            step_mismatches.push(mismatch(
                Some(i),
                TimelineField::Duration,
                lengths
                    .iter()
                    .map(|l| ms_text(*l))
                    .collect::<Vec<_>>()
                    .join(" or "),
                ms_text(seen_step.duration().unwrap_or_default()),
            ));
        }
        // A step that lasts no time has nothing between its ends, but its sets still switch.
        let lasts = duration_ms > 0.0 || model::switches(step, native).is_some();
        if played != Played::Nothing && lasts && step.kind != "media" {
            step_mismatches.extend(step_motion(input, step, native, &seen_step, i));
        }
        let counts = effects.entry(step.native.clone()).or_default();
        match played {
            Played::Native(_) => counts.native += 1,
            Played::Fallback(_) => counts.fallback += 1,
            Played::Nothing => counts.unsupported += 1,
        }
        if played != Played::Nothing {
            played_steps += 1;
        }
        let units = seen_step.units();
        match step.delivery.as_deref() {
            Some("byParagraph") => delivery.paragraphs += units,
            Some("byWord") => delivery.words += units,
            Some("byChar") => delivery.chars += units,
            _ => {}
        }
        problems.extend(
            step_mismatches
                .iter()
                .map(|m| Problem::new(&field_name(m.field), describe(m))),
        );
        mismatches.extend(step_mismatches);
        plays.push(StepPlay {
            index: i,
            group: want.group,
            trigger: step.trigger.clone(),
            kind: step.kind.clone(),
            target: step.target.label(),
            native: step.native.clone(),
            effect: step.effect.clone(),
            params: step.params.as_ref().map(Value::to_string),
            delivery: step.delivery.clone().filter(|d| d != "all"),
            fallback: step.fallback.clone(),
            played: match played {
                Played::Native(_) => EffectPlay::Native,
                Played::Fallback(_) => EffectPlay::Fallback,
                Played::Nothing => EffectPlay::Unsupported,
            },
            expected: timing(want.start, duration_ms),
            actual: seen_step
                .start()
                .map(|start| timing(start, seen_step.duration().unwrap_or_default())),
            played_in: seen_step.group,
            units,
        });
        items.push(Item {
            item: ItemRef::Step(i),
            native: step.native.clone(),
            effect: step.effect.clone(),
            params: step.params.clone(),
            fallback: step.fallback.clone(),
            played,
            problems,
            accepted: Vec::new(),
        });
    }
    // Order: within each group, the steps that played sorted by when they started, against the deck's starts.
    let mut order = true;
    for k in 0..seen.stages.len() as u32 {
        let in_group: Vec<usize> = (0..steps.len())
            .filter(|i| groups.get(*i) == Some(&k))
            .collect();
        let started: Vec<(usize, f64, f64)> = in_group
            .iter()
            .filter_map(|i| {
                let start = observed.get(*i)?.start()?;
                Some((*i, start, expected.get(*i)?.start))
            })
            .collect();
        let mut by_seen = started.clone();
        by_seen.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        let mut by_deck = started;
        by_deck.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.0.cmp(&b.0)));
        let seen_order: Vec<usize> = by_seen.iter().map(|s| s.0).collect();
        let deck_order: Vec<usize> = by_deck.iter().map(|s| s.0).collect();
        if seen_order != deck_order {
            order = false;
            let text = |order: &[usize]| {
                order
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            mismatches.push(mismatch(
                None,
                TimelineField::Order,
                text(&deck_order),
                text(&seen_order),
            ));
            for item in items.iter_mut() {
                if let ItemRef::Step(i) = item.item
                    && deck_order.contains(&(i as usize))
                {
                    item.problems.push(Problem::new(
                        "order",
                        format!("group {k} played out of order"),
                    ));
                }
            }
        }
    }
    for (k, animation) in &extras {
        mismatches.push(mismatch(
            None,
            TimelineField::Extra,
            format!("group {k}"),
            format!(
                "{} {}",
                animation.element.as_deref().unwrap_or("?"),
                animation.keys.join(",")
            ),
        ));
    }
    let mut disallowed: BTreeSet<&str> = BTreeSet::new();
    let all = seen
        .transition
        .iter()
        .chain(seen.stages.iter().flat_map(|s| s.batches.iter()))
        .flat_map(|batch| batch.animations.iter());
    for animation in all {
        for key in &animation.keys {
            if !allowed_key(key) {
                disallowed.insert(key);
            }
        }
    }
    for key in disallowed {
        mismatches.push(mismatch(
            None,
            TimelineField::Property,
            "allowed".into(),
            key.to_owned(),
        ));
    }
    let (transition, transition_rows, transition_item) =
        check_transition(input, &report, &mut mismatches);
    if let Some(item) = transition_item {
        items.insert(0, item);
    }
    // Where each entrance ends, and its batch's last frame, by group: its first frame must not show it built.
    let mut entering: BTreeMap<u32, Vec<Entering>> = BTreeMap::new();
    for (step, seen_step) in steps.iter().zip(&observed) {
        let Some(group) = seen_step.group.filter(|_| step.kind == "entrance") else {
            continue;
        };
        let nodes: BTreeSet<u64> = seen_step.animations.iter().map(|a| a.node).collect();
        let batch = input.seen.stages.get(group as usize).and_then(|stage| {
            stage.batches.iter().find(|b| {
                seen_step
                    .animations
                    .iter()
                    .all(|a| b.animations.contains(a))
            })
        });
        let ends = batch.and_then(|b| Some((b.sample_pm(0)?.clone(), b.sample_pm(1000)?.clone())));
        // Where it starts (a motion that starts in view) and where it ends.
        if let Some((start, end)) = ends
            && let Some(region) = moved::boxes(&[start, end.clone()], &nodes)
        {
            entering.entry(group).or_default().push(Entering {
                label: step.target.label(),
                region,
                end: end.frame.clone(),
            });
        }
    }
    let (stage_rows, positions) = stage_rows(input, &groups, &mismatches, &entering);
    // A step whose stage's frames failed did not play as the deck says either.
    for item in items.iter_mut() {
        let ItemRef::Step(i) = item.item else {
            continue;
        };
        let Some(group) = groups.get(i as usize) else {
            continue;
        };
        if let Some((_, outcome, metrics, ..)) = stage_rows
            .iter()
            .find(|(step, ..)| *step == RowStep::Stage(*group))
            && *outcome == MotionOutcome::MotionMismatch
        {
            item.problems.extend(frame_problems(metrics));
        }
    }
    let durations_exact = !mismatches
        .iter()
        .any(|m| m.field == TimelineField::Duration);
    let delays_exact = !mismatches.iter().any(|m| m.field == TimelineField::Start);
    let fidelity = TimelineFidelity {
        steps: StepCounts {
            expected: steps.len() as u32,
            played: played_steps,
        },
        order,
        durations_exact,
        delays_exact,
        delivery,
        effects,
        transition,
        plays,
        mismatches,
        ir: None,
        diagnostics: report.counts,
    };
    let mut rows = stage_rows;
    rows.extend(transition_rows);
    SlideCheck {
        fidelity,
        rows,
        positions,
        items,
    }
}

/// How a step that animates moved: its switches ([`moved::switches`]), [`moved::moves`] over its own pauses unless it
/// only switches, and, natively, [`moved::along`] its `dir`.
fn step_motion(
    input: &SlideInput<'_>,
    step: &DeckStep,
    native: bool,
    seen: &Observed,
    index: u32,
) -> Vec<TimelineMismatch> {
    let every: Vec<&Animation> = seen.animations.iter().collect();
    let own: Vec<&Animation> = seen.moving().collect();
    let Some(stage) = seen.group.and_then(|g| input.seen.stages.get(g as usize)) else {
        return Vec::new();
    };
    let Some(batch) = stage
        .batches
        .iter()
        .find(|b| every.iter().all(|a| b.animations.contains(a)))
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut switched: BTreeSet<&str> = BTreeSet::new();
    if let Some(switches) = model::switches(step, native) {
        let names = |n: u64| node_name(&every, n);
        let checked = switches.and_then(|switches| {
            switched.extend(switches.iter().map(|s| s.key));
            let timed = moved::timed_switches(&switches, &every, &names)?;
            moved::switches(&batch.samples, &batch.animations, &timed, &names)
        });
        if let Err(why) = checked {
            out.push(TimelineMismatch {
                step: Some(index),
                field: TimelineField::Moves,
                expected: "a switch at its begin".into(),
                actual: why,
            });
        }
    }
    // A step that only sets switches at its begin, as an appear does: `moves` does not apply.
    if model::only_switches(step, native) {
        return out;
    }
    let Some((start, end)) = interval(&own) else {
        return out;
    };
    let Some(pauses) = moved::pauses_of(|t| batch.sample(t), start, end) else {
        return out;
    };
    let nodes: BTreeSet<u64> = own.iter().map(|a| a.node).collect();
    let region = moved::region(&pauses, &nodes);
    if let Err(why) = moved::moves(input.frames, &pauses, &nodes, region, &switched) {
        out.push(TimelineMismatch {
            step: Some(index),
            field: TimelineField::Moves,
            expected: "motion between its ends".into(),
            actual: why,
        });
    }
    let dir = native
        .then(|| {
            moved::directed(
                step.effect.as_deref(),
                step.params.as_ref(),
                &DIRECTED_BUILDS,
            )
        })
        .flatten();
    if let Some(dir) = dir {
        let seen = moved::motion(input.frames, &pauses, &nodes, region, step.kind != "exit");
        if let Some(Err(why)) = moved::along(dir, seen) {
            out.push(TimelineMismatch {
                step: Some(index),
                field: TimelineField::Direction,
                expected: format!("its content moves {dir}"),
                actual: why,
            });
        }
    }
    out
}

/// A transition that matches elements between its two slides.
fn is_move(effect: Option<&str>) -> bool {
    matches!(effect, Some("magicMove" | "morph"))
}

/// How many match keys two slides share.
fn shared_keys(a: Option<&DeckSlide>, b: &DeckSlide) -> usize {
    let keys = |slide: &DeckSlide| -> BTreeSet<String> {
        slide
            .match_keys
            .as_ref()
            .and_then(Value::as_object)
            .map(|keys| {
                keys.values()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    a.map_or(0, |a| keys(a).intersection(&keys(b)).count())
}

/// The properties the player may animate: opacity, transforms, clip paths, filters, masks, and the
/// deck's registered `--sd-*` properties; anything else would relayout or repaint what the backend printed.
pub fn allowed_key(key: &str) -> bool {
    matches!(
        key,
        "opacity" | "transform" | "translate" | "rotate" | "scale" | "clipPath" | "filter"
    ) || key.starts_with("mask")
        || key.starts_with("webkitMask")
        || key.starts_with("--sd-")
}

/// A timeline field's JSON name.
pub fn field_name(field: TimelineField) -> String {
    serde_json::to_value(field)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn describe(mismatch: &TimelineMismatch) -> String {
    let field = field_name(mismatch.field);
    format!(
        "{field}: expected {}, played {}",
        mismatch.expected, mismatch.actual
    )
}

/// The frame checks a stage row failed, each with its detail (`<check>: <detail>` in the row's `failures`).
fn frame_problems(metrics: &MotionRowMetrics) -> Vec<Problem> {
    let texts = [
        (
            "end-state",
            "its stage's end state differs from the still render",
        ),
        (
            "start-state",
            "its stage's first frame differs from the state before it",
        ),
        (
            "settled",
            "its stage settles differently from the still render",
        ),
        (
            "reveal",
            "its stage shows other units than the timeline says",
        ),
        ("monotonic", "a unit changed without a step of its stage"),
    ];
    metrics
        .failures
        .iter()
        .filter_map(|failure| {
            let (check, detail) = failure.split_once(": ")?;
            let (_, text) = texts.iter().find(|(name, _)| *name == check)?;
            Some(Problem::with(check, format!("{text}: {detail}"), detail))
        })
        .collect()
}

type TransitionRows = Vec<Row>;

/// Transition effects whose `dir` is the direction their content moves, with the layer that shows it: the incoming
/// one arriving, or the outgoing one leaving.
fn directed_transition(effect: &str) -> Option<bool> {
    match effect {
        "push" | "cover" | "wipe" => Some(true),
        "uncover" => Some(false),
        _ => None,
    }
}

/// The node name a pose difference reports: its element (and paragraph), a split unit of it, a slide layer by role.
fn node_name(animations: &[&Animation], node: u64) -> String {
    let Some(animation) = animations.iter().find(|a| a.node == node) else {
        return format!("node {node}");
    };
    if animation.whole {
        return format!("{} layer", animation.role);
    }
    let mut name = animation
        .element
        .clone()
        .unwrap_or_else(|| format!("node {node}"));
    if let Some(paragraph) = animation.paragraph {
        name.push_str(&format!(" ¶{paragraph}"));
    }
    if animation.split {
        name.push_str(" unit");
    }
    name
}

/// A paused end frame against its settled reference, with the animated nodes' poses at that end against theirs once
/// settled: identical; or no pose differs and the frames are within the antialias bound, or every pose compared equal
/// and the static lane's metrics class them a `match` (a glyph re-rasterized in a composited layer). Returns the check
/// and the pose differences.
fn check_end(
    input: &SlideInput<'_>,
    expected: &Sha256,
    actual: &Sha256,
    poses: Option<&moved::EndPoses>,
) -> FrameCheck {
    let mut checked = check(expected, actual);
    if checked.status == FrameMatch::Identical {
        return checked;
    }
    let differ = poses.is_some_and(|p| !p.differences.is_empty());
    let proven = poses.is_some_and(|p| p.differences.is_empty() && p.complete);
    if !differ
        && ((input.frames.antialiased)(expected, actual)
            || (proven && (input.frames.matches)(expected, actual)))
    {
        checked.status = FrameMatch::Antialiased;
    }
    checked
}

/// What each node stage 0 probed shows as the slide opens, before the builds that start by themselves play (what
/// Keynote shows as the transition into the slide ends: its all-stages export's lead page): a node one of the stage's
/// batches animates as the first such batch's first pause probed it, any other as the stage settled.
fn opening_nodes(stage: &StageSeen) -> Vec<NodeProbe> {
    stage
        .settled_nodes
        .iter()
        .map(|settled| {
            stage
                .batches
                .iter()
                .find(|b| b.animations.iter().any(|a| a.node == settled.node))
                .and_then(|b| b.sample_pm(0)?.node(settled.node))
                .unwrap_or(settled)
                .clone()
        })
        .collect()
}

/// The end detail of a failed end check: the pose differences, or `pixels` when only the frames differ.
fn end_detail(poses: Option<&moved::EndPoses>) -> String {
    match poses {
        Some(p) if !p.differences.is_empty() => p.differences.join("; "),
        _ => "pixels".to_owned(),
    }
}

fn check_transition(
    input: &SlideInput<'_>,
    report: &Reported,
    mismatches: &mut Vec<TimelineMismatch>,
) -> (Option<TransitionPlay>, TransitionRows, Option<Item>) {
    let SlideInput {
        slide, seen, first, ..
    } = input;
    let Some(transition) = &slide.transition else {
        if let Some(batch) = &seen.transition {
            mismatches.push(TimelineMismatch {
                step: None,
                field: TimelineField::Transition,
                expected: "none".into(),
                actual: format!("{} ms", ms_text(batch.duration_ms)),
            });
        }
        return (None, Vec::new(), None);
    };
    if *first {
        // The show starts on it: nothing transitions into it.
        return (None, Vec::new(), None);
    }
    let native = transition.effect.is_some() && !report.fallback_transition;
    let animates = transition_animates(transition);
    // Each failed check: its name, the timeline field it reports as, and what went wrong.
    let mut problems: Vec<(&str, TimelineField, String)> = Vec::new();
    let expected_ms = transition.duration_ms.as_f64().unwrap_or_default();
    let played = match (&seen.transition, native) {
        (None, _) if animates => {
            problems.push((
                "transition",
                TimelineField::Transition,
                format!(
                    "played nothing where the deck says {} over {} ms",
                    transition
                        .effect
                        .clone()
                        .unwrap_or_else(|| transition.fallback.clone()),
                    ms_text(expected_ms)
                ),
            ));
            Played::Nothing
        }
        // A plain cut plays as the cut it is: nothing moves.
        (Some(_), true) | (None, true) => {
            Played::Native(transition.effect.clone().unwrap_or_default())
        }
        (Some(_), false) | (None, false) => Played::Fallback(transition.fallback.clone()),
    };
    if !animates && seen.transition.is_some() {
        problems.push((
            "transition",
            TimelineField::Transition,
            "played a transition where the deck's is a cut".to_owned(),
        ));
    }
    // A Magic Move or Morph matches elements between the slide it leaves and this one. One that matches nothing there
    // but pairs this slide with the next is the move out of it, unless the next slide moves in by its own: then the
    // keys are that move's, and this one morphs nothing (as Keynote plays a move whose slides share no object).
    if native
        && is_move(transition.effect.as_deref())
        && shared_keys(input.previous, slide) == 0
        && input.next.is_some_and(|next| {
            shared_keys(Some(slide), next) > 0
                && !next
                    .transition
                    .as_ref()
                    .is_some_and(|t| is_move(t.effect.as_deref()))
        })
    {
        problems.push((
            "placement",
            TimelineField::Transition,
            "its match keys pair the slide with the next shown one: it is the move out of the slide, played into it"
                .to_owned(),
        ));
    }
    let stage0 = seen.stages.first();
    let incoming = stage0.map(|stage| {
        stage
            .batches
            .first()
            .and_then(|b| b.frame(0))
            .cloned()
            .unwrap_or_else(|| stage.settled.clone())
    });
    let (from, to) = match &seen.transition {
        Some(batch) => {
            if !same(batch.duration_ms, expected_ms) {
                problems.push((
                    "transition",
                    TimelineField::Transition,
                    format!(
                        "duration {} ms, the deck says {} ms",
                        ms_text(batch.duration_ms),
                        ms_text(expected_ms)
                    ),
                ));
            }
            let animations: Vec<&Animation> = batch.animations.iter().collect();
            let layer = |arriving: bool| -> BTreeSet<u64> {
                animations
                    .iter()
                    .filter(|a| a.whole && (a.role == "incoming") == arriving)
                    .map(|a| a.node)
                    .collect()
            };
            // A cut through black switches to black and back: it has nothing between its ends.
            let discrete = native && transition.effect.as_deref() == Some("cut");
            if animates
                && !discrete
                && let Some(pauses) = moved::pauses_of(|t| batch.sample(t), 0.0, batch.duration_ms)
            {
                let nodes: BTreeSet<u64> = animations.iter().map(|a| a.node).collect();
                if let Err(why) =
                    moved::moves(input.frames, &pauses, &nodes, None, &BTreeSet::new())
                {
                    problems.push(("moves", TimelineField::Moves, why));
                }
                let dir = native
                    .then(|| transition.params.as_ref()?.get("dir")?.as_str())
                    .flatten();
                let arriving = transition.effect.as_deref().and_then(directed_transition);
                if let (Some(dir), Some(arriving)) = (dir, arriving) {
                    let seen =
                        moved::motion(input.frames, &pauses, &layer(arriving), None, arriving);
                    if let Some(Err(why)) = moved::along(dir, seen) {
                        problems.push((
                            "direction",
                            TimelineField::Direction,
                            format!("dir {dir}: {why}"),
                        ));
                    }
                }
            }
            // A cut through black starts black. A move draws the incoming slide's matched elements at the outgoing
            // slide's poses by transforms at fractional offsets: their edges may land a device pixel from where layout
            // put them.
            let through_black = transition.effect.as_deref() == Some("cut");
            let moving = native && is_move(transition.effect.as_deref());
            let from = seen
                .before
                .as_ref()
                .zip(batch.frame(0))
                .filter(|_| !through_black)
                .map(|(before, t0)| {
                    let mut checked = check_paused(input, before, t0);
                    if checked.status == FrameMatch::Differs
                        && moving
                        && (input.frames.displaced)(before, t0)
                    {
                        checked.status = FrameMatch::Antialiased;
                    }
                    checked
                });
            // The incoming layer (and what is in it) at the end against itself as the slide opens, before stage 0's
            // builds that start by themselves: the frame it ends on is held to the same moment (`incoming`).
            let poses = batch.sample_pm(1000).zip(stage0).and_then(|(end, stage)| {
                let ends: BTreeMap<u64, &NodeProbe> = end
                    .nodes
                    .iter()
                    .filter(|p| {
                        animations
                            .iter()
                            .any(|a| a.node == p.node && a.role == "incoming")
                    })
                    .map(|p| (p.node, p))
                    .collect();
                let opening = opening_nodes(stage);
                moved::end_poses(&ends, &opening, &|n| node_name(&animations, n))
            });
            let to = incoming
                .as_ref()
                .zip(batch.frame(1000))
                .map(|(incoming, end)| check_end(input, incoming, end, poses.as_ref()));
            if failed(from.as_ref()) {
                problems.push((
                    "transition",
                    TimelineField::Transition,
                    "its first frame is not the outgoing slide".to_owned(),
                ));
            }
            if failed(to.as_ref()) {
                problems.push((
                    "transition",
                    TimelineField::Transition,
                    format!(
                        "its last frame is not the incoming slide: {}",
                        end_detail(poses.as_ref())
                    ),
                ));
            }
            (from, to)
        }
        None => (None, None),
    };
    for (_, field, problem) in &problems {
        mismatches.push(TimelineMismatch {
            step: None,
            field: *field,
            expected: String::new(),
            actual: problem.clone(),
        });
    }
    let effect = EffectRef {
        id: transition.native.clone(),
        fallback: transition.fallback.clone(),
        played: match played {
            Played::Native(_) => EffectPlay::Native,
            Played::Fallback(_) => EffectPlay::Fallback,
            Played::Nothing => EffectPlay::Unsupported,
        },
    };
    let mut failing: Vec<String> = problems
        .iter()
        .map(|(check, ..)| (*check).to_owned())
        .collect();
    failing.sort();
    failing.dedup();
    let mut rows = Vec::new();
    if let Some(batch) = &seen.transition {
        let (Some(t0), Some(t1000)) = (batch.frame(0), batch.frame(1000)) else {
            return (None, rows, None);
        };
        for pm in [250u16, 500, 750] {
            let Some(frame) = batch.frame(pm) else {
                continue;
            };
            // Motion is any change past antialiasing: a dissolve at a quarter is close to its start, yet moving.
            let same = |a: &Sha256, b: &Sha256| a == b || (input.frames.antialiased)(a, b);
            let moving = !same(frame, t0) && !same(frame, t1000);
            let outcome = if !failing.is_empty() {
                MotionOutcome::MotionMismatch
            } else if matches!(played, Played::Fallback(_)) {
                MotionOutcome::MotionFallback
            } else if moving {
                MotionOutcome::MotionMatch
            } else {
                MotionOutcome::MotionClose
            };
            rows.push((
                RowStep::Transition(pm),
                outcome,
                MotionRowMetrics {
                    effect: Some(effect.clone()),
                    frames: Some(FrameStrip {
                        start: t0.clone(),
                        mid: frame.clone(),
                        end: t1000.clone(),
                    }),
                    moving: Some(moving),
                    failures: problems
                        .iter()
                        .map(|(check, _, text)| format!("{check}: {text}"))
                        .collect(),
                    ..MotionRowMetrics::default()
                },
                frame.clone(),
                failing.clone(),
            ));
        }
    }
    let play = TransitionPlay {
        effect,
        name: transition.effect.clone(),
        params: transition.params.as_ref().map(Value::to_string),
        expected_ms: slidock_bench_metrics::quantise(expected_ms),
        actual_ms: seen
            .transition
            .as_ref()
            .map(|b| slidock_bench_metrics::quantise(b.duration_ms)),
        from,
        to,
    };
    let item = Item {
        item: ItemRef::Transition,
        native: transition.native.clone(),
        effect: transition.effect.clone(),
        params: transition.params.clone(),
        fallback: transition.fallback.clone(),
        played,
        problems: problems
            .into_iter()
            .map(|(check, _, text)| Problem::new(check, text))
            .collect(),
        accepted: Vec::new(),
    };
    (Some(play), rows, Some(item))
}

/// The stage frames of a stage: its batches' first, middle and last frames, or the settled frame throughout.
fn strip(stage: &StageSeen) -> FrameStrip {
    let first: Option<&Batch> = stage.batches.first();
    let last: Option<&Batch> = stage.batches.last();
    FrameStrip {
        start: first
            .and_then(|b| b.frame(0))
            .cloned()
            .unwrap_or_else(|| stage.settled.clone()),
        mid: first
            .and_then(|b| b.frame(500))
            .cloned()
            .unwrap_or_else(|| stage.settled.clone()),
        end: last
            .and_then(|b| b.frame(1000))
            .cloned()
            .unwrap_or_else(|| stage.settled.clone()),
    }
}

type StageRows = Vec<Row>;

/// An entrance of a stage: its target, where it ends, and its batch's last frame.
struct Entering {
    label: String,
    region: [f64; 4],
    end: Sha256,
}

fn stage_rows(
    input: &SlideInput<'_>,
    groups: &[u32],
    mismatches: &[TimelineMismatch],
    entering: &BTreeMap<u32, Vec<Entering>>,
) -> (StageRows, Vec<(u32, Sha256)>) {
    let SlideInput { slide, seen, .. } = input;
    let steps = &slide.timeline;
    let units = {
        let mut units: Vec<model::Unit> = steps.iter().flat_map(model::step_units).collect();
        units.sort();
        units.dedup();
        units
    };
    let unit_label = |unit: &model::Unit| match unit.paragraph {
        Some(p) => format!("{} ¶{p}", unit.element),
        None => unit.element.clone(),
    };
    let mut rows = Vec::new();
    let mut positions = Vec::new();
    let transition_end = seen.transition.as_ref().and_then(|b| b.frame(1000));
    for (k, stage) in seen.stages.iter().enumerate() {
        let k = k as u32;
        let frames = strip(stage);
        let mut failures: Vec<String> = Vec::new();
        // Each animated node at the end of the last batch that animated it, against itself once settled.
        let animations: Vec<&Animation> = stage
            .batches
            .iter()
            .flat_map(|b| b.animations.iter())
            .collect();
        let mut ends: BTreeMap<u64, &NodeProbe> = BTreeMap::new();
        for batch in &stage.batches {
            for probe in batch
                .sample_pm(1000)
                .map(|s| s.nodes.as_slice())
                .unwrap_or_default()
            {
                ends.insert(probe.node, probe);
            }
        }
        let poses = moved::end_poses(&ends, &stage.settled_nodes, &|n| node_name(&animations, n));
        let end_state = check_end(input, &stage.still, &frames.end, poses.as_ref());
        if !end_state.status.passes() {
            failures.push(format!("end-state: {}", end_detail(poses.as_ref())));
        }
        let settled = check_settled(input, &stage.still, &stage.settled);
        if !settled.status.passes() {
            failures.push("settled: pixels".to_owned());
        }
        let (ssim, delta_e) = if end_state.status != FrameMatch::Identical {
            (input.frames.scores)(&stage.still, &frames.end).unzip()
        } else {
            (None, None)
        };
        // The first frame against the state before the stage. Where an entrance ends, it may show its own first
        // keyframe (a motion that starts in view), but not the entrance already built: there it must differ either from
        // the state before or from its end (within the antialias bound, for a neighbour's raster). Elsewhere a frame
        // composited for the stage may rasterize anew.
        let before = if k == 0 {
            transition_end
        } else {
            seen.stages
                .get(k as usize - 1)
                .map(|previous| &previous.still)
        };
        let start_state = stage.batches.first().and(before).map(|before| {
            let strays = |a: &Sha256, b: &Sha256, region: Option<[f64; 4]>| {
                (input.frames.change)(a, b, region, ANTIALIAS_CHANNEL).map_or(0, |c| c.pixels)
            };
            let mut early: Vec<String> = Vec::new();
            let mut inside = 0u32;
            for entrance in entering.get(&k).into_iter().flatten() {
                let area = (entrance.region[2] * entrance.region[3]).max(0.0);
                let bound = ANTIALIAS_PIXELS.min(NOISE_PIXELS.max((area / 10.0) as u32));
                let region = Some(entrance.region);
                let changed = strays(before, &frames.start, region);
                inside = inside.saturating_add(changed);
                if changed > bound && strays(&entrance.end, &frames.start, region) <= bound {
                    early.push(format!("{} shows built", entrance.label));
                }
            }
            // Outside the entrances, the frame is the state before (within the antialias bound), or the static lane
            // classes it a `match` of it.
            let outside = strays(before, &frames.start, None).saturating_sub(inside);
            let pixels = (inside > 0 && outside <= ANTIALIAS_PIXELS)
                || check_paused(input, before, &frames.start).status.passes();
            if !early.is_empty() {
                failures.push(format!("start-state: {}", early.join(", ")));
            } else if !pixels {
                failures.push("start-state: pixels".to_owned());
            }
            early.is_empty() && pixels
        });
        let want = model::reveal(steps, k);
        let wrong: Vec<RevealMiss> = units
            .iter()
            .zip(stage.visible.iter())
            .filter_map(|(unit, actual)| {
                let expected = *want.get(unit)?;
                (Some(expected) != *actual).then(|| RevealMiss {
                    element: unit.element.clone(),
                    paragraph: unit.paragraph,
                    expected,
                    actual: *actual,
                })
            })
            .collect();
        if !wrong.is_empty() {
            let list: Vec<String> = wrong
                .iter()
                .map(|miss| {
                    let unit = model::Unit {
                        element: miss.element.clone(),
                        paragraph: miss.paragraph,
                    };
                    let shows = match miss.actual {
                        Some(true) => "shows",
                        Some(false) => "is hidden",
                        None => "is missing",
                    };
                    format!("{} {shows}", unit_label(&unit))
                })
                .collect();
            failures.push(format!("reveal: {}", list.join(", ")));
        }
        let mut changed: Vec<String> = Vec::new();
        let monotonic = (k > 0).then(|| {
            let previous = seen.stages.get(k as usize - 1);
            for (n, unit) in units.iter().enumerate() {
                let before = previous.and_then(|p| p.visible.get(n).copied().flatten());
                let now = stage.visible.get(n).copied().flatten();
                let explained = before == now
                    || steps.iter().zip(groups).any(|(step, g)| {
                        *g == k
                            && step.target.element == unit.element
                            && model::step_units(step).iter().any(|u| {
                                u == unit || (u.paragraph.is_none() && u.element == unit.element)
                            })
                    });
                if !explained {
                    changed.push(unit_label(unit));
                }
            }
            changed.is_empty()
        });
        if !changed.is_empty() {
            failures.push(format!("monotonic: {}", changed.join(", ")));
        }
        let in_group: BTreeSet<u32> = steps
            .iter()
            .zip(groups)
            .enumerate()
            .filter(|(_, (_, g))| **g == k)
            .map(|(i, _)| i as u32)
            .collect();
        let fell_back =
            steps.iter().zip(groups).enumerate().any(|(i, (step, g))| {
                *g == k
                    && (step.effect.is_none()
                        || input.seen.diagnostics.iter().any(|d| {
                            d.code == "RT0006" && d.message.contains(&format!(" step {i}:"))
                        }))
            });
        let mut failing: Vec<String> = failures
            .iter()
            .filter_map(|f| f.split_once(": ").map(|(check, _)| check.to_owned()))
            .collect();
        let group_label = format!("group {k}");
        for m in mismatches.iter().filter(|m| match (m.step, m.field) {
            (Some(step), _) => in_group.contains(&step),
            (
                None,
                TimelineField::Transition
                | TimelineField::Ir
                | TimelineField::Moves
                | TimelineField::Direction,
            ) => false,
            (None, TimelineField::Extra) => m.expected == group_label,
            (None, _) => true,
        }) {
            let name = field_name(m.field);
            failures.push(format!("{name}: {}", describe(m)));
            if !failing.contains(&name) {
                failing.push(name);
            }
        }
        let outcome = if !failing.is_empty() {
            MotionOutcome::MotionMismatch
        } else if fell_back {
            MotionOutcome::MotionFallback
        } else {
            MotionOutcome::MotionMatch
        };
        positions.push((k, stage.settled.clone()));
        rows.push((
            RowStep::Stage(k),
            outcome,
            MotionRowMetrics {
                frames: Some(frames),
                ssim,
                delta_e,
                end_state: Some(end_state),
                settled: Some(settled),
                start_state,
                reveal: Some(RevealCheck {
                    units: units.len() as u32,
                    wrong,
                }),
                monotonic,
                failures,
                ..MotionRowMetrics::default()
            },
            stage.settled.clone(),
            failing,
        ));
    }
    (rows, positions)
}

pub mod moved;

#[cfg(test)]
mod tests;
