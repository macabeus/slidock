//! Slide transitions (`p:transition`, Office 2010's inside alternate content). A transition becomes data: an opaque
//! `pptx:<effect>` id the presentation runtime plays, with a canonical fallback.
//!
//! On the parity path ([`read_transition`]) only the effect's name is read, from the branch the static reader
//! resolves. With native motion ([`read_native_transition`]) the transition is read from the first alternate-content
//! choice the motion reader understands (`p14`, `p15`, `p159`), else the fallback, and its effect and options map
//! to the common vocabulary through the shared table (`slidock_presets::pptx::transition`); the fallback, duration
//! and advance keep the parity path's rules; a transition without an effect cuts. An effect with no vocabulary entry
//! is PX2402, one the vocabulary only approximates PX2413, an option value read as its default PX2412, an advance
//! time or duration that is clamped or does not read PX2411; the transition's sound has no place in the IR and is
//! reported (PX2411).

use slidock_canon::js_number;
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::effects::NativeTransition;
use slidock_ir::features::FeatureId;
use slidock_ir::formats::{EffectId, FileFormat, SourceFormat};
use slidock_ir::timeline::{Advance, Transition, TransitionEffect, TransitionFallback};
use slidock_ir::units::Ms;

use crate::context::Lowering;
use crate::report::Report;
use crate::shapes::tree_children;
use crate::timing::root_children;
use crate::xml::{El, Element, Ns};

/// PowerPoint's speeds in milliseconds, for a transition that omits `dur`.
fn speed(name: &str) -> f64 {
    match name {
        "slow" => 1000.0,
        "med" => 750.0,
        _ => 500.0,
    }
}

fn fallback(name: &str) -> Option<TransitionFallback> {
    match name {
        "fade" | "dissolve" => Some(TransitionFallback::Dissolve),
        "push" => Some(TransitionFallback::Push),
        "wipe" => Some(TransitionFallback::Wipe),
        _ => None,
    }
}

/// A transition's children other than its effect.
const NOT_EFFECTS: [&str; 2] = ["sndAc", "extLst"];

const HOUR_MS: f64 = 3_600_000.0;

/// The transition's effect element, its name (`none` without one) and its fallback: a dissolve for an effect the
/// fallbacks do not name.
fn effect_of(transition: &Element) -> (Option<&Element>, &str, TransitionFallback) {
    let effect = transition
        .kids()
        .find(|kid| !NOT_EFFECTS.contains(&&*kid.local))
        .map(|rc| &**rc);
    let name = effect.map_or("none", |effect| &*effect.local);
    let fallback = match effect {
        Some(_) => fallback(name).unwrap_or(TransitionFallback::Dissolve),
        None => TransitionFallback::None,
    };
    (effect, name, fallback)
}

/// How a value read when not as written: clamped to its limit, or invalid (it reads as the default).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Read {
    Clamped,
    Invalid,
}

impl Read {
    fn token(self) -> &'static str {
        match self {
            Read::Clamped => "clamped",
            Read::Invalid => "invalid",
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Read::Clamped => "a value past its limit is clamped",
            Read::Invalid => "a value that is not one reads as the default",
        }
    }
}

/// The duration: any attribute ending in `:dur` (`p14:dur`, in ms, capped at an hour), else the speed; and the `dur`
/// as written with how it read, when not as written.
fn duration_of(transition: &Element) -> (Ms, Option<(String, Read)>) {
    let written = transition.attrs().find(|(key, _)| key.ends_with(":dur"));
    let duration = written.map_or(f64::NAN, |(_, value)| js_number(value));
    let (duration_ms, mut problem) = if duration.is_finite() && duration >= 0.0 {
        (duration, None)
    } else {
        (
            speed(transition.attr("spd").unwrap_or("fast")),
            written.map(|(_, value)| (value.to_owned(), Read::Invalid)),
        )
    };
    let rounded = slidock_canon::js_round(duration_ms);
    if rounded > HOUR_MS {
        problem = written.map(|(_, value)| (value.to_owned(), Read::Clamped));
    }
    (Ms(rounded.min(HOUR_MS) as u32), problem)
}

/// The advance: on a click (`advClick`, default true), after `advTm` ms (capped at an hour).
fn advance_of(transition: &Element) -> Advance {
    let after_ms = Some(transition)
        .int_attr("advTm")
        .filter(|after| *after >= 0)
        .map(|after| Ms(u32::try_from(after.min(HOUR_MS as i64)).unwrap_or(0)));
    Advance {
        on_click: Some(transition).bool_attr("advClick").unwrap_or(true),
        after_ms,
    }
}

pub(crate) fn read_transition(root: &Element, ctx: &mut Lowering<'_>) -> Option<Transition> {
    let transition = tree_children(root, ctx, &format!("/{}", root.name))
        .into_iter()
        .map(|(element, _)| element)
        .find(|element| element.is(Ns::P, "transition"))?;
    ctx.seen(FeatureId::SlideTransition);
    let (effect, name, fallback) = effect_of(transition);
    if effect.is_some() && self::fallback(name).is_none() {
        Report::report(
            ctx,
            codes::PX2402,
            DiagnosticSite::new().data("effect", name),
        );
    }
    let (duration_ms, _) = duration_of(transition);
    Some(Transition {
        effect: TransitionEffect {
            id: EffectId::new(SourceFormat::File(FileFormat::Pptx), name),
            fallback,
            native: None,
        },
        duration_ms,
        advance: Some(advance_of(transition)),
        easing: None,
    })
}

/// How the advance time read, when not as written: clamped to an hour, or invalid (negative, not a number).
fn advance_problem(transition: &Element) -> Option<(&str, Read)> {
    let written = transition.attr("advTm")?;
    match Some(transition).int_attr("advTm") {
        Some(after) if after > HOUR_MS as i64 => Some((written, Read::Clamped)),
        Some(after) if after >= 0 => None,
        _ => Some((written, Read::Invalid)),
    }
}

/// The transition with its native effect (native motion): see the module doc.
pub(crate) fn read_native_transition(root: &Element, ctx: &mut Lowering<'_>) -> Option<Transition> {
    let transition = root_children(root)
        .into_iter()
        .find(|element| element.is(Ns::P, "transition"))?;
    ctx.seen(FeatureId::SlideTransition);
    let (effect, name, fallback) = effect_of(transition);
    let prst = effect
        .filter(|effect| &*effect.local == "prstTrans")
        .and_then(|effect| effect.attr("prst"));
    let id = slidock_presets::pptx::transition_id(name, prst);
    let site = || DiagnosticSite::new().data("effect", id.name());
    let lookup =
        effect.and_then(|effect| slidock_presets::pptx::transition(name, |attr| effect.attr(attr)));
    // A transition without an effect only advances: it cuts.
    let mut native = effect
        .is_none()
        .then_some(slidock_ir::effects::TransitionEffect::None);
    let mut approximate = false;
    if let Some(lookup) = lookup {
        for unknown in &lookup.unknown {
            Report::report(
                ctx,
                codes::PX2412,
                site()
                    .data("option", unknown.option)
                    .data("value", unknown.value.as_str())
                    .data("used", unknown.used),
            );
        }
        native = lookup.row.effect;
        approximate = lookup.row.approximate;
    }
    match native {
        None => Report::report(ctx, codes::PX2402, site()),
        Some(effect) if approximate => Report::report(
            ctx,
            codes::PX2413,
            site().data(
                "reason",
                format!("{} only approximates the transition", effect.name()),
            ),
        ),
        Some(_) => {}
    }
    let mut lossy = false;
    let (duration_ms, bad_duration) = duration_of(transition);
    let problems = [
        bad_duration.map(|(written, read)| ("durationMs", written, read)),
        advance_problem(transition).map(|(written, read)| ("afterMs", written.to_owned(), read)),
    ];
    for (what, written, read) in problems.into_iter().flatten() {
        lossy = true;
        Report::report(
            ctx,
            codes::PX2411,
            site()
                .data("what", what)
                .data("reason", read.reason())
                .data("native", written)
                .data("read", read.token())
                .data_opt(
                    "limit",
                    (read == Read::Clamped).then_some("slidock_ir::units::Ms::MAX"),
                ),
        );
    }
    if transition.child(Ns::P, "sndAc").is_some() {
        lossy = true;
        Report::report(
            ctx,
            codes::PX2411,
            site()
                .data("what", "sound")
                .data("reason", "the transition's sound is not played"),
        );
    }
    if lossy {
        ctx.seen(FeatureId::Animation);
    }
    Some(Transition {
        effect: TransitionEffect {
            id: id.effect_id(),
            fallback,
            native: native.map(|effect| {
                Box::new(NativeTransition {
                    id: id.clone(),
                    effect,
                })
            }),
        },
        duration_ms,
        advance: Some(advance_of(transition)),
        easing: None,
    })
}
