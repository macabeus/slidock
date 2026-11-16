//! Transition effects. Each plays with two layers, the outgoing slide and the incoming one;
//! `dir` is the direction the content moves. Effects the player does not implement (vortex, ripple, shred, page
//! curl, particle effects, …) have no entry: their transitions carry only the canonical fallback.
//!
//! Prints as `{"effect": "<name>", "params": {...}}`; an effect without parameters prints no `params`. The
//! parameters refuse unknown keys; a key beside `effect` and `params` is for the containing object to refuse (the
//! IR's native effect does) or ignore (a deck reader does).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::build::{
    ClockParams, Dir4Params, DirParams, IrisParams, OrientParams, SplitParams, StripsParams,
    WheelParams, WipeParams,
};
use super::params::{Dir4, HexColor, InOut, MorphBy, Through};
use crate::diagnostics::codes;
use crate::timeline::TransitionFallback;
use crate::validate::{Validate, Validator};

/// A cut, optionally through black.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub struct CutParams {
    pub through_black: bool,
}

/// A fade, through a colour or not; `color` is given exactly when `through` is `color`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct FadeParams {
    pub through: Through,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    #[ts(as = "Option<HexColor>", optional)]
    pub color: Option<HexColor>,
}

impl Validate for FadeParams {
    fn validate(&self, v: &mut Validator) {
        match (self.through, self.color) {
            (Through::Color, None) => v.at("color", |v| {
                v.invalid(codes::IR0001, "a fade through a colour names it")
            }),
            (Through::None | Through::Black | Through::White, Some(_)) => v.at("color", |v| {
                v.invalid(codes::IR0001, "only a fade through a colour names one")
            }),
            _ => {}
        }
    }
}

/// A zoom in or out; `rotate` spins it as it goes (Newsflash).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub struct TransitionZoomParams {
    pub in_out: InOut,
    pub rotate: bool,
}

/// The outgoing slide slides away and fades, optionally through black.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub struct RevealParams {
    pub dir: Dir4,
    pub through_black: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub struct FlythroughParams {
    pub in_out: InOut,
    pub bounce: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct MorphParams {
    pub by: MorphBy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub struct MagicMoveParams {
    pub fade_unmatched: bool,
}

/// What a transition plays, in slidock's vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(tag = "effect", content = "params", rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub enum TransitionEffect {
    /// No effect: the slides swap.
    None,
    Cut(CutParams),
    Fade(FadeParams),
    Dissolve,
    Push(Dir4Params),
    Cover(DirParams),
    /// The outgoing slide moves away along `dir` (PowerPoint's Pull).
    Uncover(DirParams),
    Wipe(WipeParams),
    Split(SplitParams),
    Iris(IrisParams),
    Wheel(WheelParams),
    Clock(ClockParams),
    Wedge,
    Blinds(OrientParams),
    Checker(Dir4Params),
    RandomBars(OrientParams),
    Strips(StripsParams),
    Comb(OrientParams),
    Zoom(TransitionZoomParams),
    Flip(Dir4Params),
    Cube(Dir4Params),
    Doors(OrientParams),
    Window(OrientParams),
    Gallery(Dir4Params),
    Conveyor(Dir4Params),
    Ferris(Dir4Params),
    Pan(Dir4Params),
    Switch(Dir4Params),
    Rotate3d(Dir4Params),
    Reveal(RevealParams),
    Flythrough(FlythroughParams),
    Swap(Dir4Params),
    PageFlip(Dir4Params),
    Twist(Dir4Params),
    Mosaic(Dir4Params),
    Morph(MorphParams),
    MagicMove(MagicMoveParams),
    /// One of the implemented transitions, picked by a per-slide seed.
    Random,
}

impl TransitionEffect {
    /// The effect's name as it prints (`randomBars`).
    pub fn name(&self) -> &'static str {
        match self {
            TransitionEffect::None => "none",
            TransitionEffect::Cut(_) => "cut",
            TransitionEffect::Fade(_) => "fade",
            TransitionEffect::Dissolve => "dissolve",
            TransitionEffect::Push(_) => "push",
            TransitionEffect::Cover(_) => "cover",
            TransitionEffect::Uncover(_) => "uncover",
            TransitionEffect::Wipe(_) => "wipe",
            TransitionEffect::Split(_) => "split",
            TransitionEffect::Iris(_) => "iris",
            TransitionEffect::Wheel(_) => "wheel",
            TransitionEffect::Clock(_) => "clock",
            TransitionEffect::Wedge => "wedge",
            TransitionEffect::Blinds(_) => "blinds",
            TransitionEffect::Checker(_) => "checker",
            TransitionEffect::RandomBars(_) => "randomBars",
            TransitionEffect::Strips(_) => "strips",
            TransitionEffect::Comb(_) => "comb",
            TransitionEffect::Zoom(_) => "zoom",
            TransitionEffect::Flip(_) => "flip",
            TransitionEffect::Cube(_) => "cube",
            TransitionEffect::Doors(_) => "doors",
            TransitionEffect::Window(_) => "window",
            TransitionEffect::Gallery(_) => "gallery",
            TransitionEffect::Conveyor(_) => "conveyor",
            TransitionEffect::Ferris(_) => "ferris",
            TransitionEffect::Pan(_) => "pan",
            TransitionEffect::Switch(_) => "switch",
            TransitionEffect::Rotate3d(_) => "rotate3d",
            TransitionEffect::Reveal(_) => "reveal",
            TransitionEffect::Flythrough(_) => "flythrough",
            TransitionEffect::Swap(_) => "swap",
            TransitionEffect::PageFlip(_) => "pageFlip",
            TransitionEffect::Twist(_) => "twist",
            TransitionEffect::Mosaic(_) => "mosaic",
            TransitionEffect::Morph(_) => "morph",
            TransitionEffect::MagicMove(_) => "magicMove",
            TransitionEffect::Random => "random",
        }
    }

    /// The canonical fallback: what a runtime that does not know the effect plays.
    pub fn fallback(&self) -> TransitionFallback {
        match self {
            TransitionEffect::None | TransitionEffect::Cut(_) => TransitionFallback::None,
            TransitionEffect::Push(_)
            | TransitionEffect::Cover(_)
            | TransitionEffect::Uncover(_)
            | TransitionEffect::Comb(_)
            | TransitionEffect::Flip(_)
            | TransitionEffect::Cube(_)
            | TransitionEffect::Gallery(_)
            | TransitionEffect::Conveyor(_)
            | TransitionEffect::Ferris(_)
            | TransitionEffect::Pan(_)
            | TransitionEffect::Switch(_)
            | TransitionEffect::Rotate3d(_) => TransitionFallback::Push,
            TransitionEffect::Wipe(_)
            | TransitionEffect::Split(_)
            | TransitionEffect::Iris(_)
            | TransitionEffect::Wheel(_)
            | TransitionEffect::Clock(_)
            | TransitionEffect::Wedge
            | TransitionEffect::Blinds(_)
            | TransitionEffect::Checker(_)
            | TransitionEffect::Strips(_)
            | TransitionEffect::Doors(_)
            | TransitionEffect::Window(_) => TransitionFallback::Wipe,
            TransitionEffect::Fade(_)
            | TransitionEffect::Dissolve
            | TransitionEffect::RandomBars(_)
            | TransitionEffect::Zoom(_)
            | TransitionEffect::Reveal(_)
            | TransitionEffect::Flythrough(_)
            | TransitionEffect::Swap(_)
            | TransitionEffect::PageFlip(_)
            | TransitionEffect::Twist(_)
            | TransitionEffect::Mosaic(_)
            | TransitionEffect::Morph(_)
            | TransitionEffect::MagicMove(_)
            | TransitionEffect::Random => TransitionFallback::Dissolve,
        }
    }
}

impl Validate for TransitionEffect {
    /// Validates the parameters, at the path of the `params` value.
    fn validate(&self, v: &mut Validator) {
        match self {
            TransitionEffect::Fade(fade) => fade.validate(v),
            TransitionEffect::Wheel(wheel) => wheel.validate(v),
            _ => {}
        }
    }
}
