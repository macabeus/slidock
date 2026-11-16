//! slidock's common effect vocabulary (`docs/format.md` §5): the transitions and build effects
//! every frontend maps its native data to, with typed parameters, and the timing a player needs beyond the IR's
//! linear step model. The compiler emits data, never code: a player maps each entry to Web Animations, and plays the
//! IR's canonical fallback for anything it does not know.
//!
//! One direction convention holds everywhere: `dir` is the direction the content moves, in OOXML's tokens
//! (`l r u d lu ru ld rd`).

mod build;
mod keyframes;
pub mod limits;
mod native;
mod params;
mod retime;
mod strip;
mod timing;
mod transition;

pub(crate) use build::check_kind;
pub use build::{
    AxisParams, BuildEffect, BuildFamily, ClockParams, ColorChangeParams, Dir4Params, DirParams,
    FloatInParams, FlyInParams, GrowShrinkParams, IntensityParams, IrisParams, OrientParams,
    PathParams, PeekParams, PlayParams, PulseParams, RepeatParams, SpinInParams, SpinParams,
    SplitParams, StripsParams, TransparencyParams, WheelParams, WipeParams, ZoomParams,
};
pub use keyframes::{
    ClipTrack, ColorMix, ColorValue, Frame, HslShift, Keyframes, Offset, Paint, PathSeg, Reveal,
    Scale, SetTrack, SetValue, Track,
};
pub use native::{InvalidNativeId, NativeId, NativeStep, NativeTransition};
pub use params::{
    Axis, ClockStart, ColorProperty, Diagonal, Dir, Dir4, Distance, HexColor, InOut, InvalidColor,
    IrisShape, MorphBy, Orient, Through, Vertical, ZoomOrigin,
};
pub use retime::{RepeatWritten, Retime, TimeFilter, UnfoldedRepeat};
pub use strip::strip_native_motion;
pub use timing::{AfterEffect, Easing, Repeat, Stagger, StaggerAmount, StaggerOrder, StepTiming};
pub use transition::{
    CutParams, FadeParams, FlythroughParams, MagicMoveParams, MorphParams, RevealParams,
    TransitionEffect, TransitionZoomParams,
};

pub use crate::timeline::{
    Advance, Delivery, StepFallback, StepKind, StepTarget, Transition, TransitionFallback, Trigger,
};
