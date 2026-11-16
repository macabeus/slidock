//! A page's build animations: the timing root's main sequence flattened into timeline steps. Each
//! child `anim:par` of the main sequence is a group, which a click starts (`smil:begin="next"`) or which plays on its
//! own when the page opens; inside it, every `anim:par` or `anim:iterate` carrying a preset class or a node type is
//! one effect, starting at its time slot's begin plus its own (`timing.offsetMs`). An effect lasts until its
//! behaviours' latest end; a play command lasts its medium's own length (the movie's `mvhd`), whatever its own
//! `smil:dur`, and a stop or pause command is an instant, lasting its own duration.
//!
//! The tree is untrusted. The walk visits at most [`TIMING_NODES`] nodes; an effect nested deeper than
//! [`TIMING_DEPTH`], one whose start, or its group's or a container's, names another node (`id3.end`: a
//! self-reference or a cycle the timeline could not hold), and every effect of an interactive sequence are dropped
//! with a diagnostic, as are behaviours nested deeper than [`TIMING_DEPTH`] below their effect. A value past its
//! limit is clamped and an invalid one (NaN, negative, an animation with no duration) reads as the declared default
//! ([`DEFAULT_DURATION`], no delay), each reported (OD9411), as is a rewind (`smil:fill="remove"`) the step cannot
//! hold; an unknown option value reads as its default (OD9412); an effect with no vocabulary entry whose behaviours
//! do not compile plays its fallback (OD9410); an entry the presets table marks approximate plays the effect's own
//! behaviours (tier 2), or the approximation when they compile to nothing, noted (OD9413); an effect of an interactive
//! sequence is left out (OD9404).

mod expr;
mod preset;
mod smil;
mod targets;
pub(crate) mod value;

use std::collections::BTreeMap;

use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::effects::limits::{STEPS_PER_SLIDE, TIMING_DEPTH, TIMING_NODES};
use slidock_ir::effects::{
    AfterEffect, BuildEffect, Delivery, Easing, HexColor, NativeId, NativeStep, Paint, Repeat,
    Stagger, StaggerAmount, StaggerOrder, StepFallback, StepKind, StepTarget, StepTiming, Trigger,
};
use slidock_ir::features::FeatureId;
use slidock_ir::formats::FileFormat;
use slidock_ir::presentation::SlideSize;
use slidock_ir::source::Element as IrElement;
use slidock_ir::timeline::{StepEffect, TimelineStep};
use slidock_ir::units::{Ms, Ratio, UnitRatio, pt_to_px};
use slidock_presets::Unknown;
use slidock_presets::ooo::PresetClass;

use crate::context::Lowering;
use crate::names::qn;
use crate::odf::{Element, Ns};
use crate::transition::{report_timing, report_unknown, timing_reason, timing_root};

use self::preset::Mapped;
use self::targets::{Target, Targets};
use self::value::{Begin, Read, RepeatCount};

/// The duration an effect whose own is invalid plays for.
pub(crate) const DEFAULT_DURATION: Ms = Ms(500);

/// What a leaf of an effect does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LeafKind {
    Set,
    Animate,
    AnimateColor,
    AnimateTransform,
    AnimateMotion,
    TransitionFilter,
    Audio,
    Command,
}

impl LeafKind {
    fn of(element: &Element) -> Option<LeafKind> {
        if !element.in_namespace(Ns::Anim) {
            return None;
        }
        Some(match element.local.as_str() {
            "set" => LeafKind::Set,
            "animate" => LeafKind::Animate,
            "animateColor" => LeafKind::AnimateColor,
            "animateTransform" => LeafKind::AnimateTransform,
            "animateMotion" => LeafKind::AnimateMotion,
            "transitionFilter" => LeafKind::TransitionFilter,
            "audio" => LeafKind::Audio,
            "command" => LeafKind::Command,
            _ => return None,
        })
    }
}

/// One behaviour of an effect, with its start inside the effect.
#[derive(Clone, Debug)]
pub(crate) struct Leaf<'a> {
    pub element: &'a Element,
    pub kind: LeafKind,
    pub begin: Read<Ms>,
    /// `None` when the leaf writes no duration.
    pub dur: Option<Read<Ms>>,
    pub target: Option<&'a str>,
    pub sub_item: Option<&'a str>,
}

impl Leaf<'_> {
    pub(crate) fn attribute(&self) -> Option<&str> {
        self.element.attr(qn!("smil:attributeName"))
    }

    /// A visibility switch: the `set` an entrance or exit opens or closes with.
    fn is_visibility(&self) -> bool {
        self.kind == LeafKind::Set && self.attribute() == Some("visibility")
    }
}

/// The geometry a value expression reads, and the slide's size in pixels.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Geometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub slide_w: f64,
    pub slide_h: f64,
}

impl Geometry {
    fn of(target: &Target<'_>, size: &SlideSize) -> Geometry {
        let (w, h) = (size.w.to_f64().max(1.0), size.h.to_f64().max(1.0));
        let frame = target.shape.frame.as_ref();
        Geometry {
            x: frame.map_or(0.0, |f| f.x.to_f64() / w),
            y: frame.map_or(0.0, |f| f.y.to_f64() / h),
            width: frame.map_or(0.0, |f| f.w.to_f64() / w),
            height: frame.map_or(0.0, |f| f.h.to_f64() / h),
            slide_w: pt_to_px(w),
            slide_h: pt_to_px(h),
        }
    }
}

/// Why an effect is left out of the timeline.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Dropped {
    /// Its start names a node (its own, another effect's, a shape's click).
    Condition(String),
    /// It is nested deeper than the timing depth limit.
    Deep,
    /// It belongs to an interactive sequence (a shape's click plays it).
    Interactive,
}

impl Dropped {
    fn reason(&self) -> String {
        match self {
            Dropped::Condition(begin) => {
                format!("the effect starts on {begin}, which the timeline cannot hold; it is left out")
            }
            Dropped::Deep => format!(
                "the effect is nested deeper than {TIMING_DEPTH} levels; it is left out"
            ),
            Dropped::Interactive => {
                "the effect plays when a shape is clicked, which the timeline cannot hold; it is left out"
                    .to_owned()
            }
        }
    }
}

/// An effect node found in a sequence.
struct Found<'a> {
    node: &'a Element,
    /// The group (a child of the main sequence) it is in, and whether a click starts that group.
    group: usize,
    click: bool,
    /// The time slot (the group's child holding it) it is in.
    slot: usize,
    /// The begins of its containers below the group, added.
    offset: Read<Ms>,
    dropped: Option<Dropped>,
}

fn add(a: Read<Ms>, b: Read<Ms>) -> Read<Ms> {
    match (a.value(), b.value()) {
        (Some(x), Some(y)) => {
            let sum = u64::from(x.0) + u64::from(y.0);
            if sum > u64::from(Ms::MAX) || a.approximated() || b.approximated() {
                Read::Clamped(Ms(
                    u32::try_from(sum.min(u64::from(Ms::MAX))).unwrap_or(Ms::MAX)
                ))
            } else {
                Read::Value(Ms(u32::try_from(sum).unwrap_or(Ms::MAX)))
            }
        }
        _ => Read::Invalid,
    }
}

fn is_effect(element: &Element) -> bool {
    (element.is(Ns::Anim, "par") || element.is(Ns::Anim, "iterate"))
        && (element.attr(qn!("presentation:preset-class")).is_some()
            || matches!(
                element.attr(qn!("presentation:node-type")),
                Some("on-click" | "with-previous" | "after-previous")
            ))
}

fn is_after_effect(element: &Element) -> bool {
    element.is(Ns::Anim, "par")
        && (element.attr(qn!("presentation:master-element")).is_some()
            || element.attr(qn!("presentation:after-effect")) == Some("true"))
}

fn is_container(element: &Element) -> bool {
    element.is(Ns::Anim, "par") || element.is(Ns::Anim, "seq") || element.is(Ns::Anim, "iterate")
}

/// The walk's node budget.
struct Budget {
    left: usize,
    spent: bool,
}

impl Budget {
    fn take(&mut self) -> bool {
        if self.left == 0 {
            self.spent = true;
            return false;
        }
        self.left -= 1;
        true
    }
}

/// A begin as an offset, or the condition that makes it one the timeline cannot hold.
fn offset_of(element: &Element) -> Result<Read<Ms>, String> {
    let written = element.attr(qn!("smil:begin"));
    match value::begin(written) {
        Begin::Offset(read) => Ok(read),
        Begin::Next | Begin::Indefinite => Ok(Read::Value(Ms(0))),
        Begin::Event { .. } => Err(written.unwrap_or_default().to_owned()),
    }
}

/// A node the walk will visit: its depth from the timing root, the begins of its containers below the group, the
/// time slot it is in, and the start condition on another node met on the way, if any.
struct Pending<'a> {
    element: &'a Element,
    depth: usize,
    offset: Read<Ms>,
    slot: Option<usize>,
    condition: Option<String>,
}

/// A node of an effect: its depth below the effect, its start inside it, and the target and sub-item it inherits.
struct Inherited<'a> {
    element: &'a Element,
    depth: usize,
    offset: Read<Ms>,
    target: Option<&'a str>,
    sub: Option<&'a str>,
}

/// The effects and after effects of a sequence, in document order. `interactive` drops every effect.
struct Walk<'a> {
    found: Vec<Found<'a>>,
    afters: Vec<&'a Element>,
    groups: usize,
    slots: usize,
}

fn walk<'a>(sequence: &'a Element, interactive: bool, budget: &mut Budget, walk: &mut Walk<'a>) {
    for group in sequence.kids() {
        if !budget.take() {
            return;
        }
        if !group.in_namespace(Ns::Anim) {
            continue;
        }
        let index = walk.groups;
        walk.groups += 1;
        // A click starts the group, or it plays on its own; a group whose start names a node drops its effects.
        let click = matches!(
            value::begin(group.attr(qn!("smil:begin"))),
            Begin::Next | Begin::Indefinite
        );
        let condition = offset_of(group).err();
        let pending = |element, depth| Pending {
            element,
            depth,
            offset: Read::Value(Ms(0)),
            slot: None,
            condition: condition.clone(),
        };
        let mut stack: Vec<Pending<'a>> = if is_effect(group) {
            vec![pending(group, 2)]
        } else {
            group
                .kids()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(|child| pending(child, 3))
                .collect()
        };
        while let Some(Pending {
            element,
            depth,
            offset,
            slot,
            condition,
        }) = stack.pop()
        {
            if !budget.take() {
                return;
            }
            let slot = slot.unwrap_or_else(|| {
                walk.slots += 1;
                walk.slots
            });
            if is_effect(element) {
                let dropped = if interactive {
                    Some(Dropped::Interactive)
                } else if depth > TIMING_DEPTH {
                    Some(Dropped::Deep)
                } else {
                    condition
                        .or(offset_of(element).err())
                        .map(Dropped::Condition)
                };
                walk.found.push(Found {
                    node: element,
                    group: index,
                    click,
                    slot,
                    offset,
                    dropped,
                });
                continue;
            }
            if is_after_effect(element) {
                walk.afters.push(element);
                continue;
            }
            if !is_container(element) {
                continue;
            }
            let (own, condition) = match offset_of(element) {
                Ok(own) => (own, condition),
                Err(begin) => (Read::Value(Ms(0)), condition.or(Some(begin))),
            };
            let offset = add(offset, own);
            stack.extend(
                element
                    .kids()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .map(|child| Pending {
                        element: child,
                        depth: depth + 1,
                        offset,
                        slot: Some(slot),
                        condition: condition.clone(),
                    }),
            );
        }
    }
}

/// The behaviours of an effect, with their starts and the target and sub-item they inherit; nested at most
/// [`TIMING_DEPTH`] levels below the effect. The flag says whether a node deeper than that was left out.
fn leaves<'a>(effect: &'a Element, budget: &mut Budget) -> (Vec<Leaf<'a>>, bool) {
    let mut out = Vec::new();
    let mut deep = false;
    let inherit = |element: &'a Element, target: Option<&'a str>, sub: Option<&'a str>| {
        (
            element.attr(qn!("smil:targetElement")).or(target),
            element.attr(qn!("anim:sub-item")).or(sub),
        )
    };
    let (target, sub) = inherit(effect, None, None);
    let mut stack: Vec<Inherited<'a>> = effect
        .kids()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|element| Inherited {
            element,
            depth: 1,
            offset: Read::Value(Ms(0)),
            target,
            sub,
        })
        .collect();
    while let Some(Inherited {
        element,
        depth,
        offset,
        target,
        sub,
    }) = stack.pop()
    {
        if depth > TIMING_DEPTH {
            deep = true;
            continue;
        }
        if !budget.take() {
            continue;
        }
        let (target, sub) = inherit(element, target, sub);
        let own = offset_of(element).unwrap_or(Read::Value(Ms(0)));
        if let Some(kind) = LeafKind::of(element) {
            out.push(Leaf {
                element,
                kind,
                begin: add(offset, own),
                dur: element.attr(qn!("smil:dur")).map(value::clock),
                target,
                sub_item: sub,
            });
        } else if is_container(element) {
            let offset = add(offset, own);
            stack.extend(
                element
                    .kids()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .map(|child| Inherited {
                        element: child,
                        depth: depth + 1,
                        offset,
                        target,
                        sub,
                    }),
            );
        }
    }
    (out, deep)
}

/// Whether an effect builds by word or by character (`anim:iterate`).
fn by_unit(node: &Element) -> bool {
    std::iter::once(node)
        .chain(node.kids())
        .filter(|element| element.is(Ns::Anim, "iterate"))
        .any(|iterate| {
            matches!(
                iterate.attr(qn!("anim:iterate-type")),
                Some("by-word" | "by-letter")
            )
        })
}

/// The step kind of a custom effect: what its visibility switches do.
fn custom_kind(leaves: &[Leaf<'_>]) -> StepKind {
    let to = |value: &str| {
        leaves
            .iter()
            .any(|leaf| leaf.is_visibility() && leaf.element.attr(qn!("smil:to")) == Some(value))
    };
    if to("hidden") && !to("visible") {
        StepKind::Exit
    } else if to("visible") {
        StepKind::Entrance
    } else {
        StepKind::Emphasis
    }
}

/// Why an effect's duration is not as written: the value of the behaviour that made it so, or `None` for a
/// behaviour that writes none.
type DurationSource<'a> = Option<Option<&'a str>>;

/// Whether a leaf animates over time: an animation or a filter, which a missing duration leaves indefinite (SMIL's
/// default), unlike a switch (`set`), a sound or a command.
fn animates_over_time(leaf: &Leaf<'_>) -> bool {
    matches!(
        leaf.kind,
        LeafKind::Animate
            | LeafKind::AnimateColor
            | LeafKind::AnimateTransform
            | LeafKind::AnimateMotion
            | LeafKind::TransitionFilter
    )
}

/// Whether a leaf is a media command that plays its medium (`anim:command="play"`).
fn plays_medium(leaf: &Leaf<'_>) -> bool {
    leaf.kind == LeafKind::Command
        && leaf.element.attr(qn!("anim:command")).map(str::trim) == Some("play")
}

/// An effect's duration: the latest end of its behaviours, visibility switches and sets held indefinitely aside (an
/// effect that is only a switch lasts 0 ms). `Invalid` when a behaviour's own duration is, or when an animation
/// writes none (indefinite), with the source of that.
fn duration<'a>(leaves: &[Leaf<'a>]) -> (Read<Ms>, DurationSource<'a>) {
    let mut end = Read::Value(Ms(0));
    for leaf in leaves.iter().filter(|leaf| !leaf.is_visibility()) {
        // A set that holds indefinitely is a switch: it ends where it starts.
        let held = leaf.kind == LeafKind::Set
            && leaf.element.attr(qn!("smil:dur")).map(str::trim) == Some("indefinite");
        let Some(dur) = leaf.dur.filter(|_| !held) else {
            if animates_over_time(leaf) {
                return (Read::Invalid, Some(None));
            }
            end = match (end, leaf.begin.value()) {
                (Read::Value(e), Some(b)) if b > e => leaf.begin,
                _ => end,
            };
            continue;
        };
        let written = leaf.element.attr(qn!("smil:dur"));
        if dur.value().is_none() {
            return (Read::Invalid, Some(written));
        }
        // A behaviour's own reversal and repeat run it over more than one span.
        let retime = smil::own_time(leaf).retime;
        let dur = match dur {
            Read::Value(ms) => retime
                .length(ms)
                .map_or(Read::Clamped(Ms(Ms::MAX)), Read::Value),
            Read::Clamped(ms) => Read::Clamped(retime.length(ms).unwrap_or(Ms(Ms::MAX))),
            Read::Invalid => Read::Invalid,
        };
        let leaf_end = add(leaf.begin, dur);
        if leaf_end.approximated() {
            return (leaf_end, Some(written));
        }
        if leaf_end.value() > end.value() {
            end = leaf_end;
        }
    }
    (end, None)
}

/// Where an effect's switch to visible ends when it does not hold (`smil:fill` remove, or none with a duration, as
/// LibreOffice's Flash Once writes it): the target shows only until then. `None` when every such switch holds.
fn shown_only_while(leaves: &[Leaf<'_>]) -> Option<Ms> {
    leaves
        .iter()
        .filter(|leaf| {
            leaf.is_visibility()
                && leaf.element.attr(qn!("smil:to")).map(str::trim) == Some("visible")
        })
        .filter(|leaf| {
            !matches!(
                leaf.element.attr(qn!("smil:fill")),
                Some("hold" | "freeze" | "transition")
            )
        })
        .find_map(|leaf| {
            let begin = leaf.begin.value()?;
            let dur = leaf.dur?.value()?;
            Some(Ms(begin.0.checked_add(dur.0)?.min(Ms::MAX)))
        })
}

/// What an effect does when it ends (`smil:fill`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fill {
    /// `hold`, `freeze`, `transition`: it keeps its end state.
    Keep,
    /// `remove`: it rewinds to its start (LibreOffice's "rewind when done").
    Rewind,
    /// No fill, `auto` or `default`: the preset's own.
    Preset,
}

impl Fill {
    fn parse(token: Option<&str>) -> Option<Fill> {
        Some(match token {
            None | Some("auto" | "default") => Fill::Preset,
            Some("hold" | "freeze" | "transition") => Fill::Keep,
            Some("remove") => Fill::Rewind,
            Some(_) => return None,
        })
    }
}

/// Sets an effect's `hold`; `false` for an effect with no such parameter.
fn set_hold(effect: &mut BuildEffect, hold: bool) -> bool {
    match effect {
        BuildEffect::GrowShrink(p) => p.hold = hold,
        BuildEffect::Spin(p) => p.hold = hold,
        BuildEffect::Transparency(p) => p.hold = hold,
        BuildEffect::Path(p) => p.hold = hold,
        _ => return false,
    }
    true
}

/// Whether an emphasis returns to its start on its own.
fn returns(effect: &BuildEffect) -> bool {
    matches!(
        effect,
        BuildEffect::Pulse(_)
            | BuildEffect::Teeter(_)
            | BuildEffect::Jiggle(_)
            | BuildEffect::Wave(_)
            | BuildEffect::Blink(_)
            | BuildEffect::Flash(_)
            | BuildEffect::BoldFlash(_)
    )
}

/// The colour of an after effect's dimming, or that it hides the target.
fn after_effect(node: &Element) -> Option<AfterEffect> {
    node.kids().find_map(|set| {
        if !set.is(Ns::Anim, "set") {
            return None;
        }
        match (
            set.attr(qn!("smil:attributeName")),
            set.attr(qn!("smil:to")),
        ) {
            (Some("visibility"), Some("hidden")) => Some(AfterEffect::Hide),
            (Some("dim" | "color" | "fill-color"), Some(color)) => {
                HexColor::parse(color).ok().map(AfterEffect::Dim)
            }
            _ => None,
        }
    })
}

/// The native id of an effect: `odp:<preset-id>[/<sub-type>]`, else its class.
fn native_of(node: &Element) -> NativeId {
    let name = node
        .attr(qn!("presentation:preset-id"))
        .or_else(|| node.attr(qn!("presentation:preset-class")))
        .unwrap_or("custom");
    let name = match node.attr(qn!("presentation:preset-sub-type")) {
        Some(sub) if !sub.is_empty() => format!("{name}/{sub}"),
        _ => name.to_owned(),
    };
    NativeId::new(FileFormat::Odp.into(), &name)
}

/// Reads one page's timeline.
struct Reader<'p, 'a, 'd> {
    ctx: &'p Lowering<'a, 'd>,
    targets: Targets<'a>,
    size: &'p SlideSize,
    afters: BTreeMap<&'a str, AfterEffect>,
}

impl<'a> Reader<'_, 'a, '_> {
    fn site(&self, native: &NativeId, node: Option<&Target<'_>>) -> DiagnosticSite {
        DiagnosticSite::new()
            .node(node.map(|target| target.shape.id.clone()))
            .data("effect", native.to_string())
    }

    fn unknown(&self, native: &NativeId, target: Option<&Target<'_>>, unknown: &Unknown) {
        report_unknown(self.ctx, self.site(native, target), unknown);
    }

    fn drop(&self, native: &NativeId, target: Option<&Target<'_>>, what: &str, reason: String) {
        report_timing(self.ctx, self.site(native, target), what, reason);
    }

    /// Reports a timing value that is not as written.
    fn approximated(
        &self,
        native: &NativeId,
        target: &Target<'_>,
        what: &str,
        read: Read<impl Copy>,
        written: Option<&str>,
        used: &str,
    ) {
        if read.approximated() {
            report_timing(
                self.ctx,
                self.site(native, Some(target)),
                what,
                timing_reason(read, written.unwrap_or_default(), used),
            );
        }
    }

    /// The length of the medium a media command plays, from the movie's own metadata; past [`Ms::MAX`] it clamps,
    /// and a medium whose length does not read takes [`DEFAULT_DURATION`], each reported.
    fn medium_length(&self, native: &NativeId, target: &Target<'_>) -> Read<Ms> {
        let length = target
            .shape
            .media
            .and_then(|sha256| self.ctx.media.borrow().length(&sha256));
        let read = match length {
            Some(ms) if ms.is_finite() && ms >= 0.0 && ms <= f64::from(Ms::MAX) => {
                Read::Value(Ms(ms as u32))
            }
            Some(ms) if ms.is_finite() && ms > 0.0 => Read::Clamped(Ms(Ms::MAX)),
            _ => Read::Invalid,
        };
        match (read, length) {
            (Read::Clamped(_), Some(ms)) => self.drop(
                native,
                Some(target),
                "duration",
                format!(
                    "the medium's length, {ms} ms, is past the limit; it reads as {} ms",
                    Ms::MAX
                ),
            ),
            (Read::Invalid, _) => self.drop(
                native,
                Some(target),
                "duration",
                format!(
                    "the medium's length does not read; the command reads as {} ms",
                    DEFAULT_DURATION.0
                ),
            ),
            _ => {}
        }
        read
    }

    fn lower(
        &self,
        found: &Found<'a>,
        budget: &mut Budget,
        first_of_group: bool,
        previous_slot: Option<usize>,
    ) -> Option<TimelineStep> {
        let node = found.node;
        let native = native_of(node);
        let (leaves, deep) = leaves(node, budget);
        let target_name = leaves
            .iter()
            .find_map(|leaf| leaf.target)
            .or_else(|| node.attr(qn!("smil:targetElement")));
        let target = target_name.and_then(|name| self.targets.resolve(name));
        if let Some(dropped) = &found.dropped {
            if *dropped == Dropped::Interactive {
                self.ctx.report(
                    codes::OD9404,
                    self.site(&native, target.as_ref())
                        .feature(FeatureId::Animation),
                );
            } else {
                self.drop(&native, target.as_ref(), "start", dropped.reason());
            }
            return None;
        }
        let Some(target) = target else {
            self.drop(
                &native,
                None,
                "target",
                format!(
                    "the effect targets {}, which is no shape of the slide; it is left out",
                    target_name.unwrap_or("nothing")
                ),
            );
            return None;
        };

        if deep {
            self.drop(
                &native,
                Some(&target),
                "behaviours",
                format!(
                    "the effect's behaviours nested deeper than {TIMING_DEPTH} levels below it are left out"
                ),
            );
        }

        let class_token = node.attr(qn!("presentation:preset-class"));
        let class = match class_token {
            None => PresetClass::Custom,
            Some(token) => PresetClass::parse(token).unwrap_or_else(|| {
                self.unknown(
                    &native,
                    Some(&target),
                    &Unknown {
                        option: "presentation:preset-class",
                        value: token.to_owned(),
                        used: "custom",
                    },
                );
                PresetClass::Custom
            }),
        };
        let kind = match class.kind() {
            Some(kind) => kind,
            None if class == PresetClass::OleAction => {
                self.ctx.report(
                    codes::OD9410,
                    self.site(&native, Some(&target)).data(
                        "play",
                        "nothing: an OLE action has no step, so it is left out",
                    ),
                );
                return None;
            }
            None => custom_kind(&leaves),
        };

        let trigger = match node.attr(qn!("presentation:node-type")) {
            _ if found.click && first_of_group => Trigger::Click,
            Some("on-click") => Trigger::Click,
            Some("with-previous") => Trigger::WithPrevious,
            Some("after-previous") => Trigger::AfterPrevious,
            other => {
                let derived = if first_of_group {
                    Trigger::AfterPrevious
                } else if previous_slot == Some(found.slot) {
                    Trigger::WithPrevious
                } else {
                    Trigger::AfterPrevious
                };
                if let Some(token) = other {
                    self.unknown(
                        &native,
                        Some(&target),
                        &Unknown {
                            option: "presentation:node-type",
                            value: token.to_owned(),
                            used: match derived {
                                Trigger::WithPrevious => "with-previous",
                                _ => "after-previous",
                            },
                        },
                    );
                }
                derived
            }
        };

        let begin_written = node.attr(qn!("smil:begin"));
        let delay = offset_of(node).unwrap_or(Read::Invalid);
        self.approximated(&native, &target, "delay", delay, begin_written, "no delay");
        let offset = match delay {
            Read::Invalid => None,
            delay => add(found.offset, delay).value(),
        };

        let geometry = Geometry::of(&target, self.size);
        let mut unknown = Vec::new();
        let Mapped {
            effect,
            delivery: implied,
            lost,
            approximate,
        } = preset::map(
            class,
            node.attr(qn!("presentation:preset-id")),
            node.attr(qn!("presentation:preset-sub-type")),
            &leaves,
            &geometry,
            &mut unknown,
        );
        for unknown in &unknown {
            self.unknown(&native, Some(&target), unknown);
        }
        // A font family or size change moves the layout, which no effect plays.
        let layout = leaves
            .iter()
            .any(|leaf| leaf.attribute().is_some_and(smil::changes_layout));
        let (effect, lost) = if layout { (None, None) } else { (effect, lost) };
        // A word or a character has no fill or line of its own: a step built by unit plays its text's tracks.
        let effect = match effect {
            Some(BuildEffect::Behaviors(mut tracks)) if by_unit(node) => {
                tracks.retain(|track| !matches!(track.paint(), Some(Paint::Fill | Paint::Line)));
                (!tracks.is_empty()).then_some(BuildEffect::Behaviors(tracks))
            }
            effect => effect,
        };
        // A vocabulary entry plays no sound; tier 2 names what it leaves out itself.
        let sounds = leaves.iter().any(|leaf| leaf.kind == LeafKind::Audio);
        let lost = match (&effect, lost) {
            (Some(BuildEffect::Behaviors(_)), lost) | (None, lost) => lost,
            (Some(_), None) if sounds => Some("its sound".to_owned()),
            (Some(_), lost) => lost,
        };
        let fallback = match &effect {
            Some(effect) => effect.fallback(kind),
            None => slidock_presets::kind_fallback(kind),
        };
        if effect.is_none() || lost.is_some() {
            let play = match (&effect, &lost) {
                (Some(BuildEffect::Behaviors(_)), Some(lost)) => {
                    format!("its behaviours without {lost}")
                }
                (Some(effect), Some(lost)) => format!("{} without {lost}", effect.name()),
                _ => format!(
                    "its fallback ({})",
                    match fallback {
                        StepFallback::Appear => "appear",
                        StepFallback::Dissolve => "dissolve",
                        StepFallback::None => "none",
                    }
                ),
            };
            self.ctx.report(
                codes::OD9410,
                self.site(&native, Some(&target)).data("play", play),
            );
        } else if let Some(effect) = effect.as_ref().filter(|_| approximate) {
            self.ctx.report(
                codes::OD9413,
                self.site(&native, Some(&target)).data(
                    "reason",
                    format!(
                        "{} only approximates the preset, and its behaviours compile to nothing",
                        effect.name()
                    ),
                ),
            );
        }

        let mut effect = effect;
        let fill_written = node.attr(qn!("smil:fill"));
        let fill = Fill::parse(fill_written).unwrap_or_else(|| {
            self.unknown(
                &native,
                Some(&target),
                &Unknown {
                    option: "smil:fill",
                    value: fill_written.unwrap_or_default().to_owned(),
                    used: "the preset's",
                },
            );
            Fill::Preset
        });
        if fill != Fill::Preset {
            let held = effect
                .as_mut()
                .is_some_and(|effect| set_hold(effect, fill == Fill::Keep));
            let honoured = match (&effect, kind) {
                (_, StepKind::Media) => true,
                (_, StepKind::Entrance | StepKind::Exit) => false,
                (Some(effect), _) => held || returns(effect),
                (None, _) => fallback == StepFallback::None,
            };
            if fill == Fill::Rewind && !honoured {
                self.drop(
                    &native,
                    Some(&target),
                    "fill",
                    "the effect rewinds when done (smil:fill remove), which the timeline cannot hold; it keeps its \
                     end state"
                        .to_owned(),
                );
            }
        }

        // A play command lasts as long as its medium plays, whatever its own `smil:dur` (LibreOffice writes the
        // command's instant, 0.001 s); stop and pause are instants, lasting their own duration.
        let plays_medium = kind == StepKind::Media && leaves.iter().any(plays_medium);
        let (dur_read, dur_source) = if plays_medium {
            (self.medium_length(&native, &target), None)
        } else {
            duration(&leaves)
        };
        let used = format!("{} ms", dur_read.value().unwrap_or(DEFAULT_DURATION).0);
        match dur_source {
            Some(None) => self.drop(
                &native,
                Some(&target),
                "duration",
                format!(
                    "a behaviour writes no duration, which leaves it indefinite; it reads as {used}"
                ),
            ),
            Some(written) => {
                self.approximated(&native, &target, "duration", dur_read, written, &used);
            }
            None => {}
        }
        // An entrance whose visibility switch does not hold (Flash Once) shows its target while the switch lasts,
        // then hides it.
        let shown = (kind == StepKind::Entrance)
            .then(|| shown_only_while(&leaves))
            .flatten();
        let duration = match shown {
            Some(end) => end,
            None if matches!(effect, Some(BuildEffect::Appear)) => Ms(0),
            None => dur_read.value().unwrap_or(DEFAULT_DURATION),
        };

        // Delivery: an iteration, else a paragraph target, else what the preset implies.
        let iterate = std::iter::once(node)
            .chain(node.kids())
            .find(|element| element.is(Ns::Anim, "iterate"));
        let mut stagger = None;
        let mut delivery = None;
        if let Some(iterate) = iterate {
            let iterate_type = iterate.attr(qn!("anim:iterate-type"));
            delivery = match iterate_type {
                Some("by-word") => Some(Delivery::ByWord),
                Some("by-letter") => Some(Delivery::ByChar),
                Some("by-paragraph") => Some(Delivery::ByParagraph),
                None => None,
                Some(other) => {
                    self.unknown(
                        &native,
                        Some(&target),
                        &Unknown {
                            option: "anim:iterate-type",
                            value: other.to_owned(),
                            used: "all at once",
                        },
                    );
                    None
                }
            };
            if delivery.is_some()
                && let Some(written) = iterate.attr(qn!("anim:iterate-interval"))
            {
                let read = value::clock(written);
                self.approximated(
                    &native,
                    &target,
                    "iteration interval",
                    read,
                    Some(written),
                    "no stagger",
                );
                stagger = read.value().map(|ms| Stagger {
                    per_unit: StaggerAmount::Ms(ms),
                    order: StaggerOrder::Forward,
                });
            }
        }
        let sub_item = leaves.iter().find_map(|leaf| leaf.sub_item);
        let text_only =
            sub_item == Some("text") && target.paragraph.is_none() && target.shape.paragraphs > 0;
        let paragraphs = match (target.paragraph, text_only) {
            (Some(index), _) => Some((index, index)),
            (None, true) => Some((0, target.shape.paragraphs - 1)),
            (None, false) => None,
        };
        let delivery = delivery
            .or(target.paragraph.map(|_| Delivery::ByParagraph))
            .or(implied)
            .unwrap_or(Delivery::All);

        let repeat =
            node.attr(qn!("smil:repeatCount")).and_then(|written| {
                match value::repeat_count(written) {
                    RepeatCount::Indefinite => Some(
                        if node.attr(qn!("smil:end")).map(slidock_canon::js_trim) == Some("next") {
                            Repeat::UntilNextClick
                        } else {
                            Repeat::UntilEndOfSlide
                        },
                    ),
                    RepeatCount::Times(read) => {
                        self.approximated(
                            &native,
                            &target,
                            "repeat",
                            read,
                            Some(written),
                            &match read.value() {
                                Some(count) => format!("{count} times"),
                                None => "once".to_owned(),
                            },
                        );
                        read.value()
                            .filter(|count| *count != 1.0)
                            .and_then(|count| Ratio::quantise(count).ok())
                            .map(Repeat::Count)
                    }
                }
            });
        let auto_reverse = match node.attr(qn!("smil:autoReverse")) {
            None => false,
            Some(written) => node.bool_attr(qn!("smil:autoReverse")).unwrap_or_else(|| {
                self.unknown(
                    &native,
                    Some(&target),
                    &Unknown {
                        option: "smil:autoReverse",
                        value: written.to_owned(),
                        used: "false",
                    },
                );
                false
            }),
        };
        let easing = self.easing(node, &native, &target);
        let after = node
            .attr(qn!("xml:id"))
            .and_then(|id| self.afters.get(id))
            .copied()
            .or(shown.map(|_| AfterEffect::Hide));
        let timing = StepTiming {
            offset_ms: offset,
            easing,
            stagger,
            repeat,
            auto_reverse,
            after,
        };
        Some(TimelineStep {
            target: StepTarget {
                element: target.shape.id.clone(),
                paragraphs,
                background: sub_item == Some("background"),
            },
            trigger,
            delay_ms: delay.value().unwrap_or(Ms(0)),
            duration_ms: duration,
            kind,
            effect: StepEffect {
                id: native.effect_id(),
                fallback,
                native: effect.map(|effect| {
                    Box::new(NativeStep {
                        id: native.clone(),
                        effect,
                    })
                }),
            },
            delivery,
            timing: (!timing.is_default()).then(|| Box::new(timing)),
        })
    }

    /// SMIL's accelerate and decelerate, which together cannot pass the whole duration.
    fn easing(&self, node: &Element, native: &NativeId, target: &Target<'_>) -> Option<Easing> {
        let read = |name| {
            node.attr(name)
                .map(|written: &str| value::fraction(written).ok_or_else(|| written.to_owned()))
        };
        let accel = read(qn!("smil:accelerate"));
        let decel = read(qn!("smil:decelerate"));
        if accel.is_none() && decel.is_none() {
            return None;
        }
        let bad = |option: &'static str, value: String| {
            self.unknown(
                native,
                Some(target),
                &Unknown {
                    option,
                    value,
                    used: "0",
                },
            );
            0.0
        };
        let accel = match accel {
            None => 0.0,
            Some(Ok(x)) => x,
            Some(Err(value)) => bad("smil:accelerate", value),
        };
        let decel = match decel {
            None => 0.0,
            Some(Ok(x)) => x,
            Some(Err(value)) => bad("smil:decelerate", value),
        };
        let (Ok(accel_ratio), Ok(decel_ratio)) =
            (UnitRatio::quantise(accel), UnitRatio::quantise(decel))
        else {
            return None;
        };
        if accel_ratio.steps() + decel_ratio.steps() > 100_000 {
            self.unknown(
                native,
                Some(target),
                &Unknown {
                    option: "smil:accelerate",
                    value: format!("{accel} with smil:decelerate {decel}"),
                    used: "linear",
                },
            );
            return None;
        }
        (accel_ratio.steps() + decel_ratio.steps() > 0).then_some(Easing::AccelDecel {
            accel: accel_ratio,
            decel: decel_ratio,
        })
    }
}

/// The page's timeline, from its timing root's main sequence; interactive sequences are dropped and reported.
pub(crate) fn read<'a>(
    page: &'a Element,
    path: &str,
    elements: &[IrElement],
    size: &SlideSize,
    ctx: &Lowering<'a, '_>,
) -> Vec<TimelineStep> {
    let Some(root) = timing_root(page) else {
        return Vec::new();
    };
    let mut budget = Budget {
        left: TIMING_NODES,
        spent: false,
    };
    let mut found = Walk {
        found: Vec::new(),
        afters: Vec::new(),
        groups: 0,
        slots: 0,
    };
    for sequence in root.children_of(Ns::Anim, "seq") {
        match sequence.attr(qn!("presentation:node-type")) {
            Some("main-sequence") => walk(sequence, false, &mut budget, &mut found),
            Some("interactive-sequence") => walk(sequence, true, &mut budget, &mut found),
            _ => {}
        }
    }
    let mut reader = Reader {
        ctx,
        targets: Targets::of(page, path, elements),
        size,
        afters: BTreeMap::new(),
    };
    for node in &found.afters {
        let master = node.attr(qn!("presentation:master-element"));
        match (master, after_effect(node)) {
            (Some(master), Some(after)) => {
                reader.afters.insert(master, after);
            }
            _ => report_timing(
                ctx,
                DiagnosticSite::new(),
                "after effect",
                "an after effect names no effect or does nothing the timeline holds; it is left out".to_owned(),
            ),
        }
    }

    let mut steps = Vec::new();
    let mut group = None;
    let mut previous_slot = None;
    for found in &found.found {
        let first_of_group = group != Some(found.group);
        if steps.len() >= STEPS_PER_SLIDE {
            report_timing(
                ctx,
                DiagnosticSite::new(),
                "timeline",
                format!("the slide has more than {STEPS_PER_SLIDE} steps; the rest are left out"),
            );
            break;
        }
        let step = reader.lower(found, &mut budget, first_of_group, previous_slot);
        if let Some(step) = step {
            group = Some(found.group);
            previous_slot = Some(found.slot);
            steps.push(step);
        } else if !found.click || !first_of_group {
            group = Some(found.group);
            previous_slot = Some(found.slot);
        }
    }
    if budget.spent {
        report_timing(
            ctx,
            DiagnosticSite::new(),
            "timing tree",
            format!("the timing tree has more than {TIMING_NODES} nodes; the rest is left out"),
        );
    }
    steps
}
