//! A slide's builds as timeline steps. The steps' IR fields: chunk order, a
//! click step per chunk that waits for a click and an after-previous step per automatic one, the chunk's delay and
//! duration, the kind from the animation type, the delivery, and one paragraph per chunk for a build in several chunks.
//!
//! The native data adds each step's vocabulary entry and timing. An automatic chunk starts "with" or "after" the
//! step before it ([`start_of`]) by its `referent`: false with, true after. Keynote 14.5 writes that pair for
//! PowerPoint's with-previous and after-previous (its import of the PPTX motion probe), beside an `eventTrigger` of
//! 0 (with), 1 (click) or 2 (after); older decks state 1 on every chunk, automatic ones included, so the trigger
//! decides nothing alone and a 0 or 2 that disagrees with the chunk is reported. A build's `delayAutomaticWith` or
//! `delayAutomaticAfter`, which no deck written by Keynote has shown yet, also names the start
//! and gives the delay when the chunk states none; where it disagrees with the referent it wins (KN1404). The IR
//! keeps the after-previous trigger, so every step carries its absolute start inside its click group
//! (`timing.offsetMs`): a click step starts at its delay, a with step at the previous step's start plus its delay,
//! an after step at the previous step's end (repeats included) plus its delay.
//!
//! The parity path reads the delivery from the effect's name and the chunking ([`delivery`]). The deck path reads
//! Keynote's own text delivery first ([`native_delivery`]): `custom_textDelivery` by object plays the text whole,
//! whatever the effect's name says.
//!
//! A chunk's `index` places it in its build. The IR's paragraph is the count of the build's chunks so far; an index
//! that disagrees is reported (KN1404).

use std::collections::BTreeMap;

use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::effects::{Repeat, StepTiming, limits};
use slidock_ir::features::FeatureId;
use slidock_ir::formats::{EffectId, FileFormat, SourceFormat};
use slidock_ir::ids::NodeId;
use slidock_ir::origin::Origin;
use slidock_ir::source::Element;
use slidock_ir::timeline::{
    Delivery, StepEffect, StepFallback, StepKind, StepTarget, TimelineStep, Trigger,
};
use slidock_ir::units::Ms;

use super::effects::{Findings, native_build, step_fallback};
use crate::context::Lowering;
use crate::num::ms;
use crate::proto::{kn, tsp};

/// `BuildAttributesArchive.BuildAttributesTextDelivery`.
const UNDEFINED: i32 = 0;
const BY_OBJECT: i32 = 1;
const BY_WORD: i32 = 2;
const BY_CHARACTER: i32 = 3;
const BY_LINE: i32 = 4;

pub fn effect_id(native: &str) -> EffectId {
    EffectId::new(SourceFormat::File(FileFormat::Keynote), native)
}

fn ids_of(elements: &[Element], out: &mut BTreeMap<String, NodeId>) {
    for element in elements {
        if let Origin::Source {
            object: Some(object),
            ..
        } = element.origin()
            && !object.is_empty()
        {
            out.insert(object.clone(), element.id().clone());
        }
        if let Element::Group(group) = element {
            ids_of(&group.children, out);
        }
    }
}

fn step_kind(animation_type: Option<&str>, effect: &str) -> StepKind {
    if effect == "apple:movie-start" {
        return StepKind::Media;
    }
    match animation_type.map(str::to_lowercase).as_deref() {
        Some("in") => StepKind::Entrance,
        Some("out") => StepKind::Exit,
        _ => StepKind::Emphasis,
    }
}

/// The parity path's delivery: by character for that text delivery or an effect named `… character`, then by word, then
/// by paragraph for a line delivery or a paragraph or bullet chunking, else all at once.
fn delivery(build: &kn::BuildArchive, effect: &str) -> Delivery {
    let text = text_delivery(build);
    if text == Some(BY_CHARACTER) || effect.ends_with(" character") {
        Delivery::ByChar
    } else if text == Some(BY_WORD) {
        Delivery::ByWord
    } else if text == Some(BY_LINE) || by_chunks(build) {
        Delivery::ByParagraph
    } else {
        Delivery::All
    }
}

fn text_delivery(build: &kn::BuildArchive) -> Option<i32> {
    build
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.custom_text_delivery)
}

/// Whether the build's chunking (`BuildArchive.delivery`, a UI string) splits the text by paragraph or bullet.
fn by_chunks(build: &kn::BuildArchive) -> bool {
    let named = build
        .delivery
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    named.contains("paragraph") || named.contains("bullet")
}

/// The delivery the deck path plays: Keynote's text delivery when it states one (by object keeps the chunk whole,
/// by paragraph when the chunking splits it), the parity path's reading ([`delivery`]) when it states none. An unknown
/// value keeps that reading, reported (KN1404).
fn native_delivery(build: &kn::BuildArchive, effect: &str, findings: &mut Findings) -> Delivery {
    match text_delivery(build) {
        None | Some(UNDEFINED) => delivery(build, effect),
        Some(BY_OBJECT) if by_chunks(build) => Delivery::ByParagraph,
        Some(BY_OBJECT) => Delivery::All,
        Some(BY_WORD) => Delivery::ByWord,
        Some(BY_CHARACTER) => Delivery::ByChar,
        Some(BY_LINE) => Delivery::ByParagraph,
        Some(other) => {
            findings.approximate(format!(
                "the text delivery {other} is unknown; it plays {}",
                delivery_name(delivery(build, effect))
            ));
            delivery(build, effect)
        }
    }
}

fn delivery_name(delivery: Delivery) -> &'static str {
    match delivery {
        Delivery::All => "all at once",
        Delivery::ByParagraph => "by paragraph",
        Delivery::ByWord => "by word",
        Delivery::ByChar => "by character",
    }
}

/// `BuildAttributesArchive.eventTrigger` as Keynote 14.5 writes it (its import of the PPTX motion probe): with the
/// step before, on a click, after the step before.
mod event {
    pub const WITH: u32 = 0;
    pub const CLICK: u32 = 1;
    pub const AFTER: u32 = 2;
}

/// How a chunk starts against the step before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Start {
    Click,
    After,
    With,
}

impl Start {
    fn name(self) -> &'static str {
        match self {
            Start::Click => "click",
            Start::After => "after",
            Start::With => "with",
        }
    }
}

/// A build's automatic delay in seconds, when it states a usable one; a value that is not finite or is negative
/// is reported and left unread.
fn automatic_delay(value: Option<f64>, findings: &mut Findings) -> Option<f64> {
    let value = value?;
    if value.is_finite() && value >= 0.0 {
        Some(value)
    } else {
        findings.approximate(format!(
            "the automatic delay {} is out of range; the chunk's referent plays",
            slidock_canon::to_js_string(value)
        ));
        None
    }
}

/// How a chunk starts, and the delay its build states for an automatic start (used when the chunk states none).
fn start_of(
    chunk: &kn::BuildChunkArchive,
    attributes: Option<&kn::BuildAttributesArchive>,
    findings: &mut Findings,
) -> (Start, Option<f64>) {
    let (start, delay) = chunk_start(chunk, attributes, findings);
    match attributes.and_then(|attributes| attributes.event_trigger) {
        None | Some(event::CLICK) => {}
        Some(trigger @ (event::WITH | event::AFTER)) => {
            let said = if trigger == event::WITH {
                Start::With
            } else {
                Start::After
            };
            if said != start {
                findings.approximate(format!(
                    "the event trigger says {}; the chunk's own trigger, {}, plays",
                    said.name(),
                    start.name()
                ));
            }
        }
        Some(trigger) => findings.approximate(format!(
            "the event trigger {trigger} is unknown; the chunk's own trigger plays"
        )),
    }
    (start, delay)
}

/// How a chunk starts by its own fields: a click unless automatic; automatic, with or after by its build's
/// automatic delay when one is stated, else by its `referent`.
fn chunk_start(
    chunk: &kn::BuildChunkArchive,
    attributes: Option<&kn::BuildAttributesArchive>,
    findings: &mut Findings,
) -> (Start, Option<f64>) {
    if chunk.automatic != Some(true) {
        return (Start::Click, None);
    }
    let referent = if chunk.referent == Some(false) {
        Start::With
    } else {
        Start::After
    };
    let after = automatic_delay(attributes.and_then(|a| a.delay_automatic_after), findings);
    let with = automatic_delay(attributes.and_then(|a| a.delay_automatic_with), findings);
    let (start, delay) = match (after, with) {
        (Some(delay), None) => (Start::After, delay),
        (None, Some(delay)) => (Start::With, delay),
        (None, None) => return (referent, None),
        (Some(_), Some(_)) => {
            findings.approximate(
                "the build states both a with and an after delay; the chunk's referent plays",
            );
            return (referent, None);
        }
    };
    if chunk.referent.is_some() && start != referent {
        findings.approximate(format!(
            "the chunk's referent says {} and its build's automatic delay says {}; {} plays",
            referent.name(),
            start.name(),
            start.name()
        ));
    }
    (start, Some(delay))
}

/// How long a step plays, repeats included, in whole milliseconds within the bound.
fn played(duration: Ms, repeat: Option<Repeat>) -> u64 {
    let times = match repeat {
        Some(Repeat::Count(count)) => count.to_f64().max(1.0),
        _ => 1.0,
    };
    let total = f64::from(duration.0) * times;
    if total.is_finite() {
        // Bounded by the clamp below.
        total.min(f64::from(Ms::MAX)) as u64
    } else {
        u64::from(Ms::MAX)
    }
}

/// Where each step starts inside its click group.
#[derive(Debug, Default)]
struct Offsets {
    previous: Option<(u64, u64)>,
}

impl Offsets {
    fn next(&mut self, start: Start, delay: Ms, length: u64) -> Ms {
        let delay = u64::from(delay.0);
        let at = match (start, self.previous) {
            (Start::Click, _) | (_, None) => delay,
            (Start::With, Some((begin, _))) => begin + delay,
            (Start::After, Some((_, end))) => end + delay,
        };
        let at = at.min(u64::from(Ms::MAX));
        self.previous = Some((at, at + length));
        // Within an hour by the clamp.
        Ms(at as u32)
    }
}

/// Builds in the order the slide plays them. A chunk that waits for a click is a click step; an automatic one plays
/// after the step before it (the first ones as the slide appears). A build in several chunks plays one paragraph per
/// chunk. A chunk whose build or drawable is missing from the slide is reported and skipped (KN1401). The first
/// [`limits::STEPS_PER_SLIDE`] steps carry native data; the rest play their fallback (KN1404, once).
pub fn timeline(
    ctx: &mut Lowering<'_>,
    slide: &kn::SlideArchive,
    elements: &[Element],
) -> Vec<TimelineStep> {
    let graph = ctx.graph;
    let mut targets = BTreeMap::new();
    ids_of(elements, &mut targets);
    let chunks: Vec<(&tsp::Reference, Option<&kn::BuildChunkArchive>)> = slide
        .build_chunks
        .iter()
        .map(|reference| {
            (
                reference,
                graph.get::<kn::BuildChunkArchive>(Some(reference)),
            )
        })
        .collect();
    let mut chunks_per_build: BTreeMap<u64, usize> = BTreeMap::new();
    for (_, chunk) in &chunks {
        if let Some(id) = chunk
            .and_then(|chunk| chunk.build.as_ref())
            .and_then(|b| b.identifier)
        {
            *chunks_per_build.entry(id).or_default() += 1;
        }
    }
    let mut seen_per_build: BTreeMap<u64, u32> = BTreeMap::new();
    let mut steps = Vec::new();
    let mut offsets = Offsets::default();
    for (reference, chunk) in chunks {
        let build_id = chunk
            .and_then(|chunk| chunk.build.as_ref())
            .and_then(|b| b.identifier);
        let build = graph.get::<kn::BuildArchive>(chunk.and_then(|chunk| chunk.build.as_ref()));
        let target = build
            .and_then(|build| build.drawable.as_ref())
            .and_then(|drawable| drawable.identifier)
            .and_then(|id| targets.get(&id.to_string()));
        let (Some(chunk), Some(build_id), Some(build), Some(target)) =
            (chunk, build_id, build, target)
        else {
            let named = |id: Option<u64>| id.map_or_else(|| "none".to_owned(), |id| id.to_string());
            ctx.report(
                codes::KN1401,
                DiagnosticSite::new()
                    .data("chunk", named(reference.identifier))
                    .data("build", named(build_id)),
            );
            continue;
        };
        let index = seen_per_build.entry(build_id).or_default();
        let paragraph = *index;
        *index += 1;
        let attributes = build
            .attributes
            .as_ref()
            .and_then(|attributes| attributes.animation_attributes.as_ref());
        let effect = attributes
            .and_then(|attributes| attributes.effect.as_deref())
            .unwrap_or_default();
        let fallback = step_fallback(effect);
        let named = if effect.is_empty() { "none" } else { effect };
        if fallback.is_none() {
            ctx.report(codes::KN1402, DiagnosticSite::new().data("effect", named));
        }
        let several = chunks_per_build.get(&build_id).copied().unwrap_or(0) > 1;
        let delay_ms = ms(chunk.delay);
        let duration_ms = ms(chunk
            .duration
            .or_else(|| attributes.and_then(|attributes| attributes.duration))
            .or(build.duration));
        let kind = step_kind(attributes.and_then(|a| a.animation_type.as_deref()), effect);
        let mut findings = Findings::default();
        let delivery = if ctx.native_motion {
            native_delivery(build, effect, &mut findings)
        } else {
            delivery(build, effect)
        };
        let (native, timing) = if steps.len() < limits::STEPS_PER_SLIDE {
            let (start, automatic) = start_of(chunk, build.attributes.as_ref(), &mut findings);
            if let Some(index) = chunk.index.filter(|index| *index != paragraph) {
                findings.approximate(format!(
                    "the chunk's index {index} is not its place {paragraph} in the build; the build order plays"
                ));
            }
            let native = native_build(
                ctx,
                &mut findings,
                effect,
                kind,
                build,
                build_id,
                duration_ms,
                delivery,
            );
            let delay = match (chunk.delay, automatic) {
                (None, Some(seconds)) => ms(Some(seconds)),
                _ => delay_ms,
            };
            let offset = offsets.next(start, delay, played(duration_ms, native.repeat));
            findings.report(ctx, named);
            let timing = StepTiming {
                offset_ms: Some(offset),
                easing: native.easing,
                repeat: native.repeat,
                ..StepTiming::default()
            };
            (native.native, Some(Box::new(timing)))
        } else {
            if steps.len() == limits::STEPS_PER_SLIDE {
                ctx.report_motion(
                    codes::KN1404,
                    DiagnosticSite::new().data("effect", named).data(
                        "reason",
                        format!(
                            "the slide's builds past {} play their fallback",
                            limits::STEPS_PER_SLIDE
                        ),
                    ),
                );
            }
            (None, None)
        };
        steps.push(TimelineStep {
            target: StepTarget {
                element: target.clone(),
                paragraphs: several.then_some((paragraph, paragraph)),
                background: false,
            },
            trigger: if chunk.automatic == Some(true) {
                Trigger::AfterPrevious
            } else {
                Trigger::Click
            },
            delay_ms,
            duration_ms,
            kind,
            effect: StepEffect {
                id: effect_id(named),
                fallback: fallback.unwrap_or(StepFallback::Dissolve),
                native,
            },
            delivery,
            timing,
        });
    }
    if !steps.is_empty() {
        ctx.see(FeatureId::SlideTimeline);
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_start_by_click_with_and_after() {
        let mut offsets = Offsets::default();
        assert_eq!(offsets.next(Start::After, Ms(100), 500), Ms(100));
        assert_eq!(offsets.next(Start::After, Ms(300), 1000), Ms(900));
        assert_eq!(offsets.next(Start::With, Ms(0), 700), Ms(900));
        assert_eq!(offsets.next(Start::With, Ms(50), 10), Ms(950));
        assert_eq!(offsets.next(Start::After, Ms(0), 10), Ms(960));
        assert_eq!(offsets.next(Start::Click, Ms(200), 10), Ms(200));
        assert_eq!(offsets.next(Start::After, Ms(Ms::MAX), 10), Ms(Ms::MAX));
    }

    #[test]
    fn referent_tells_with_from_after_when_the_build_states_no_automatic_delay() {
        let chunk = |automatic, referent| kn::BuildChunkArchive {
            automatic,
            referent,
            ..kn::BuildChunkArchive::default()
        };
        let start = |chunk: &kn::BuildChunkArchive,
                     attributes: Option<&kn::BuildAttributesArchive>| {
            start_of(chunk, attributes, &mut Findings::default())
        };
        assert_eq!(
            start(&chunk(Some(true), Some(false)), None),
            (Start::With, None)
        );
        assert_eq!(
            start(&chunk(Some(true), Some(true)), None),
            (Start::After, None)
        );
        assert_eq!(start(&chunk(Some(true), None), None), (Start::After, None));
        assert_eq!(
            start(&chunk(Some(false), Some(false)), None),
            (Start::Click, None)
        );
        assert_eq!(start(&chunk(None, None), None), (Start::Click, None));
        let with = kn::BuildAttributesArchive {
            delay_automatic_with: Some(0.5),
            ..kn::BuildAttributesArchive::default()
        };
        assert_eq!(
            start(&chunk(Some(true), None), Some(&with)),
            (Start::With, Some(0.5))
        );
        assert_eq!(start(&chunk(None, None), Some(&with)), (Start::Click, None));
    }
}
