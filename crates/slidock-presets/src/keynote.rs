//! Keynote's effects: the ids `AnimationAttributesArchive.effect` holds, for builds (In, Out, Action) and for
//! transitions, and the direction numbers of `direction`/`database_direction`.
//!
//! The ids are the strings in Keynote 14.5's binaries. Their
//! meaning is read from the names, so every row is [`Certainty::Inferred`] except the ones the corpus holds and
//! the parity path already plays, and the ones Keynote's own output settles (its movie of the transition catalog and
//! that catalog's PPTX export, and its build movies of its import of the PPTX probe, `probes/keynote/RUNBOOK.md`); the
//! catalog decks confirm the rest. A Keynote direction becomes the effect's `dir` (rows with `refine` hold a default
//! until then). The Keynote frontend reads this table and nothing else: what Keynote's files show is written here.

use slidock_ir::effects::{
    Axis, AxisParams, BuildEffect, ClockParams, ClockStart, Delivery, Dir, Dir4, Dir4Params,
    DirParams, Distance, Easing, FadeParams, FlyInParams, GrowShrinkParams, InOut, IntensityParams,
    IrisParams, IrisShape, MagicMoveParams, NativeId, Orient, OrientParams, PathParams, PathSeg,
    PlayParams, PulseParams, RepeatParams, Scale, SpinInParams, SpinParams, StepKind, Through,
    TransitionEffect, TransitionFallback, TransitionZoomParams, TransparencyParams, WipeParams,
    ZoomOrigin, ZoomParams,
};
use slidock_ir::formats::FileFormat;
use slidock_ir::units::{Deg, Ms, Px, Ratio, UnitRatio};

use crate::{BuildRow, Certainty, TransitionRow};

/// The prefix of Keynote's plug-in effects.
pub const PLUGIN: &str = "com.apple.iWork.Keynote.";

/// Every id the table knows: Keynote 14.5's built-in effect ids, then the plug-in ids harvested beside them.
pub const IDS: &[&str] = &[
    "apple:3d-crane",
    "apple:3d-rotate",
    "apple:3d-rotatinggrow",
    "apple:action-blink",
    "apple:action-bounce",
    "apple:action-flip",
    "apple:action-jiggle",
    "apple:action-motion-path",
    "apple:action-opacity",
    "apple:action-pop",
    "apple:action-pulse",
    "apple:action-rotation",
    "apple:action-scale",
    "apple:appear",
    "apple:apple-grid",
    "apple:audio-start",
    "apple:audio-stop",
    "apple:bc-3",
    "apple:bc-appear",
    "apple:bc-dissolve",
    "apple:bc-drop",
    "apple:bc-expand",
    "apple:bc-flip",
    "apple:bc-move",
    "apple:bc-orbital",
    "apple:bc-pop",
    "apple:bc-scale-big",
    "apple:bc-superflip",
    "apple:bc-typewriter",
    "apple:bc-zoom",
    "apple:bc-zoom-big",
    "apple:bc-zoom-big character",
    "apple:bounce",
    "apple:burn",
    "apple:bw-dissolve",
    "apple:bw-pop",
    "apple:bw-zoom-big",
    "apple:c-dissolve",
    "apple:ca-anagram",
    "apple:ca-cube",
    "apple:ca-dissolve-and-flip",
    "apple:ca-implied-motion-path",
    "apple:ca-isometric",
    "apple:ca-pop",
    "apple:ca-push",
    "apple:ca-revolve",
    "apple:ca-swing",
    "apple:ca-text-shimmer",
    "apple:ca-text-sparkle",
    "apple:ca-zoom",
    "apple:compress",
    "apple:dissolve",
    "apple:dissolve character",
    "apple:doorway",
    "apple:drift",
    "apple:drift and scale",
    "apple:drift and scale character",
    "apple:drift object",
    "apple:drive-by",
    "apple:drive-by object",
    "apple:drop",
    "apple:dropbuild",
    "apple:dropbuild2",
    "apple:droplet",
    "apple:emphasis-motion-path",
    "apple:emphasis-opacity",
    "apple:emphasis-rotation",
    "apple:emphasis-scale",
    "apple:fade",
    "apple:fade and move",
    "apple:fade and move character",
    "apple:fade-and-move",
    "apple:fall",
    "apple:fall-apart",
    "apple:fallingtiles",
    "apple:flash",
    "apple:gallery-appear",
    "apple:gallery-dissolve",
    "apple:gallery-fade-and-move",
    "apple:gallery-move",
    "apple:keyboard",
    "apple:magic chart",
    "apple:magic-move-implied-motion-path",
    "apple:motion dissolve",
    "apple:move in",
    "apple:move in character",
    "apple:movie-start",
    "apple:movie-stop",
    "apple:pageflip",
    "apple:pivot",
    "apple:pivot-build",
    "apple:pop",
    "apple:push",
    "apple:reveal",
    "apple:revolve",
    "apple:scale",
    "apple:sidezoom",
    "apple:slide",
    "apple:spin",
    "apple:superflip",
    "apple:twirl",
    "apple:twist-and-scale",
    "apple:web-video-start",
    "apple:wipe",
    "apple:wipe-iris",
    "apple:wipe-radial",
    "apple:zoom",
    "apple:zoom character",
    "com.apple.iWork.Keynote.BLTBlinds",
    "com.apple.iWork.Keynote.BLTFadeThruColor",
    "com.apple.iWork.Keynote.BLTMosaicFlip",
    "com.apple.iWork.Keynote.BLTReflection",
    "com.apple.iWork.Keynote.BLTRevolvingDoor",
    "com.apple.iWork.Keynote.BLTShutter",
    "com.apple.iWork.Keynote.BLTSwoosh",
    "com.apple.iWork.Keynote.BUKAnvil",
    "com.apple.iWork.Keynote.BUKFlashBulbs",
    "com.apple.iWork.Keynote.BUKFlop",
    "com.apple.iWork.Keynote.BUKLensFlare",
    "com.apple.iWork.Keynote.BUKTwist",
    "com.apple.iWork.Keynote.Blur",
    "com.apple.iWork.Keynote.Crumble",
    "com.apple.iWork.Keynote.FromDarkness",
    "com.apple.iWork.Keynote.KLNBCBlast",
    "com.apple.iWork.Keynote.KLNBCConvergence",
    "com.apple.iWork.Keynote.KLNBCImplodeExplode",
    "com.apple.iWork.Keynote.KLNBlur",
    "com.apple.iWork.Keynote.KLNBouncy",
    "com.apple.iWork.Keynote.KLNBulletFlare",
    "com.apple.iWork.Keynote.KLNColorPlanes",
    "com.apple.iWork.Keynote.KLNComet",
    "com.apple.iWork.Keynote.KLNConfetti",
    "com.apple.iWork.Keynote.KLNConfettiByCharacter",
    "com.apple.iWork.Keynote.KLNConfettiByWord",
    "com.apple.iWork.Keynote.KLNDiffuse",
    "com.apple.iWork.Keynote.KLNExplosion",
    "com.apple.iWork.Keynote.KLNFlame",
    "com.apple.iWork.Keynote.KLNShimmer",
    "com.apple.iWork.Keynote.KLNShockwave",
    "com.apple.iWork.Keynote.KLNSparkle",
    "com.apple.iWork.Keynote.KLNSquish",
    "com.apple.iWork.Keynote.KLNSwap",
    "com.apple.iWork.Keynote.KNBuildSkid",
    "com.apple.iWork.Keynote.KNBuildSkidByCharacter",
    "com.apple.iWork.Keynote.KNFireworks",
    "com.apple.iWork.Keynote.LineDraw",
    "com.apple.iWork.Keynote.LineDrawForDrawing",
    "com.apple.iWork.Keynote.LineDrawForLine",
    "com.apple.iWork.Keynote.Trace",
    "com.apple.iWork.Keynote.Vanish",
];

/// The ids the corpus holds and the parity path already plays, whose meaning is settled.
const SEEN: &[&str] = &[
    "apple:appear",
    "apple:bc-appear",
    "apple:dissolve",
    "apple:dissolve character",
    "apple:magic-move-implied-motion-path",
    "apple:movie-start",
];

/// The native id of a Keynote effect: `keynote:<id>`.
pub fn effect_id(id: &str) -> NativeId {
    NativeId::new(FileFormat::Keynote.into(), id)
}

const fn degrees(deg: i64) -> Deg {
    Deg::from_steps(deg * 1_000)
}

fn fly_in(dir: Dir) -> BuildEffect {
    BuildEffect::FlyIn(FlyInParams {
        dir,
        distance: Distance::OffSlide,
    })
}

fn zoom(fade: bool) -> BuildEffect {
    BuildEffect::Zoom(ZoomParams {
        in_out: InOut::In,
        origin: ZoomOrigin::Object,
        slight: false,
        fade,
        overshoot: false,
    })
}

fn path() -> BuildEffect {
    BuildEffect::Path(PathParams {
        d: vec![PathSeg::MoveTo(Px::ZERO, Px::ZERO)],
        auto_rotate: false,
        hold: true,
    })
}

/// A build row before its certainty is settled.
fn row(effect: BuildEffect, kind: StepKind) -> BuildRow {
    BuildRow::effect(effect, kind).inferred()
}

/// The row a build with effect `id` plays in a step of `kind` (`In` → entrance, `Out` → exit, an action →
/// emphasis or motion). `None` for an id that is not a build effect.
pub fn build(id: &str, kind: StepKind) -> Option<BuildRow> {
    let by_char = |row: BuildRow| row.delivered(Delivery::ByChar);
    let built = match id {
        "apple:appear" | "apple:bc-appear" | "apple:gallery-appear" => {
            row(BuildEffect::Appear, kind)
        }
        "apple:dissolve"
        | "apple:c-dissolve"
        | "apple:bc-dissolve"
        | "apple:bw-dissolve"
        | "apple:gallery-dissolve"
        | "apple:motion dissolve" => row(BuildEffect::Fade, kind),
        "apple:dissolve character" => by_char(row(BuildEffect::Fade, kind)),
        "apple:move in" | "apple:bc-move" | "apple:gallery-move" => {
            row(fly_in(Dir::Right), kind).refine()
        }
        "apple:move in character" => by_char(row(fly_in(Dir::Right), kind).refine()),
        "apple:fade and move"
        | "apple:fade-and-move"
        | "apple:gallery-fade-and-move"
        | "apple:drift"
        | "apple:drift object" => row(
            BuildEffect::FadeAndMove(DirParams { dir: Dir::Right }),
            kind,
        )
        .refine(),
        "apple:fade and move character" => by_char(
            row(
                BuildEffect::FadeAndMove(DirParams { dir: Dir::Right }),
                kind,
            )
            .refine(),
        ),
        "apple:drift and scale" => row(
            BuildEffect::DriftAndScale(DirParams { dir: Dir::Right }),
            kind,
        )
        .refine(),
        "apple:drift and scale character" => by_char(
            row(
                BuildEffect::DriftAndScale(DirParams { dir: Dir::Right }),
                kind,
            )
            .refine(),
        ),
        // Keynote's build movies play the wipe with a feathered edge, in the direction its number names. With no number
        // it plays the default `r` unreported, as every directional row does; no movie shows that case.
        "apple:wipe" => row(
            BuildEffect::Wipe(WipeParams {
                dir: Dir::Right,
                soft: true,
            }),
            kind,
        )
        .refine()
        .confirmed(),
        // Keynote's build movies play every iris as a circle, whatever shape the PPTX it imported asked for; its edge
        // is feathered, which the vocabulary's iris has no slot for.
        "apple:wipe-iris" => row(
            BuildEffect::Iris(IrisParams {
                shape: IrisShape::Circle,
                in_out: InOut::Out,
            }),
            kind,
        )
        .refine()
        .approximate()
        .confirmed(),
        "apple:wipe-radial" => row(
            BuildEffect::Clock(ClockParams {
                cw: true,
                from: ClockStart::Twelve,
            }),
            kind,
        )
        .refine(),
        // Keynote's build movies play the zoom fading as it scales, entrances and exits alike.
        "apple:zoom" => row(zoom(true), kind).refine().confirmed(),
        "apple:scale" | "apple:bc-scale-big" | "apple:bc-zoom" | "apple:bc-zoom-big"
        | "apple:bw-zoom-big" => row(zoom(false), kind),
        "apple:zoom character" | "apple:bc-zoom-big character" => by_char(row(zoom(false), kind)),
        "apple:pop" | "apple:bc-pop" | "apple:bw-pop" | "apple:ca-pop" | "apple:action-pop" => {
            row(BuildEffect::Pop, kind)
        }
        "apple:drop" | "apple:dropbuild" | "apple:dropbuild2" | "apple:bc-drop" | "apple:fall" => {
            row(BuildEffect::Drop, kind)
        }
        "apple:spin"
        | "apple:twirl"
        | "apple:twist-and-scale"
        | "apple:revolve"
        | "apple:ca-revolve" => row(
            BuildEffect::SpinIn(SpinInParams {
                deg: degrees(360),
                cw: true,
            }),
            kind,
        )
        .refine(),
        "apple:bc-typewriter" | "apple:keyboard" => row(BuildEffect::Typewriter, kind),
        "apple:compress" => row(
            BuildEffect::Compress(AxisParams {
                axis: Axis::Vertical,
            }),
            kind,
        ),
        "apple:bc-expand" => row(BuildEffect::Expand(AxisParams { axis: Axis::Both }), kind),
        "apple:pivot" | "apple:pivot-build" => row(
            BuildEffect::Swivel(AxisParams {
                axis: Axis::Horizontal,
            }),
            kind,
        )
        .approximate(),
        "apple:superflip" | "apple:bc-superflip" | "apple:bc-flip" | "apple:action-flip" => row(
            BuildEffect::Flip(AxisParams {
                axis: Axis::Horizontal,
            }),
            kind,
        ),
        "apple:drive-by" | "apple:drive-by object" => {
            row(fly_in(Dir::Right), kind).refine().approximate()
        }
        "apple:bounce" | "apple:action-bounce" | "com.apple.iWork.Keynote.KLNBouncy" => {
            row(BuildEffect::Bounce, kind)
        }
        "apple:action-motion-path"
        | "apple:emphasis-motion-path"
        | "apple:ca-implied-motion-path" => row(path(), StepKind::Motion).refine(),
        "apple:action-rotation" | "apple:emphasis-rotation" => row(
            BuildEffect::Spin(SpinParams {
                deg: degrees(360),
                cw: true,
                hold: true,
            }),
            kind,
        )
        .refine(),
        "apple:action-scale" | "apple:emphasis-scale" => row(
            BuildEffect::GrowShrink(GrowShrinkParams {
                scale: Scale {
                    x: Ratio::from_steps(150_000),
                    y: Ratio::from_steps(150_000),
                },
                hold: true,
            }),
            kind,
        )
        .refine(),
        "apple:action-opacity" | "apple:emphasis-opacity" => row(
            BuildEffect::Transparency(TransparencyParams {
                alpha: UnitRatio::from_steps(50_000),
                hold: true,
            }),
            kind,
        )
        .refine(),
        "apple:action-blink" => row(BuildEffect::Blink(RepeatParams { repeat: 1 }), kind).refine(),
        "apple:action-pulse" => row(
            BuildEffect::Pulse(PulseParams {
                scale: Ratio::from_steps(105_000),
            }),
            kind,
        ),
        "apple:action-jiggle" => row(
            BuildEffect::Jiggle(IntensityParams {
                intensity: UnitRatio::from_steps(50_000),
            }),
            kind,
        )
        .refine(),
        "apple:movie-start" | "apple:audio-start" => {
            row(BuildEffect::Play(PlayParams { from: Ms(0) }), kind)
        }
        "apple:movie-stop" | "apple:audio-stop" => row(BuildEffect::Stop, kind),
        "com.apple.iWork.Keynote.KLNBlur" | "com.apple.iWork.Keynote.Blur" => {
            row(BuildEffect::Fade, kind).approximate()
        }
        "com.apple.iWork.Keynote.KLNSquish" => row(
            BuildEffect::Stretch(AxisParams {
                axis: Axis::Vertical,
            }),
            kind,
        )
        .approximate(),
        "com.apple.iWork.Keynote.KNBuildSkid" => {
            row(fly_in(Dir::Right), kind).refine().approximate()
        }
        "com.apple.iWork.Keynote.KNBuildSkidByCharacter" => {
            by_char(row(fly_in(Dir::Right), kind).refine().approximate())
        }
        "com.apple.iWork.Keynote.BUKFlop" => row(
            BuildEffect::Flip(AxisParams {
                axis: Axis::Horizontal,
            }),
            kind,
        )
        .approximate(),
        "com.apple.iWork.Keynote.BUKTwist" => row(
            BuildEffect::SpinIn(SpinInParams {
                deg: degrees(360),
                cw: true,
            }),
            kind,
        )
        .approximate(),
        "apple:web-video-start"
        | "apple:magic chart"
        | "apple:burn"
        | "apple:fall-apart"
        | "apple:fallingtiles"
        | "apple:droplet"
        | "apple:flash"
        | "apple:bc-orbital"
        | "apple:bc-3"
        | "apple:ca-text-shimmer"
        | "apple:ca-text-sparkle"
        | "apple:3d-crane"
        | "apple:3d-rotate"
        | "apple:3d-rotatinggrow" => BuildRow::fallback(kind).inferred(),
        plugin if plugin.starts_with(PLUGIN) && IDS.contains(&plugin) => {
            BuildRow::fallback(kind).inferred()
        }
        _ => return None,
    };
    Some(if SEEN.contains(&id) {
        built.confirmed()
    } else {
        built
    })
}

/// The curve Keynote 14.5 plays a move in on when the transition stores no `custom_timing_curve`: fast from the
/// start, slowing into place. Fitted to the catalog movie's move in (31.0–32.5 s) over its 1.5 s, mean progress error
/// 0.0014 (CSS ease-in-out 0.017, ease-out 0.020).
const MOVE_IN_CURVE: Easing = Easing::Bezier(
    Ratio::from_steps(62_000),
    Ratio::from_steps(92_000),
    Ratio::from_steps(62_000),
    Ratio::from_steps(98_000),
);

/// The curve Keynote 14.5 plays a reveal on when the transition stores no `custom_timing_curve`: slow from the start,
/// leaving at speed. Fitted to the catalog movie's reveal (44.0–45.5 s) over its 1.5 s, mean progress error 0.0024
/// (CSS ease-in-out 0.015, ease-in 0.028).
const REVEAL_CURVE: Easing = Easing::Bezier(
    Ratio::from_steps(48_000),
    Ratio::from_steps(0),
    Ratio::from_steps(56_000),
    Ratio::from_steps(32_000),
);

fn transition_row(effect: TransitionEffect) -> TransitionRow {
    TransitionRow::effect(effect).inferred()
}

fn left() -> Dir4Params {
    Dir4Params { dir: Dir4::Left }
}

/// The row a transition with effect `id` plays. `None` for an id that is not a transition effect.
///
/// A row's own curve is the one Keynote's movie of the transition catalog plays the effect on, where the transition
/// stores no `custom_timing_curve`: the dissolve (11.5 s), the fade through black (18.0 s) and the
/// push (37.5 s) fit CSS ease-in-out within 0.005 of progress; the move in and the reveal do not, and carry their
/// fitted curves. The ids sharing an arm with a measured one take its curve; the other rows have none measured.
pub fn transition(id: &str) -> Option<TransitionRow> {
    use TransitionEffect as T;
    let row = match id {
        "apple:dissolve"
        | "apple:c-dissolve"
        | "apple:bc-dissolve"
        | "apple:gallery-dissolve"
        | "apple:motion dissolve" => transition_row(T::Fade(FadeParams {
            through: Through::None,
            color: None,
        }))
        .plays_on(Easing::InOut),
        "apple:fade" | "com.apple.iWork.Keynote.BLTFadeThruColor" => {
            transition_row(T::Fade(FadeParams {
                through: Through::Black,
                color: None,
            }))
            .refine()
            .plays_on(Easing::InOut)
        }
        "apple:flash" => transition_row(T::Fade(FadeParams {
            through: Through::White,
            color: None,
        })),
        "apple:push" | "apple:ca-push" => transition_row(T::Push(left()))
            .refine()
            .plays_on(Easing::InOut),
        // Keynote's move in slides the incoming slide over the outgoing one, which stays still: a cover, as its movie
        // plays it and its PPTX export writes it (`<p:cover dir="l"/>`).
        "apple:slide" => transition_row(T::Cover(DirParams { dir: Dir::Left }))
            .refine()
            .confirmed()
            .plays_on(MOVE_IN_CURVE),
        "apple:drive-by" | "apple:drive-by object" => {
            transition_row(T::Push(left())).refine().approximate()
        }
        "apple:move in" => transition_row(T::Cover(DirParams { dir: Dir::Left })).refine(),
        "apple:reveal" => transition_row(T::Uncover(DirParams { dir: Dir::Left }))
            .refine()
            .plays_on(REVEAL_CURVE),
        // Keynote's movie plays the wipe with a feathered edge moving right when no direction is stored, and its
        // export writes it `<p:wipe dir="r"/>`.
        "apple:wipe" => transition_row(T::Wipe(WipeParams {
            dir: Dir::Right,
            soft: true,
        }))
        .refine()
        .confirmed(),
        "apple:wipe-iris" => transition_row(T::Iris(IrisParams {
            shape: IrisShape::Circle,
            in_out: InOut::Out,
        }))
        .refine(),
        "apple:wipe-radial" => transition_row(T::Clock(ClockParams {
            cw: true,
            from: ClockStart::Twelve,
        }))
        .refine(),
        "apple:scale" | "apple:zoom" | "apple:ca-zoom" => {
            transition_row(T::Zoom(TransitionZoomParams {
                in_out: InOut::In,
                rotate: false,
            }))
            .refine()
        }
        "apple:sidezoom" => transition_row(T::Zoom(TransitionZoomParams {
            in_out: InOut::In,
            rotate: false,
        }))
        .approximate(),
        "apple:ca-cube" => transition_row(T::Cube(left())).refine(),
        "apple:3d-rotate" => transition_row(T::Rotate3d(left())).refine(),
        "apple:3d-rotatinggrow" | "apple:3d-crane" => {
            transition_row(T::Rotate3d(left())).refine().approximate()
        }
        "apple:doorway" => transition_row(T::Doors(OrientParams {
            orient: Orient::Vertical,
        })),
        "apple:ca-swing" | "com.apple.iWork.Keynote.BLTRevolvingDoor" => {
            transition_row(T::Doors(OrientParams {
                orient: Orient::Vertical,
            }))
            .approximate()
        }
        "apple:ca-isometric" | "apple:apple-grid" => {
            transition_row(T::Gallery(left())).refine().approximate()
        }
        "apple:pageflip" | "apple:superflip" => transition_row(T::PageFlip(left())).refine(),
        "apple:spin"
        | "apple:twirl"
        | "apple:twist-and-scale"
        | "apple:revolve"
        | "apple:ca-revolve"
        | "apple:ca-dissolve-and-flip"
        | "com.apple.iWork.Keynote.BUKFlop" => {
            transition_row(T::Flip(left())).refine().approximate()
        }
        "com.apple.iWork.Keynote.BUKTwist" => transition_row(T::Twist(left())).approximate(),
        "com.apple.iWork.Keynote.KLNSwap" => transition_row(T::Swap(left())).refine(),
        "com.apple.iWork.Keynote.BLTMosaicFlip" => transition_row(T::Mosaic(left())).approximate(),
        "com.apple.iWork.Keynote.BLTBlinds" => transition_row(T::Blinds(OrientParams {
            orient: Orient::Vertical,
        })),
        "apple:magic-move-implied-motion-path" | "apple:ca-implied-motion-path" => {
            transition_row(T::MagicMove(MagicMoveParams {
                fade_unmatched: true,
            }))
            .refine()
        }
        "apple:ca-anagram" => transition_row(T::Fade(FadeParams {
            through: Through::None,
            color: None,
        }))
        .approximate(),
        "apple:ca-text-shimmer"
        | "apple:ca-text-sparkle"
        | "apple:droplet"
        | "apple:fall"
        | "apple:fallingtiles"
        | "apple:pivot"
        | "apple:magic chart" => TransitionRow::fallback(TransitionFallback::Dissolve).inferred(),
        plugin if plugin.starts_with(PLUGIN) && IDS.contains(&plugin) => {
            TransitionRow::fallback(TransitionFallback::Dissolve).inferred()
        }
        _ => return None,
    };
    Some(if SEEN.contains(&id) {
        row.confirmed()
    } else {
        row
    })
}

/// What a Keynote direction number means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeynoteDirection {
    /// The direction the content moves.
    Motion(Dir),
    Rotation {
        cw: bool,
    },
    /// The scale direction as an entrance reads it; an exit reads it through [`KeynoteDirection::on_exit`].
    Zoom(InOut),
}

impl KeynoteDirection {
    /// The direction an exit's number names. Keynote's in and out name what the element does, for entrances and
    /// exits alike, so an exit's scale direction is the opposite ([`InOut::opposite`]): the vocabulary's `inOut`
    /// names the entrance an exit reverses. Keynote's build movies show it: an exit iris 41 (in) closes to the
    /// centre and 42 (out) opens a hole from it, an exit zoom 43 (in) grows and 44 (out) shrinks, each the reverse
    /// of the entrance of the other number. A motion or rotation reads the same on an exit.
    pub fn on_exit(self) -> KeynoteDirection {
        match self {
            KeynoteDirection::Zoom(in_out) => KeynoteDirection::Zoom(in_out.opposite()),
            other => other,
        }
    }
}

/// The kind of effect a direction steers, which decides how sure a scale number's reading is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Steered {
    Motion,
    Rotation,
    Iris,
    Zoom,
}

/// The effects Keynote's own files pin a direction number's reading on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pinned {
    Everywhere,
    OnIris,
    OnZoom,
}

/// What a direction number means (`AnimationAttributesArchive.direction`, else `database_direction`), and where
/// Keynote's own files pin it. `None` for 0 (no direction) and for a number Keynote's files never show (the caller
/// keeps the row's default and reports it). The schema types no enum for the field; the attended catalog
/// (`probes/keynote/RUNBOOK.md`) pins the numbers:
/// - Keynote's import of `probes/out/motion-probe.pptx` stores, for each OOXML direction token, its own number: a
///   push, cover or pull `l u r d` becomes 12 14 11 13, a cover or pull `rd ld ru lu` 21 22 23 24, a fly-in, wipe
///   and peek the same; an entrance iris `in`/`out` (box, circle, diamond, plus) 41/42 and a circle, diamond or plus
///   transition 42; an entrance zoom `in`/`out` 43/44, a `warp` in/out 43/44;
/// - Keynote's movie of its transition catalog plays a push, move-in and reveal with no number moving the content
///   left, and its PPTX export writes them `dir="l"`; it plays the iris opening from the centre and writes it
///   `circle`, which the import reads back as 42.
///
/// So Keynote numbers the direction the content moves, the vocabulary's convention. The rotation
/// pair 31/32 is the schema's `BuildAttributesRotationDirection`. Two readings stay inferred: 41/42 on a zoom or
/// scale (Keynote's own zoom pair is 43/44; it gives 41/42 only to PowerPoint's fly-through, whose in/out name the
/// opposite scale), and 43/44 on an iris.
pub fn direction(number: u32) -> Option<(KeynoteDirection, Pinned)> {
    use KeynoteDirection::{Motion, Rotation, Zoom};
    let motion = |dir| Some((Motion(dir), Pinned::Everywhere));
    match number {
        11 => motion(Dir::Right),
        12 => motion(Dir::Left),
        13 => motion(Dir::Down),
        14 => motion(Dir::Up),
        21 => motion(Dir::RightDown),
        22 => motion(Dir::LeftDown),
        23 => motion(Dir::RightUp),
        24 => motion(Dir::LeftUp),
        31 => Some((Rotation { cw: true }, Pinned::Everywhere)),
        32 => Some((Rotation { cw: false }, Pinned::Everywhere)),
        41 => Some((Zoom(InOut::In), Pinned::OnIris)),
        42 => Some((Zoom(InOut::Out), Pinned::OnIris)),
        43 => Some((Zoom(InOut::In), Pinned::OnZoom)),
        44 => Some((Zoom(InOut::Out), Pinned::OnZoom)),
        _ => None,
    }
}

/// How sure the reading of `number` is on an effect of kind `steered`: confirmed where Keynote's files pin it there,
/// inferred elsewhere and for a number the table does not know.
pub fn direction_certainty(number: u32, steered: Steered) -> Certainty {
    match direction(number) {
        Some((_, Pinned::Everywhere)) => Certainty::Confirmed,
        Some((_, Pinned::OnIris)) if steered == Steered::Iris => Certainty::Confirmed,
        Some((_, Pinned::OnZoom)) if steered == Steered::Zoom => Certainty::Confirmed,
        _ => Certainty::Inferred,
    }
}

/// The direction numbers [`direction`] knows.
pub const DIRECTIONS: [u32; 14] = [11, 12, 13, 14, 21, 22, 23, 24, 31, 32, 41, 42, 43, 44];
