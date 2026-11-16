//! Build effects: entrances, exits, emphases, motion paths, media commands and tier-2 keyframes.
//! Entrance and exit share recipes: an exit `E(dir)` plays as `reverse(entrance E(opposite(dir)))` and leaves the
//! target hidden, its other parameters unchanged. So `dir` is the direction the content moves in both, while
//! `inOut`, `axis` and the rest name the entrance an exit reverses: an exit `zoom{in}` reverses the zoom-in entrance
//! (from small to its size), so it shrinks away, and an exit `zoom{out}` grows away.
//!
//! Prints as `{"effect": "<name>", "params": {...}}`; an effect without parameters prints no `params`. The
//! parameters refuse unknown keys; a key beside `effect` and `params` is for the containing object to refuse (the
//! IR's native effect does) or ignore (a deck reader does).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::keyframes::{Keyframes, PathSeg, Scale, validate_path};
use super::limits;
use super::params::{
    Axis, ClockStart, ColorProperty, Diagonal, Dir, Dir4, Distance, HexColor, InOut, IrisShape,
    Orient, Vertical, ZoomOrigin,
};
use crate::diagnostics::codes;
use crate::timeline::{StepFallback, StepKind};
use crate::units::{Deg, Ms, Ratio, UnitRatio};
use crate::validate::{Validate, Validator};

/// A direction the content moves, any of eight.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct DirParams {
    pub dir: Dir,
}

/// A straight direction the content moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct Dir4Params {
    pub dir: Dir4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct OrientParams {
    pub orient: Orient,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct AxisParams {
    pub axis: Axis,
}

/// A wipe: the clip's leading edge moves along `dir`; `soft` feathers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct WipeParams {
    pub dir: Dir,
    pub soft: bool,
}

/// A split along a line of `orient`, opening outward or closing inward.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub struct SplitParams {
    pub orient: Orient,
    pub in_out: InOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub struct IrisParams {
    pub shape: IrisShape,
    pub in_out: InOut,
}

/// A wheel of 1 to 8 spokes, turning clockwise unless `cw` is false.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct WheelParams {
    pub spokes: u8,
    pub cw: bool,
}

impl Validate for WheelParams {
    fn validate(&self, v: &mut Validator) {
        v.at("spokes", |v| {
            v.int(i64::from(self.spokes), 1, i64::from(limits::SPOKES))
        });
    }
}

/// A single clock sweep from an hour of the clock face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct ClockParams {
    pub cw: bool,
    pub from: ClockStart,
}

/// Diagonal strips moving toward `corner`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct StripsParams {
    pub corner: Diagonal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct FlyInParams {
    pub dir: Dir,
    pub distance: Distance,
}

/// PowerPoint's Float In: rises (`u`, Ascend) or descends (`d`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct FloatInParams {
    pub dir: Vertical,
}

/// The element slides in from just outside its own box, clipped to it; `slow` is Crawl.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct PeekParams {
    pub dir: Dir4,
    pub slow: bool,
}

/// A zoom in (from small) or out (from large) about `origin`; `slight` scales from near the final size, `fade`
/// fades in as well, `overshoot` passes the final size and settles (Magnify). An exit plays it in reverse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub struct ZoomParams {
    pub in_out: InOut,
    pub origin: ZoomOrigin,
    pub slight: bool,
    pub fade: bool,
    pub overshoot: bool,
}

/// A spin of `deg` degrees in the direction `cw` gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct SpinInParams {
    pub deg: Deg,
    pub cw: bool,
}

fn spin_degrees(v: &mut Validator, deg: Deg) {
    v.at("deg", |v| {
        v.min(deg.to_f64(), 0.0, true);
        v.max(deg.to_f64(), limits::SPIN_DEGREES);
    });
}

impl Validate for SpinInParams {
    fn validate(&self, v: &mut Validator) {
        spin_degrees(v, self.deg);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct PulseParams {
    pub scale: Ratio,
}

/// Scales to `scale`, kept when `hold`, otherwise back to the start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct GrowShrinkParams {
    pub scale: Scale,
    pub hold: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct SpinParams {
    pub deg: Deg,
    pub cw: bool,
    pub hold: bool,
}

/// How strongly an element teeters, jiggles or waves (0 to 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct IntensityParams {
    pub intensity: UnitRatio,
}

/// How many times a blink or flash plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct RepeatParams {
    pub repeat: u32,
}

/// Fades to `alpha`, kept when `hold`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct TransparencyParams {
    pub alpha: UnitRatio,
    pub hold: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct ColorChangeParams {
    pub property: ColorProperty,
    pub to: HexColor,
}

/// Moves along `d`; `autoRotate` turns the element along the path, `hold` keeps it at the end.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub struct PathParams {
    pub d: Vec<PathSeg>,
    pub auto_rotate: bool,
    pub hold: bool,
}

/// Starts the medium at `from` milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct PlayParams {
    pub from: Ms,
}

/// What a build step plays, in slidock's vocabulary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "effect", content = "params", rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub enum BuildEffect {
    // Entrances and exits.
    Appear,
    Fade,
    Dissolve,
    FlyIn(FlyInParams),
    FloatIn(FloatInParams),
    Peek(PeekParams),
    Wipe(WipeParams),
    Split(SplitParams),
    Iris(IrisParams),
    Wheel(WheelParams),
    Wedge,
    Clock(ClockParams),
    Blinds(OrientParams),
    Checker(Dir4Params),
    RandomBars(OrientParams),
    Strips(StripsParams),
    Zoom(ZoomParams),
    Grow(AxisParams),
    Expand(AxisParams),
    Compress(AxisParams),
    Stretch(AxisParams),
    GrowTurn,
    Swivel(AxisParams),
    Flip(AxisParams),
    SpinIn(SpinInParams),
    Bounce,
    Drop,
    Pop,
    Boomerang,
    Sling,
    Whip,
    Swish,
    Typewriter,
    Credits,
    DriftAndScale(DirParams),
    FadeAndMove(DirParams),
    /// One of the implemented entrances, picked by a per-slide seed.
    Random,
    // Emphases.
    Pulse(PulseParams),
    GrowShrink(GrowShrinkParams),
    Spin(SpinParams),
    Teeter(IntensityParams),
    Jiggle(IntensityParams),
    Wave(IntensityParams),
    Blink(RepeatParams),
    Flash(RepeatParams),
    BoldFlash(RepeatParams),
    Transparency(TransparencyParams),
    ColorChange(ColorChangeParams),
    // Motion.
    Path(PathParams),
    // Media.
    Play(PlayParams),
    Pause,
    Stop,
    Toggle,
    /// Tier 2: keyframes computed from the source's behaviours.
    Behaviors(Vec<Keyframes>),
}

/// The family a build effect belongs to, which decides the step kinds it can play in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuildFamily {
    Entrance,
    Emphasis,
    Motion,
    Media,
    Behaviors,
}

impl BuildEffect {
    /// The effect's name as it prints (`flyIn`).
    pub fn name(&self) -> &'static str {
        match self {
            BuildEffect::Appear => "appear",
            BuildEffect::Fade => "fade",
            BuildEffect::Dissolve => "dissolve",
            BuildEffect::FlyIn(_) => "flyIn",
            BuildEffect::FloatIn(_) => "floatIn",
            BuildEffect::Peek(_) => "peek",
            BuildEffect::Wipe(_) => "wipe",
            BuildEffect::Split(_) => "split",
            BuildEffect::Iris(_) => "iris",
            BuildEffect::Wheel(_) => "wheel",
            BuildEffect::Wedge => "wedge",
            BuildEffect::Clock(_) => "clock",
            BuildEffect::Blinds(_) => "blinds",
            BuildEffect::Checker(_) => "checker",
            BuildEffect::RandomBars(_) => "randomBars",
            BuildEffect::Strips(_) => "strips",
            BuildEffect::Zoom(_) => "zoom",
            BuildEffect::Grow(_) => "grow",
            BuildEffect::Expand(_) => "expand",
            BuildEffect::Compress(_) => "compress",
            BuildEffect::Stretch(_) => "stretch",
            BuildEffect::GrowTurn => "growTurn",
            BuildEffect::Swivel(_) => "swivel",
            BuildEffect::Flip(_) => "flip",
            BuildEffect::SpinIn(_) => "spinIn",
            BuildEffect::Bounce => "bounce",
            BuildEffect::Drop => "drop",
            BuildEffect::Pop => "pop",
            BuildEffect::Boomerang => "boomerang",
            BuildEffect::Sling => "sling",
            BuildEffect::Whip => "whip",
            BuildEffect::Swish => "swish",
            BuildEffect::Typewriter => "typewriter",
            BuildEffect::Credits => "credits",
            BuildEffect::DriftAndScale(_) => "driftAndScale",
            BuildEffect::FadeAndMove(_) => "fadeAndMove",
            BuildEffect::Random => "random",
            BuildEffect::Pulse(_) => "pulse",
            BuildEffect::GrowShrink(_) => "growShrink",
            BuildEffect::Spin(_) => "spin",
            BuildEffect::Teeter(_) => "teeter",
            BuildEffect::Jiggle(_) => "jiggle",
            BuildEffect::Wave(_) => "wave",
            BuildEffect::Blink(_) => "blink",
            BuildEffect::Flash(_) => "flash",
            BuildEffect::BoldFlash(_) => "boldFlash",
            BuildEffect::Transparency(_) => "transparency",
            BuildEffect::ColorChange(_) => "colorChange",
            BuildEffect::Path(_) => "path",
            BuildEffect::Play(_) => "play",
            BuildEffect::Pause => "pause",
            BuildEffect::Stop => "stop",
            BuildEffect::Toggle => "toggle",
            BuildEffect::Behaviors(_) => "behaviors",
        }
    }

    pub fn family(&self) -> BuildFamily {
        match self {
            BuildEffect::Pulse(_)
            | BuildEffect::GrowShrink(_)
            | BuildEffect::Spin(_)
            | BuildEffect::Teeter(_)
            | BuildEffect::Jiggle(_)
            | BuildEffect::Wave(_)
            | BuildEffect::Blink(_)
            | BuildEffect::Flash(_)
            | BuildEffect::BoldFlash(_)
            | BuildEffect::Transparency(_)
            | BuildEffect::ColorChange(_) => BuildFamily::Emphasis,
            BuildEffect::Path(_) => BuildFamily::Motion,
            BuildEffect::Play(_) | BuildEffect::Pause | BuildEffect::Stop | BuildEffect::Toggle => {
                BuildFamily::Media
            }
            BuildEffect::Behaviors(_) => BuildFamily::Behaviors,
            _ => BuildFamily::Entrance,
        }
    }

    /// Whether a step of `kind` can play the effect: entrances, exits and emphases play entrance and emphasis
    /// recipes (PowerPoint's Flash Once enters with a blink; Keynote's action builds emphasise with entrance
    /// recipes); motion plays a path; media plays a media command; tier-2 keyframes play in any kind but media.
    pub fn plays_in(&self, kind: StepKind) -> bool {
        match (self.family(), kind) {
            (
                BuildFamily::Entrance | BuildFamily::Emphasis,
                StepKind::Entrance | StepKind::Exit | StepKind::Emphasis,
            ) => true,
            (BuildFamily::Motion, StepKind::Motion) => true,
            (BuildFamily::Media, StepKind::Media) => true,
            (BuildFamily::Behaviors, kind) => kind != StepKind::Media,
            _ => false,
        }
    }

    /// The canonical fallback for a step of `kind` playing the effect: entrances and exits
    /// dissolve, except an appear or a typewriter, which appear; emphases, motion and media play none.
    pub fn fallback(&self, kind: StepKind) -> StepFallback {
        match kind {
            StepKind::Emphasis | StepKind::Motion | StepKind::Media => StepFallback::None,
            StepKind::Entrance | StepKind::Exit => match self {
                BuildEffect::Appear | BuildEffect::Typewriter => StepFallback::Appear,
                _ => StepFallback::Dissolve,
            },
        }
    }
}

impl Validate for BuildEffect {
    /// Validates the parameters, at the path of the `params` value.
    fn validate(&self, v: &mut Validator) {
        match self {
            BuildEffect::Wheel(wheel) => wheel.validate(v),
            BuildEffect::SpinIn(spin) => spin.validate(v),
            BuildEffect::Spin(spin) => spin_degrees(v, spin.deg),
            BuildEffect::Pulse(pulse) => v.at("scale", |v| {
                v.min(pulse.scale.to_f64(), 0.0, true);
                v.max(pulse.scale.to_f64(), limits::SCALE);
            }),
            BuildEffect::GrowShrink(grow) => v.at("scale", |v| {
                for (axis, value) in [("x", grow.scale.x), ("y", grow.scale.y)] {
                    v.at(axis, |v| {
                        v.min(value.to_f64(), 0.0, false);
                        v.max(value.to_f64(), limits::SCALE);
                    });
                }
            }),
            BuildEffect::Teeter(p) | BuildEffect::Jiggle(p) | BuildEffect::Wave(p) => {
                v.field("intensity", &p.intensity);
            }
            BuildEffect::Blink(p) | BuildEffect::Flash(p) | BuildEffect::BoldFlash(p) => {
                v.at("repeat", |v| {
                    v.int(i64::from(p.repeat), 1, i64::from(limits::REPEAT_COUNT));
                });
            }
            BuildEffect::Transparency(p) => v.field("alpha", &p.alpha),
            BuildEffect::Path(path) => v.at("d", |v| validate_path(v, &path.d)),
            BuildEffect::Play(play) => v.field("from", &play.from),
            BuildEffect::Behaviors(tracks) => {
                v.array(tracks.len(), 1, limits::TRACKS);
                tracks.validate(v);
            }
            BuildEffect::Appear
            | BuildEffect::Fade
            | BuildEffect::Dissolve
            | BuildEffect::FlyIn(_)
            | BuildEffect::FloatIn(_)
            | BuildEffect::Peek(_)
            | BuildEffect::Wipe(_)
            | BuildEffect::Split(_)
            | BuildEffect::Iris(_)
            | BuildEffect::Wedge
            | BuildEffect::Clock(_)
            | BuildEffect::Blinds(_)
            | BuildEffect::Checker(_)
            | BuildEffect::RandomBars(_)
            | BuildEffect::Strips(_)
            | BuildEffect::Zoom(_)
            | BuildEffect::Grow(_)
            | BuildEffect::Expand(_)
            | BuildEffect::Compress(_)
            | BuildEffect::Stretch(_)
            | BuildEffect::GrowTurn
            | BuildEffect::Swivel(_)
            | BuildEffect::Flip(_)
            | BuildEffect::Bounce
            | BuildEffect::Drop
            | BuildEffect::Pop
            | BuildEffect::Boomerang
            | BuildEffect::Sling
            | BuildEffect::Whip
            | BuildEffect::Swish
            | BuildEffect::Typewriter
            | BuildEffect::Credits
            | BuildEffect::DriftAndScale(_)
            | BuildEffect::FadeAndMove(_)
            | BuildEffect::Random
            | BuildEffect::ColorChange(_)
            | BuildEffect::Pause
            | BuildEffect::Stop
            | BuildEffect::Toggle => {}
        }
    }
}

/// Reports a step whose kind cannot play its effect.
pub(crate) fn check_kind(v: &mut Validator, effect: &BuildEffect, kind: StepKind) {
    if !effect.plays_in(kind) {
        v.invalid(
            codes::IR0001,
            format!("a {kind:?} step cannot play the {} effect", effect.name()),
        );
    }
}
