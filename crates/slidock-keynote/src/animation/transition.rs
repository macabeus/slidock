//! A slide's transition. Its IR fields are the effect id and fallback, the
//! duration, and an automatic transition's advance after its delay. The native data adds the vocabulary entry (with the
//! direction, a fade's colour and Magic Move's options) and the timing curve. A transition stored only in the legacy
//! `database_*` fields is left out and reported (KN1409).
//!
//! Keynote stores a transition on the slide it leaves: its movie of the transition catalog plays slide N's effect
//! between slides N and N + 1, and its PPTX export writes it on slide N + 1. The parity path keeps it where Keynote
//! stores it ([`Placement::Stored`]). The deck path holds the IR's convention, the transition into a slide
//! ([`Placement::Into`]): a shown slide plays the effect stored on the shown slide before it, with its own advance (how
//! long it stays before an automatic transition leaves it); a skipped slide plays none, as Keynote never leaves it; the
//! last shown slide's own effect leads nowhere and is left out, reported (KN1409). A transition's other reports
//! (KN1403, its feature) go with the slide it plays into.

use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::effects::{NativeTransition, TransitionEffect as VocabularyTransition};
use slidock_ir::features::FeatureId;
use slidock_ir::timeline::{Advance, Transition, TransitionEffect, TransitionFallback};
use slidock_ir::units::Ms;

use super::effects::{native_transition, transition_fallback};
use super::timeline::effect_id;
use crate::context::Lowering;
use crate::num::ms;
use crate::proto::kn;

const NO_TRANSITION: &str = "none";

/// The effect id of a slide's transition, `None` for none.
pub fn transition_effect(slide: &kn::SlideArchive) -> Option<&str> {
    slide
        .transition
        .as_ref()
        .and_then(|transition| transition.attributes.as_ref())
        .and_then(|attributes| attributes.animation_attributes.as_ref())
        .and_then(|attributes| attributes.effect.as_deref())
        .filter(|effect| !effect.is_empty() && *effect != NO_TRANSITION)
}

/// Which transition a slide carries.
#[derive(Clone, Copy, Debug)]
pub enum Placement<'s> {
    /// The one stored on the slide, which Keynote plays as the slide is left (the parity path).
    Stored,
    /// The one into the slide (the deck path): the effect stored on `previous`, the shown slide before it, if any.
    Into {
        previous: Option<&'s kn::SlideArchive>,
        /// The slide is skipped: nothing plays into it.
        skipped: bool,
        /// The slide is the last shown one: its own effect leads nowhere.
        last: bool,
    },
}

/// A slide's transition as `placement` places it.
pub fn placed_transition(
    ctx: &mut Lowering<'_>,
    slide: &kn::SlideArchive,
    placement: Placement<'_>,
) -> Option<Transition> {
    let Placement::Into {
        previous,
        skipped,
        last,
    } = placement
    else {
        return transition(ctx, slide);
    };
    if skipped {
        return None;
    }
    if last && let Some(effect) = transition_effect(slide) {
        ctx.report_motion(
            codes::KN1409,
            DiagnosticSite::new().data("effect", effect).data(
                "reason",
                "it leaves the last shown slide, and no slide follows it",
            ),
        );
    }
    let advance = slide
        .transition
        .as_ref()
        .and_then(|transition| transition.attributes.as_ref())
        .and_then(|attributes| attributes.animation_attributes.as_ref())
        .and_then(advance_of);
    match previous.and_then(|previous| transition(ctx, previous)) {
        Some(incoming) => Some(Transition {
            advance,
            ..incoming
        }),
        None => advance.map(|advance| Transition {
            effect: TransitionEffect {
                id: effect_id(NO_TRANSITION),
                fallback: TransitionFallback::None,
                native: Some(Box::new(NativeTransition {
                    id: slidock_presets::keynote::effect_id(NO_TRANSITION),
                    effect: VocabularyTransition::None,
                })),
            },
            duration_ms: Ms(0),
            advance: Some(advance),
            easing: None,
        }),
    }
}

/// An automatic transition's advance: after its delay, with no click.
fn advance_of(attributes: &kn::AnimationAttributesArchive) -> Option<Advance> {
    (attributes.is_automatic == Some(true)).then(|| Advance {
        on_click: false,
        after_ms: Some(ms(attributes.delay)),
    })
}

/// The transition stored on a slide, as Keynote plays it when the slide is left.
pub fn transition(ctx: &mut Lowering<'_>, slide: &kn::SlideArchive) -> Option<Transition> {
    let transition = slide.transition.as_ref()?.attributes.as_ref()?;
    let Some(attributes) = transition.animation_attributes.as_ref() else {
        if let Some(legacy) = transition
            .database_effect
            .as_deref()
            .filter(|effect| !effect.is_empty() && *effect != NO_TRANSITION)
        {
            ctx.report_motion(
                codes::KN1409,
                DiagnosticSite::new().data("effect", legacy).data(
                    "reason",
                    "it is stored in the legacy fields, which are not read",
                ),
            );
        }
        return None;
    };
    let effect = transition_effect(slide)?;
    let fallback = transition_fallback(effect);
    if fallback.is_none() {
        ctx.report(codes::KN1403, DiagnosticSite::new().data("effect", effect));
    }
    ctx.see(FeatureId::SlideTransition);
    let native = native_transition(ctx, effect, transition);
    Some(Transition {
        effect: TransitionEffect {
            id: effect_id(effect),
            fallback: fallback.unwrap_or(TransitionFallback::Dissolve),
            native: native.native,
        },
        duration_ms: ms(attributes.duration),
        advance: advance_of(attributes),
        easing: native.easing,
    })
}
