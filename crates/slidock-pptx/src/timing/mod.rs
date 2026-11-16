//! Slide animations (`p:timing`, ECMA-376 Part 1 §19.5). On the parity path nothing is lowered: a
//! slide that animates anything (a timed effect or a build list) is reported with PX2401 and shows fully built
//! ([`drop_timing`]). With native motion, [`read_timeline`] lowers the main sequence into timeline steps:
//! - each click group of the main sequence (a `p:par` whose start waits for a click, or one starting on its own)
//!   holds time slots (a `p:par` starting at an offset), and each effect in a slot (a `p:cTn` with a preset, or a
//!   custom one with behaviours only) is one step, its offset the slot's start plus its own delay;
//! - the trigger comes from the click group and `nodeType`, the target from the first behaviour's `p:tgtEl`
//!   (`p:spTgt` → the slide's element of that id, `p:txEl/p:pRg` → a paragraph range, `p:bg` → the shape without
//!   its text), the delivery from `p:iterate` and the build list (`p:bldP@build="p"`), the duration from the
//!   behaviours (a 1 ms `p:set` is a switch, 0 ms long), and repeats, auto-reverse, accelerate/decelerate and the
//!   after effect (`afterEffect="1"`: hide or dim) from the effect's time node;
//! - a media node's mute and loop go to its video element ([`apply_playback`]);
//! - an after effect belongs to the effect before it in its own slot; one anywhere else (PowerPoint's "hide on next
//!   mouse click" sits in the next click group) is a step of its own, played from its behaviours, so its click
//!   stays;
//! - what the effect plays comes from its preset ([`effect`]).
//!
//! The tree is untrusted: it is read within the timing bounds (nodes, nesting, steps), every value is read bounded
//! ([`values`]), and an effect whose start condition names itself or closes a cycle is dropped. What the steps
//! cannot hold is reported: PX2403 for an effect of an interactive sequence (left out), PX2410 for an effect that
//! plays its fallback or its behaviours without one's own reversal (`autoRev` on a behaviour, which a track plays
//! forward and holds), PX2411 for timing that is dropped, clamped or invalid (media node options included), PX2412
//! for an option value read as its default, PX2413 for an approximation that plays because the preset's behaviours
//! compile to nothing.

mod behavior;
mod effect;
mod fmla;
mod keyframes;
mod values;

use std::collections::{BTreeMap, BTreeSet};

use slidock_core_api::visit::walk_elements;
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::effects::limits::{REPEAT_COUNT, STEPS_PER_SLIDE, TIMING_DEPTH, TIMING_NODES};
use slidock_ir::effects::{
    AfterEffect, BuildEffect, Delivery, Easing, NativeStep, Paint, Repeat, Stagger, StaggerAmount,
    StaggerOrder, StepKind, StepTiming,
};
use slidock_ir::elements::MediaKind;
use slidock_ir::features::FeatureId;
use slidock_ir::formats::IdPrefix;
use slidock_ir::frame::Frame;
use slidock_ir::ids::{ElementId, IdPart, node_id};
use slidock_ir::presentation::SlideSize;
use slidock_ir::source::Element as IrElement;
use slidock_ir::timeline::{StepEffect, StepFallback, StepTarget, TimelineStep, Trigger};
use slidock_ir::units::{Ms, Ratio, UnitRatio, pt_to_px};
use slidock_presets::Unknown;

use crate::context::Lowering;
use crate::report::Report;
use crate::xml::{El, Element, Ns};

use behavior::{Behavior, Part, Target, Value};
use effect::{Played, Preset};
use keyframes::{ShapeBox, Stage};
use values::{Hostile, Read, RepeatCount, Time};

/// The duration a step plays when its own does not read: PowerPoint's default for most presets (0.5 s).
pub(crate) const DEFAULT_DURATION: Ms = Ms(500);

/// Whether a timing tree animates anything: a timed effect or a build list.
fn animates(timing: &Element) -> bool {
    timing.kids().any(|element| {
        (element.ns == Ns::P
            && ((&*element.local == "cTn" && element.attr("presetClass").is_some())
                || &*element.local == "bldLst"))
            || animates(element)
    })
}

pub(crate) fn drop_timing(root: &Element, ctx: &mut Lowering<'_>) {
    let Some(timing) = root.child(Ns::P, "timing") else {
        return;
    };
    if animates(timing) {
        ctx.seen(FeatureId::Animation);
        Report::report(
            ctx,
            codes::PX2401,
            DiagnosticSite::new().feature(FeatureId::Animation),
        );
    }
}

/// The prefixes of `mc:Choice` requirements the native motion reader understands.
const UNDERSTOOD: [&str; 6] = ["p14", "p15", "p159", "a14", "a16", "ahyp"];

/// The slide root's children with alternate content resolved for native motion: the first choice whose
/// requirements are all understood, else the fallback.
pub(crate) fn root_children(root: &Element) -> Vec<&Element> {
    let mut out = Vec::new();
    for kid in root.kids() {
        if !kid.is(Ns::Mc, "AlternateContent") {
            out.push(&**kid);
            continue;
        }
        let choice = kid.kids().find(|choice| {
            choice.is(Ns::Mc, "Choice")
                && choice
                    .attr("Requires")
                    .unwrap_or("")
                    .split_ascii_whitespace()
                    .all(|prefix| UNDERSTOOD.contains(&prefix))
        });
        let branch = choice.or_else(|| kid.child(Ns::Mc, "Fallback"));
        if let Some(branch) = branch {
            out.extend(branch.kids().map(|rc| &**rc));
        }
    }
    out
}

/// A step's start inside its slot: a delay, or a condition on another time node.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Start {
    Delay(Ms),
    On { node: String },
}

/// An effect node found in the main sequence, with where it sits.
struct Found<'x> {
    ctn: &'x Element,
    group: usize,
    auto: bool,
    /// Its slot's position in the group and whether it is the slot's first effect.
    slot: usize,
    first_in_slot: bool,
    /// The slot's start plus the starts of the containers in between.
    base: Ms,
    start: Start,
    delay: Ms,
    hostile: Vec<Hostile>,
    after: Option<&'x Element>,
    /// An after effect outside its effect's slot, played as a step of its own.
    own_after: bool,
    dropped: Option<Lost>,
}

impl Found<'_> {
    /// The preset the step plays: none for an after effect of its own, whose preset attributes are its effect's.
    fn preset(&self) -> Preset {
        if self.own_after {
            Preset::default()
        } else {
            Preset::of(self.ctn)
        }
    }
}

/// Why effects were not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lost {
    Nested,
    Interactive,
    Outside,
    Budget,
    Steps,
    SelfReference,
    Cycle,
    Media,
}

impl Lost {
    fn what(self) -> &'static str {
        match self {
            Lost::Interactive => "interactive",
            Lost::Steps | Lost::Budget => "slide",
            Lost::Nested | Lost::Outside => "nested",
            Lost::SelfReference | Lost::Cycle => "start",
            Lost::Media => "media",
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Lost::Nested => "an effect nested deeper than the timing depth limit is dropped",
            Lost::Interactive => "an effect a click on a shape triggers is dropped",
            Lost::Outside => "an effect outside the main sequence is dropped",
            Lost::Budget => "the time nodes past the timing node limit are dropped",
            Lost::Steps => "the effects past the step limit are dropped",
            Lost::SelfReference => "an effect whose start waits on itself is dropped",
            Lost::Cycle => "effects whose starts wait on each other in a cycle are dropped",
            Lost::Media => "a media node's options are not played",
        }
    }
}

/// The behaviours of an effect node.
fn behaviours(ctn: &Element, hostile: &mut Vec<Hostile>) -> Vec<Behavior> {
    ctn.child(Ns::P, "childTnLst")
        .kids()
        .into_iter()
        .filter(|kid| !time_node(kid))
        .map(|kid| behavior::read(kid, hostile))
        .collect()
}

/// The native id and target shape of an effect node playing `preset`, for what reports it.
fn describe(ctn: &Element, preset: &Preset) -> (String, Option<String>) {
    let number = |text: Option<&str>| text.and_then(|t| t.trim().parse::<u32>().ok()).unwrap_or(0);
    let class = preset
        .class
        .as_deref()
        .filter(|class| slidock_presets::pptx::PptxClass::parse(class).is_some())
        .unwrap_or("custom");
    let native = slidock_presets::pptx::build_id(
        class,
        number(preset.id.as_deref()),
        number(preset.subtype.as_deref()),
    );
    let mut ignored = Vec::new();
    let spid = behaviours(ctn, &mut ignored).iter().find_map(|b| {
        match b.common().and_then(|c| c.target.as_ref()) {
            Some(Target::Shape { spid, .. }) => Some(spid.clone()),
            _ => None,
        }
    });
    (native.to_string(), spid)
}

fn time_node(element: &Element) -> bool {
    element.ns == Ns::P && matches!(&*element.local, "par" | "seq" | "excl")
}

/// An effect node: a time node with a preset, or a custom one whose children are behaviours only.
fn is_effect(ctn: &Element) -> bool {
    if ctn.bool_attr("afterEffect") == Some(true) {
        return false;
    }
    if Preset::of(ctn).is_effect() {
        return true;
    }
    let kids = ctn.child(Ns::P, "childTnLst").kids();
    !kids.is_empty() && !kids.iter().any(|kid| time_node(kid))
}

/// The time nodes of a list.
fn time_children(ctn: Option<&Element>) -> Vec<&Element> {
    ctn.child(Ns::P, "childTnLst").kids()
}

/// The start of a time node: its first start condition's delay, or the condition on another node it names.
fn start_of(ctn: &Element, hostile: &mut Vec<Hostile>) -> (Start, Ms) {
    let Some(cond) = ctn
        .child(Ns::P, "stCondLst")
        .children(Ns::P, "cond")
        .into_iter()
        .next()
    else {
        return (Start::Delay(Ms(0)), Ms(0));
    };
    let delay = match values::time("delayMs", cond.attr("delay"), hostile) {
        Ok(Some(Time::Ms(ms))) => ms,
        Ok(None) | Err(()) => Ms(0),
        Ok(Some(Time::Indefinite)) => {
            hostile.push(Hostile {
                field: "delayMs",
                native: "indefinite".to_owned(),
                read: Read::Invalid,
                limit: None,
            });
            Ms(0)
        }
    };
    match cond.child(Ns::P, "tn").and_then(|tn| tn.attr("val")) {
        Some(node) if cond.attr("evt").is_some() => (
            Start::On {
                node: node.trim().to_owned(),
            },
            delay,
        ),
        _ => (Start::Delay(delay), delay),
    }
}

fn add(a: Ms, b: Ms) -> Ms {
    Ms(a.0.saturating_add(b.0).min(Ms::MAX))
}

/// Where a slot's effects sit.
#[derive(Clone)]
struct Place {
    group: usize,
    auto: bool,
    slot: usize,
    base: Ms,
    hostile: Vec<Hostile>,
}

/// The walk over the time-node tree, within its bounds.
#[derive(Default)]
struct Walk<'x> {
    visited: usize,
    found: Vec<Found<'x>>,
    lost: Vec<(Lost, String, Option<String>, Option<String>)>,
    media: Vec<MediaNode>,
    over_budget: bool,
}

/// A media node's playback options: mute and loop, which a video element holds, and the others, which the deck
/// does not play.
/// Whether a media node's `vol` (ST_PositiveFixedPercentage: thousandths of a percent, or a percentage) is 0. A volume
/// that cannot be read is heard.
fn silent_volume(vol: Option<&str>) -> bool {
    vol.map(str::trim)
        .map(|vol| vol.strip_suffix('%').unwrap_or(vol))
        .and_then(|vol| vol.parse::<f64>().ok())
        .is_some_and(|vol| vol == 0.0)
}

struct MediaNode {
    native: String,
    spid: Option<String>,
    mute: bool,
    looped: bool,
    others: Vec<&'static str>,
}

/// The playback a media node gives a video element: muted, looped.
pub(crate) type Playback = (ElementId, bool, bool);

impl<'x> Walk<'x> {
    /// Counts a node; false once the node budget is spent.
    fn visit(&mut self) -> bool {
        self.visited += 1;
        if self.visited > TIMING_NODES {
            if !self.over_budget {
                self.over_budget = true;
                self.lost.push((Lost::Budget, String::new(), None, None));
            }
            return false;
        }
        true
    }

    /// Records every effect under `node` as lost, searched without recursion, within the node budget.
    fn lose_all(&mut self, node: &'x Element, why: Lost) {
        let mut stack = vec![node];
        while let Some(at) = stack.pop() {
            if !self.visit() {
                return;
            }
            if at.is(Ns::P, "cTn") && is_effect(at) {
                let (native, spid) = describe(at, &Preset::of(at));
                self.lost.push((why, native, spid, None));
                continue;
            }
            let kids: Vec<&Element> = at.kids().map(|rc| &**rc).collect();
            stack.extend(kids.into_iter().rev().filter(|kid| {
                time_node(kid) || kid.is(Ns::P, "cTn") || kid.is(Ns::P, "childTnLst")
            }));
        }
    }

    fn root(&mut self, timing: &'x Element) {
        let Some(root) = timing
            .child(Ns::P, "tnLst")
            .child(Ns::P, "par")
            .child(Ns::P, "cTn")
        else {
            return;
        };
        let mut main = false;
        for kid in time_children(Some(root)) {
            if !self.visit() {
                return;
            }
            let ctn = Some(kid).child(Ns::P, "cTn");
            match (&*kid.local, ctn.attr("nodeType")) {
                ("seq", Some("mainSeq")) if !main && kid.ns == Ns::P => {
                    main = true;
                    self.main_sequence(ctn, 2);
                }
                ("seq", Some("interactiveSeq")) => self.lose_all(kid, Lost::Interactive),
                _ if time_node(kid) => self.lose_all(kid, Lost::Outside),
                ("video" | "audio", _) if kid.ns == Ns::P => self.media(kid),
                _ => {}
            }
        }
    }

    /// A media node (`p:video`/`p:audio` with `p:cMediaNode`): its mute (or a volume of 0) and loop (`repeatCount`
    /// other than once) go to its video element; what else it asks of playback (a span of several slides, hide when
    /// stopped) is reported. Another volume is not: the deck plays media at the level the viewer sets, and PowerPoint
    /// writes a volume on every node.
    fn media(&mut self, node: &'x Element) {
        let Some(media) = node.child(Ns::P, "cMediaNode") else {
            return;
        };
        let ctn = Some(media).child(Ns::P, "cTn");
        let mut ignored = Vec::new();
        let looped = ctn.attr("repeatCount").is_some()
            && values::repeat_count(ctn.attr("repeatCount"), &mut ignored)
                != Some(RepeatCount::Thousandths(1_000));
        let others: Vec<&str> = [
            (
                "slides",
                media
                    .attr("numSld")
                    .is_some_and(|slides| slides.trim() != "1"),
            ),
            (
                "hideWhenStopped",
                media.bool_attr("showWhenStopped") == Some(false),
            ),
        ]
        .into_iter()
        .filter_map(|(option, asked)| asked.then_some(option))
        .collect();
        let spid = Some(media)
            .child(Ns::P, "tgtEl")
            .child(Ns::P, "spTgt")
            .attr("spid")
            .map(|spid| spid.trim().to_owned());
        self.media.push(MediaNode {
            native: format!("pptx:{}", node.local),
            spid,
            mute: media.bool_attr("mute") == Some(true) || silent_volume(media.attr("vol")),
            looped,
            others,
        });
    }

    fn main_sequence(&mut self, seq: Option<&'x Element>, depth: usize) {
        for (group, kid) in time_children(seq).into_iter().enumerate() {
            if !self.visit() {
                return;
            }
            if !kid.is(Ns::P, "par") {
                self.lose_all(kid, Lost::Outside);
                continue;
            }
            if depth + 1 > TIMING_DEPTH {
                self.lose_all(kid, Lost::Nested);
                continue;
            }
            let ctn = Some(kid).child(Ns::P, "cTn");
            let conds = ctn.child(Ns::P, "stCondLst").children(Ns::P, "cond");
            let waits = conds
                .iter()
                .any(|cond| cond.attr("delay") == Some("indefinite"));
            let auto = conds
                .iter()
                .any(|cond| matches!(cond.attr("evt"), Some("onBegin" | "begin")))
                || (!conds.is_empty() && !waits);
            for (slot, slot_par) in time_children(ctn).into_iter().enumerate() {
                if !self.visit() {
                    return;
                }
                if !slot_par.is(Ns::P, "par") {
                    self.lose_all(slot_par, Lost::Outside);
                    continue;
                }
                if depth + 2 > TIMING_DEPTH {
                    self.lose_all(slot_par, Lost::Nested);
                    continue;
                }
                let mut hostile = Vec::new();
                let begin = match slot_par
                    .child(Ns::P, "cTn")
                    .map(|ctn| start_of(ctn, &mut hostile))
                {
                    Some((Start::Delay(ms), _)) => ms,
                    _ => Ms(0),
                };
                let before = self.found.len();
                let place = Place {
                    group,
                    auto,
                    slot,
                    base: begin,
                    hostile,
                };
                self.slot(slot_par, &place, depth + 2);
                if let Some(first) = self.found.get_mut(before) {
                    first.first_in_slot = true;
                }
            }
        }
    }

    /// The effects of a slot, or of a container inside one, at `depth`.
    fn slot(&mut self, par: &'x Element, place: &Place, depth: usize) {
        for kid in time_children(par.child(Ns::P, "cTn").map(|rc| &**rc)) {
            if !self.visit() {
                return;
            }
            if !time_node(kid) {
                continue;
            }
            if depth + 1 > TIMING_DEPTH {
                self.lose_all(kid, Lost::Nested);
                continue;
            }
            let Some(ctn) = kid.child(Ns::P, "cTn").map(|rc| &**rc) else {
                continue;
            };
            let own_after = ctn.bool_attr("afterEffect") == Some(true);
            if own_after {
                let owner = self.found.last_mut().filter(|last| {
                    last.after.is_none() && last.group == place.group && last.slot == place.slot
                });
                if let Some(owner) = owner {
                    owner.after = Some(ctn);
                    continue;
                }
            }
            if !kid.is(Ns::P, "par") {
                self.lose_all(kid, Lost::Outside);
                continue;
            }
            let mut hostile = place.hostile.clone();
            let (start, delay) = start_of(ctn, &mut hostile);
            if own_after || is_effect(ctn) {
                self.found.push(Found {
                    ctn,
                    group: place.group,
                    auto: place.auto,
                    slot: place.slot,
                    first_in_slot: false,
                    base: place.base,
                    start,
                    delay,
                    hostile,
                    after: None,
                    own_after,
                    dropped: None,
                });
                continue;
            }
            let base = match start {
                Start::Delay(ms) => add(place.base, ms),
                Start::On { .. } => place.base,
            };
            let inner = Place {
                base,
                hostile,
                ..place.clone()
            };
            self.slot(kid, &inner, depth + 1);
        }
    }
}

/// Marks the effects whose start condition names themselves or closes a cycle of conditions.
fn drop_cycles(found: &mut [Found<'_>]) {
    let index: BTreeMap<String, usize> = found
        .iter()
        .enumerate()
        .filter_map(|(i, f)| f.ctn.attr("id").map(|id| (id.trim().to_owned(), i)))
        .collect();
    let next = |i: usize| match found.get(i).map(|f| &f.start) {
        Some(Start::On { node }) => index.get(node).copied(),
        _ => None,
    };
    let mut drops = Vec::new();
    for i in 0..found.len() {
        let mut at = next(i);
        if at == Some(i) {
            drops.push((i, Lost::SelfReference));
            continue;
        }
        for _ in 0..found.len() {
            match at {
                Some(j) if j == i => {
                    drops.push((i, Lost::Cycle));
                    break;
                }
                Some(j) => at = next(j),
                None => break,
            }
        }
    }
    for (i, why) in drops {
        if let Some(f) = found.get_mut(i) {
            f.dropped = Some(why);
        }
    }
}

/// The build list's paragraph builds: the shapes and groups `p:bldP@build="p"` names.
fn paragraph_builds(timing: &Element) -> BTreeSet<(String, String)> {
    timing
        .child(Ns::P, "bldLst")
        .children(Ns::P, "bldP")
        .into_iter()
        .filter(|bld| bld.attr("build") == Some("p"))
        .map(|bld| {
            (
                bld.attr("spid").unwrap_or("").trim().to_owned(),
                bld.attr("grpId").unwrap_or("0").trim().to_owned(),
            )
        })
        .collect()
}

/// The frames of the slide's own elements, group children included: the ones a step can target.
fn targetable(elements: &[IrElement]) -> BTreeMap<ElementId, Option<&Frame>> {
    walk_elements(elements)
        .map(|element| (element.id().clone(), element.frame().as_ref()))
        .collect()
}

fn stage_of(size: &SlideSize, frame: Option<&Frame>) -> Stage {
    let (w, h) = (size.w.to_f64(), size.h.to_f64());
    let shape = frame.filter(|_| w > 0.0 && h > 0.0).map(|frame| ShapeBox {
        x: (frame.x.to_f64() + frame.w.to_f64() / 2.0) / w,
        y: (frame.y.to_f64() + frame.h.to_f64() / 2.0) / h,
        w: frame.w.to_f64() / w,
        h: frame.h.to_f64() / h,
        rotation: frame.rotation.to_f64(),
    });
    Stage {
        w_px: pt_to_px(w),
        h_px: pt_to_px(h),
        shape,
    }
}

/// A seed for a step's formulas: FNV-1a of the slide's id, then the step's index.
fn seed(slide: &str, step: usize) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in slide.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash ^ step as u64
}

/// What reports one effect's losses, and whether any was lossy.
struct StepReport<'a, 'l> {
    ctx: &'a mut Lowering<'l>,
    node: Option<ElementId>,
    effect: String,
    spid: Option<String>,
    lossy: &'a mut bool,
}

impl StepReport<'_, '_> {
    fn site(&self) -> DiagnosticSite {
        DiagnosticSite::new()
            .node(self.node.clone())
            .data("effect", self.effect.as_str())
            .data_opt("spid", self.spid.clone())
    }

    fn timing(&mut self, what: &str, reason: &str) {
        *self.lossy = true;
        let site = self.site().data("what", what).data("reason", reason);
        self.ctx.reporter.report(codes::PX2411, site);
    }

    fn interactive(&mut self) {
        *self.lossy = true;
        let site = self.site().feature(FeatureId::Animation);
        self.ctx.reporter.report(codes::PX2403, site);
    }

    fn approximated(&mut self, reason: &str) {
        let site = self.site().data("reason", reason);
        self.ctx.reporter.report(codes::PX2413, site);
    }

    fn media(&mut self, reason: &str, options: &str) {
        *self.lossy = true;
        let site = self
            .site()
            .data("what", Lost::Media.what())
            .data("reason", reason)
            .data("options", options);
        self.ctx.reporter.report(codes::PX2411, site);
    }

    fn hostile(&mut self, hostile: &Hostile) {
        *self.lossy = true;
        let reason = match hostile.read {
            Read::Clamped => "a value past its limit is clamped",
            Read::Invalid => "a value that is not one reads as the default",
        };
        let site = self
            .site()
            .data("what", hostile.field)
            .data("reason", reason)
            .data("native", hostile.native.as_str())
            .data("read", hostile.read.token())
            .data_opt("limit", hostile.limit);
        self.ctx.reporter.report(codes::PX2411, site);
    }

    fn unknown(&mut self, unknown: &Unknown) {
        let site = self
            .site()
            .data("option", unknown.option)
            .data("value", unknown.value.as_str())
            .data("used", unknown.used);
        self.ctx.reporter.report(codes::PX2412, site);
    }

    /// A tier-2 effect that plays without part of its behaviours.
    fn partial(&mut self, play: &str) {
        let site = self.site().data("play", play);
        self.ctx.reporter.report(codes::PX2410, site);
    }

    fn unmapped(&mut self, fallback: StepFallback) {
        let play = match fallback {
            StepFallback::Appear => "appear",
            StepFallback::Dissolve => "dissolve",
            StepFallback::None => "none",
        };
        let site = self.site().data("play", play);
        self.ctx.reporter.report(codes::PX2410, site);
    }
}

/// The step's duration: the latest end of its behaviours (a switch ends where it starts), else its own `dur`; a
/// behaviour duration that does not read makes it [`DEFAULT_DURATION`].
fn duration(ctn: &Element, behaviors: &[Behavior], hostile: &mut Vec<Hostile>) -> Ms {
    if hostile
        .iter()
        .any(|h| h.field == "durationMs" && h.read == Read::Invalid)
    {
        return DEFAULT_DURATION;
    }
    let mut end: Option<Ms> = None;
    for behavior in behaviors {
        let Some(common) = behavior.common() else {
            continue;
        };
        let begin = match common.begin {
            Some(Time::Ms(ms)) => ms,
            _ => Ms(0),
        };
        let length = match common.dur {
            _ if behavior::is_switch(behavior) => Ms(0),
            // A behaviour's own reversal and repeat run it over more than one span.
            Some(Time::Ms(ms)) => common.retime(ms).0.length(ms).unwrap_or(Ms(Ms::MAX)),
            _ => continue,
        };
        let at = add(begin, length);
        end = Some(end.map_or(at, |end| end.max(at)));
    }
    if let Some(end) = end {
        return end;
    }
    match values::time("durationMs", ctn.attr("dur"), hostile) {
        Ok(Some(Time::Ms(ms))) => ms,
        Err(()) => DEFAULT_DURATION,
        Ok(None | Some(Time::Indefinite)) => Ms(0),
    }
}

/// Where an effect's switch to visible ends when it does not hold (`fill` remove, or none, with a duration, as
/// PowerPoint's and LibreOffice's Flash Once write it): the target shows only until then. `None` when every such
/// switch holds.
fn shown_only_while(behaviors: &[Behavior]) -> Option<Ms> {
    behaviors
        .iter()
        .filter(|behavior| behavior.visibility() == Some(true))
        .filter_map(Behavior::common)
        .filter(|common| !common.holds)
        .find_map(|common| {
            let begin = match common.begin {
                None => Ms(0),
                Some(Time::Ms(ms)) => ms,
                Some(Time::Indefinite) => return None,
            };
            let Some(Time::Ms(dur)) = common.dur else {
                return None;
            };
            Some(add(begin, dur))
        })
}

/// The after effect a node holds: hide, or dim the text to a colour.
fn after_effect(ctn: &Element, report: &mut StepReport<'_, '_>) -> Option<AfterEffect> {
    let mut ignored = Vec::new();
    for behavior in behaviours(ctn, &mut ignored) {
        if behavior.visibility() == Some(false) {
            return Some(AfterEffect::Hide);
        }
        if let Behavior::Set {
            to: Some(Value::Color(color)),
            ..
        } = &behavior
        {
            if let Some(color) = color {
                return Some(AfterEffect::Dim(*color));
            }
            report.unknown(&Unknown {
                option: "after",
                value: "a scheme colour".to_owned(),
                used: "keep",
            });
            return None;
        }
    }
    None
}

/// The repeat an effect node asks for: a count, until the next click or until the end of the slide. A count
/// `repeatDur` gives clamps to [`REPEAT_COUNT`]; a `repeatDur` shorter than the effect plays it whole, reported.
fn repeat_of(
    ctn: &Element,
    duration: Ms,
    hostile: &mut Vec<Hostile>,
    report: &mut StepReport<'_, '_>,
) -> Option<Repeat> {
    match values::repeat_count(ctn.attr("repeatCount"), hostile) {
        Some(RepeatCount::Indefinite) => {
            let until_click = ctn
                .child(Ns::P, "endCondLst")
                .children(Ns::P, "cond")
                .iter()
                .any(|cond| cond.attr("evt") == Some("onNext"));
            return Some(if until_click {
                Repeat::UntilNextClick
            } else {
                Repeat::UntilEndOfSlide
            });
        }
        Some(RepeatCount::Thousandths(1_000)) => return None,
        Some(RepeatCount::Thousandths(n)) => {
            return Some(Repeat::Count(Ratio::from_steps(
                i64::try_from(n).unwrap_or(0).saturating_mul(100),
            )));
        }
        None => {}
    }
    match values::time("repeat", ctn.attr("repeatDur"), hostile) {
        Ok(Some(Time::Indefinite)) => Some(Repeat::UntilEndOfSlide),
        Ok(Some(Time::Ms(total))) if duration.0 > 0 && total.0 > duration.0 => {
            let thousandths = u64::from(total.0) * 1_000 / u64::from(duration.0);
            let limit = u64::from(REPEAT_COUNT) * 1_000;
            if thousandths > limit {
                hostile.push(Hostile {
                    field: "repeat",
                    native: values::thousandths(thousandths),
                    read: Read::Clamped,
                    limit: Some(values::REPEAT_LIMIT),
                });
            }
            let count = f64::from(total.0) / f64::from(duration.0);
            Ratio::quantise(count.min(f64::from(REPEAT_COUNT)))
                .ok()
                .map(Repeat::Count)
        }
        Ok(Some(Time::Ms(total))) if total.0 < duration.0 => {
            report.timing(
                "repeat",
                "a repeat duration shorter than the effect plays the whole effect",
            );
            None
        }
        _ => None,
    }
}

/// The easing of an effect node's `accel`/`decel`; values that do not read, or that add up to more than the
/// duration, play linear and are reported.
fn easing_of(ctn: &Element, report: &mut StepReport<'_, '_>) -> Option<Easing> {
    let written = || {
        format!(
            "accel={} decel={}",
            ctn.attr("accel").unwrap_or("0"),
            ctn.attr("decel").unwrap_or("0")
        )
    };
    let read = (
        values::fraction(ctn.attr("accel")),
        values::fraction(ctn.attr("decel")),
    );
    let (Ok(accel), Ok(decel)) = read else {
        report.unknown(&Unknown {
            option: "accel",
            value: written(),
            used: "linear",
        });
        return None;
    };
    let (accel, decel) = (accel.unwrap_or(0), decel.unwrap_or(0));
    if accel == 0 && decel == 0 {
        return None;
    }
    if accel + decel > 100_000 {
        report.unknown(&Unknown {
            option: "accel",
            value: written(),
            used: "linear",
        });
        return None;
    }
    Some(Easing::AccelDecel {
        accel: UnitRatio::from_steps(i64::from(accel)),
        decel: UnitRatio::from_steps(i64::from(decel)),
    })
}

/// The stagger of a step delivered by word or character (`p:iterate` with `p:tmAbs` or `p:tmPct`); an interval
/// that does not read (out of range, `indefinite`, missing its value) is no stagger, recorded in `hostile`.
fn stagger_of(iterate: Option<&Element>, hostile: &mut Vec<Hostile>) -> Option<Stagger> {
    let iterate = iterate?;
    let invalid = |text: Option<&str>, hostile: &mut Vec<Hostile>| {
        hostile.push(Hostile {
            field: "stagger",
            native: text.unwrap_or("").to_owned(),
            read: Read::Invalid,
            limit: None,
        });
        None
    };
    let per_unit = if let Some(abs) = iterate.child(Ns::P, "tmAbs") {
        let text = abs.attr("val");
        match values::time("stagger", text, hostile) {
            Ok(Some(Time::Ms(ms))) => StaggerAmount::Ms(ms),
            Ok(None | Some(Time::Indefinite)) => return invalid(text, hostile),
            Err(()) => return None,
        }
    } else {
        let text = iterate.child(Ns::P, "tmPct")?.attr("val");
        match values::fraction(text) {
            Ok(Some(fraction)) => {
                StaggerAmount::Fraction(UnitRatio::from_steps(i64::from(fraction)))
            }
            Ok(None) | Err(()) => return invalid(text, hostile),
        }
    };
    let order = if iterate.bool_attr("backwards") == Some(true) {
        StaggerOrder::Backward
    } else {
        StaggerOrder::Forward
    };
    Some(Stagger { per_unit, order })
}

/// The slide's timeline steps (native motion): see the module doc.
pub(crate) fn read_timeline(
    root: &Element,
    elements: &[IrElement],
    size: &SlideSize,
    ctx: &mut Lowering<'_>,
) -> (Vec<TimelineStep>, Vec<Playback>) {
    let Some(timing) = root_children(root)
        .into_iter()
        .find(|kid| kid.is(Ns::P, "timing"))
    else {
        return (Vec::new(), Vec::new());
    };
    let mut walk = Walk::default();
    walk.root(timing);
    let Walk {
        mut found,
        lost,
        media,
        ..
    } = walk;
    let mut lost = lost;
    if found.len() > STEPS_PER_SLIDE {
        for extra in found.split_off(STEPS_PER_SLIDE) {
            let (native, spid) = describe(extra.ctn, &extra.preset());
            lost.push((Lost::Steps, native, spid, None));
        }
    }
    drop_cycles(&mut found);
    let paragraph_builds = paragraph_builds(timing);
    let targets = targetable(elements);
    let body = ctx.container.body().to_owned();
    let mut lossy = false;
    let videos: BTreeSet<ElementId> = walk_elements(elements)
        .filter(|element| {
            matches!(element, IrElement::Media(media) if media.media_kind == MediaKind::Video)
        })
        .map(|element| element.id().clone())
        .collect();
    let mut playback = Vec::new();
    for node in media {
        let video = node
            .spid
            .as_deref()
            .and_then(|spid| node_id(IdPrefix::Pp, &[IdPart::Str(&body), IdPart::Str(spid)]).ok())
            .filter(|id| videos.contains(id));
        let mut options = Vec::new();
        match video {
            Some(video) => {
                if node.mute || node.looped {
                    playback.push((video, node.mute, node.looped));
                }
            }
            None => {
                options.extend(node.mute.then_some("mute"));
                options.extend(node.looped.then_some("loop"));
            }
        }
        options.extend(node.others);
        if !options.is_empty() {
            lost.push((Lost::Media, node.native, node.spid, Some(options.join(" "))));
        }
    }
    for (why, effect, spid, options) in lost {
        let mut report = StepReport {
            ctx: &mut *ctx,
            node: None,
            effect,
            spid,
            lossy: &mut lossy,
        };
        match (why, options) {
            (_, Some(options)) => report.media(why.reason(), &options),
            (Lost::Interactive, None) => report.interactive(),
            (_, None) => report.timing(why.what(), why.reason()),
        }
    }
    let mut steps: Vec<TimelineStep> = Vec::new();
    let mut open_groups: BTreeSet<usize> = BTreeSet::new();
    for (index, found) in found.iter().enumerate() {
        let preset = found.preset();
        let (effect_name, written_spid) = describe(found.ctn, &preset);
        let mut report = StepReport {
            ctx: &mut *ctx,
            node: None,
            effect: effect_name,
            spid: written_spid,
            lossy: &mut lossy,
        };
        if let Some(why) = found.dropped {
            report.timing(why.what(), why.reason());
            continue;
        }
        let ctn = found.ctn;
        let mut hostile = found.hostile.clone();
        let behaviors = behaviours(ctn, &mut hostile);
        let target = behaviors
            .iter()
            .find_map(|b| b.common().and_then(|c| c.target.clone()));
        let (spid, part) = match target {
            Some(Target::Shape { spid, part }) => (spid, part),
            Some(Target::Other(kind)) => {
                report.timing(
                    "target",
                    &format!("an effect on a {kind} target is dropped"),
                );
                continue;
            }
            None => {
                report.timing("target", "an effect that names no target is dropped");
                continue;
            }
        };
        let element = node_id(IdPrefix::Pp, &[IdPart::Str(&body), IdPart::Str(&spid)])
            .ok()
            .filter(|id| targets.contains_key(id));
        let Some(element) = element else {
            report.timing(
                "target",
                "an effect on a shape that is not on the slide is dropped",
            );
            continue;
        };
        let frame = targets.get(&element).copied().flatten();
        let stage = stage_of(size, frame);
        let duration_ms = duration(ctn, &behaviors, &mut hostile);
        let mut played: Played =
            effect::play(&preset, &behaviors, duration_ms, &stage, seed(&body, index));
        report.node = Some(element.clone());
        report.effect = played.id.to_string();
        // An entrance whose visibility switch does not hold (Flash Once) shows its target while the switch lasts,
        // then hides it.
        let shown = (played.kind == StepKind::Entrance)
            .then(|| shown_only_while(&behaviors))
            .flatten();
        let flashes = shown.is_some();
        // An appear is a switch: whatever else its behaviours do is not played.
        let duration_ms = match shown {
            Some(end) => end,
            None if matches!(played.effect, Some(BuildEffect::Appear)) => Ms(0),
            None => duration_ms,
        };
        let (paragraphs, background) = match part {
            Part::Whole => (None, false),
            Part::Paragraphs(st, end) => (Some((st, end)), false),
            Part::Background => (None, true),
            Part::Characters => {
                report.timing("target", "a character range plays on the whole shape");
                (None, false)
            }
            Part::Inside(inside) => {
                report.timing(
                    "target",
                    &format!("a {inside} part plays on the whole object"),
                );
                (None, false)
            }
        };
        let repeat = repeat_of(ctn, duration_ms, &mut hostile, &mut report);
        let easing = easing_of(ctn, &mut report);
        let iterate = ctn.child(Ns::P, "iterate").map(|rc| &**rc);
        let unit = match iterate.and_then(|it| it.attr("type")).unwrap_or("el") {
            "wd" => Some(Delivery::ByWord),
            "lt" => Some(Delivery::ByChar),
            "el" => None,
            other => {
                report.unknown(&Unknown {
                    option: "iterate",
                    value: other.to_owned(),
                    used: "el",
                });
                None
            }
        };
        let group_id = ctn.attr("grpId").unwrap_or("0").trim().to_owned();
        let by_paragraph =
            paragraphs.is_some() && paragraph_builds.contains(&(spid.clone(), group_id));
        let delivery = unit
            .or_else(|| by_paragraph.then_some(Delivery::ByParagraph))
            .or(played.delivery)
            .unwrap_or(Delivery::All);
        let stagger = unit.and_then(|_| stagger_of(iterate, &mut hostile));
        // A word or a character has no fill or line of its own: a step built by unit plays its text's tracks.
        if unit.is_some()
            && let Some(BuildEffect::Behaviors(tracks)) = &mut played.effect
        {
            tracks.retain(|track| !matches!(track.paint(), Some(Paint::Fill | Paint::Line)));
            if tracks.is_empty() {
                played.effect = None;
                played.unmapped = Some(effect::Unmapped::NoEntry);
            }
        }
        let after = found
            .after
            .and_then(|after| after_effect(after, &mut report))
            .or(flashes.then_some(AfterEffect::Hide));
        if found.own_after && matches!(after_effect(ctn, &mut report), Some(AfterEffect::Dim(_))) {
            report.timing("after", "a dim on a later click is not played");
        }
        let offset = match found.start {
            Start::Delay(delay) => Some(add(found.base, delay)),
            Start::On { .. } => None,
        };
        // The first step left in a click group takes the click; the rest follow `nodeType`, or their place when it
        // says nothing usable.
        let first_in_group = open_groups.insert(found.group);
        let placed = if found.slot > 0 && found.first_in_slot {
            Trigger::AfterPrevious
        } else {
            Trigger::WithPrevious
        };
        let trigger = if first_in_group && !found.auto {
            Trigger::Click
        } else {
            match ctn.attr("nodeType") {
                Some("withEffect") => Trigger::WithPrevious,
                Some("afterEffect") => Trigger::AfterPrevious,
                Some("clickEffect") | None => placed,
                Some(other) => {
                    report.unknown(&Unknown {
                        option: "nodeType",
                        value: other.to_owned(),
                        used: "its place",
                    });
                    placed
                }
            }
        };
        for h in &hostile {
            report.hostile(h);
        }
        for unknown in &played.unknown {
            report.unknown(unknown);
        }
        if let Some(reason) = &played.conflict {
            report.approximated(reason);
        }
        if played.unmapped.is_some() {
            report.unmapped(played.fallback);
        } else if let Some(effect) = played.effect.as_ref().filter(|_| played.approximated) {
            report.approximated(&format!(
                "{} only approximates the preset, and its behaviours compile to nothing",
                effect.name()
            ));
        } else if !played.lost.is_empty() {
            report.partial(&format!(
                "its behaviours without {}",
                played.lost.join(", ")
            ));
        }
        let timing = StepTiming {
            offset_ms: offset,
            easing,
            stagger,
            repeat,
            auto_reverse: ctn.bool_attr("autoRev") == Some(true),
            after,
        };
        let mut step = TimelineStep {
            target: StepTarget {
                element,
                paragraphs,
                background,
            },
            trigger,
            delay_ms: found.delay,
            duration_ms,
            kind: played.kind,
            effect: StepEffect {
                id: played.id.effect_id(),
                fallback: played.fallback,
                native: played.effect.map(|effect| {
                    Box::new(NativeStep {
                        id: played.id,
                        effect,
                    })
                }),
            },
            delivery,
            timing: (!timing.is_default()).then(|| Box::new(timing)),
        };
        if step.effect.native.is_some() && !slidock_ir::validate::validate(&step).is_empty() {
            step.effect.native = None;
            step.effect.fallback = slidock_presets::kind_fallback(step.kind);
            report.unmapped(step.effect.fallback);
        }
        steps.push(step);
    }
    if !steps.is_empty() {
        ctx.seen(FeatureId::SlideTimeline);
    }
    if lossy {
        ctx.seen(FeatureId::Animation);
    }
    (steps, playback)
}

/// Gives each video element the playback its media node asks for (muted, looped).
pub(crate) fn apply_playback(elements: &mut [IrElement], playback: &[Playback]) {
    if playback.is_empty() {
        return;
    }
    let mut stack: Vec<&mut IrElement> = elements.iter_mut().collect();
    while let Some(element) = stack.pop() {
        match element {
            IrElement::Media(media) => {
                for (id, muted, looped) in playback {
                    if *id == media.id {
                        media.muted |= *muted;
                        media.looped |= *looped;
                    }
                }
            }
            IrElement::Group(group) => stack.extend(group.children.iter_mut()),
            _ => {}
        }
    }
}
