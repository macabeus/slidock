//! Keynote's effects in the common vocabulary: the effect id
//! picks a row of `slidock-presets`' Keynote table, which also reads the direction numbers and holds everything
//! Keynote's own files show (this module adds no reading of its own), and the attribute archives replace its defaults wherever they
//! state a value (the direction number, an action's angle, scale, opacity and repeats, the colour of a fade through
//! a colour, Magic Move's options) and give the easing. A value the tables do not know keeps the row's default and
//! is reported, never coerced; a parameter the vocabulary has no slot for is reported as an approximation.
//!
//! A build's native effect plays in the kind its animation type names strictly ([`native_kind`]): an unknown type,
//! or one the effect does not belong to, plays the fallback.
//!
//! The IR's `id` and `fallback` are the parity path's: [`step_fallback`] and [`transition_fallback`].

use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::effects::{
    BuildEffect, Delivery, Dir, Easing, FadeParams, GrowShrinkParams, IntensityParams,
    MagicMoveParams, NativeStep, NativeTransition, Repeat, RepeatParams, Scale, SpinParams,
    StepKind, Through, TransitionEffect, TransparencyParams, limits,
};
use slidock_ir::timeline::{StepFallback, TransitionFallback};
use slidock_ir::units::{Deg, Ratio, UnitRatio};
use slidock_ir::validate::validate;
use slidock_presets::Certainty;
use slidock_presets::keynote::{self as table, KeynoteDirection, Steered};

use super::motion_path;
use crate::context::Lowering;
use crate::num::wide;
use crate::proto::{kn, tsd, tsp};

/// Magic Move's effect ids.
pub const MAGIC_MOVE: [&str; 2] = [
    "apple:magic-move-implied-motion-path",
    "apple:ca-implied-motion-path",
];

/// The IR's fallback for a build effect; `None` is reported (KN1402) and dissolves.
pub fn step_fallback(effect: &str) -> Option<StepFallback> {
    match effect {
        "apple:appear" => Some(StepFallback::Appear),
        "apple:dissolve" | "apple:dissolve character" => Some(StepFallback::Dissolve),
        "apple:movie-start" => Some(StepFallback::None),
        _ => None,
    }
}

/// The IR's fallback for a transition effect; `None` is reported (KN1403) and dissolves.
pub fn transition_fallback(effect: &str) -> Option<TransitionFallback> {
    match effect {
        "apple:dissolve" | "apple:magic-move-implied-motion-path" => {
            Some(TransitionFallback::Dissolve)
        }
        "apple:push" => Some(TransitionFallback::Push),
        "apple:wipe" => Some(TransitionFallback::Wipe),
        _ => None,
    }
}

/// `TransitionAttributesArchive.TransitionCustomAttributesTimingCurveType`.
mod curve {
    pub const LINEAR: i32 = 1;
    pub const EASE_IN: i32 = 2;
    pub const EASE_OUT: i32 = 3;
    pub const EASE_IN_OUT: i32 = 4;
    pub const CUSTOM: i32 = 5;
}

/// `BuildAttributesArchive.BuildAttributesAcceleration`.
mod acceleration {
    pub const NONE: i32 = 0;
    pub const EASE_IN: i32 = 1;
    pub const EASE_OUT: i32 = 2;
    pub const EASE_BOTH: i32 = 3;
    pub const CUSTOM: i32 = 4;
}

/// `TransitionCustomAttributesTextDeliveryType`: Magic Move's match granularity.
const MATCH_BY_WORD: i32 = 2;
const MATCH_BY_CHARACTER: i32 = 3;

/// `BuildAttributesRotationDirection`.
const CLOCKWISE: i32 = 31;
const COUNTERCLOCKWISE: i32 = 32;

/// `BuildAttributesDeliveryOption`: undefined and forward play forward.
const DELIVERY_FORWARD: [i32; 2] = [0, 1];
const DELIVERY_RANDOM: i32 = 5;

/// `BuildAttributesCurveStyle`.
const CURVE_STRAIGHT: i32 = 0;
const CURVE_CURVED: i32 = 1;

/// The longest animation type a diagnostic quotes, in characters.
const QUOTED: usize = 64;

/// `ActionBuildAttributesJiggleIntensity` (small, medium, large) as the vocabulary's intensity.
const JIGGLE_INTENSITY: [(i32, i64); 3] = [(0, 25_000), (1, 50_000), (2, 100_000)];

/// What mapping one effect found to report.
#[derive(Debug, Default)]
pub struct Findings {
    approximations: Vec<String>,
    unknown_direction: Option<i64>,
    easing: Option<&'static str>,
}

impl Findings {
    pub fn approximate(&mut self, reason: impl Into<String>) {
        self.approximations.push(reason.into());
    }

    /// KN1404 (approximations, joined), KN1405 (an unknown direction) and KN1408 (an approximated curve).
    pub fn report(self, ctx: &mut Lowering<'_>, effect: &str) {
        if !self.approximations.is_empty() {
            ctx.report_motion(
                codes::KN1404,
                DiagnosticSite::new()
                    .data("effect", effect.to_owned())
                    .data("reason", self.approximations.join("; ")),
            );
        }
        if let Some(direction) = self.unknown_direction {
            ctx.report_motion(
                codes::KN1405,
                DiagnosticSite::new()
                    .data("direction", direction as f64)
                    .data("effect", effect.to_owned()),
            );
        }
        if let Some(easing) = self.easing {
            ctx.report_motion(
                codes::KN1408,
                DiagnosticSite::new()
                    .data("effect", effect.to_owned())
                    .data("easing", easing),
            );
        }
    }
}

/// The direction number an effect states: the animation attributes', else the legacy `database_direction`; 0 is
/// none.
fn direction_number(
    animation: Option<&kn::AnimationAttributesArchive>,
    legacy: Option<u32>,
) -> Option<u32> {
    animation
        .and_then(|animation| animation.direction)
        .or(legacy)
        .filter(|number| *number != 0)
}

/// A direction mirrored left to right, as a right-to-left writing direction reads it.
const fn mirrored(dir: Dir) -> Dir {
    match dir {
        Dir::Left => Dir::Right,
        Dir::Right => Dir::Left,
        Dir::LeftUp => Dir::RightUp,
        Dir::RightUp => Dir::LeftUp,
        Dir::LeftDown => Dir::RightDown,
        Dir::RightDown => Dir::LeftDown,
        Dir::Up | Dir::Down => dir,
    }
}

/// Reads a direction number onto an effect through `steer`, which says what kind of direction the effect took, or
/// `None` when it takes none of that kind. No number keeps the row's default, unreported; an unknown number, or one
/// the effect does not take, keeps it too (KN1405). A number Keynote's own files pin plays silently; any other reading, and a right-to-left mirror, is
/// reported as an approximation (KN1404). Keynote names the motion, as the vocabulary does.
fn read_direction<E>(
    effect: &mut E,
    animation: Option<&kn::AnimationAttributesArchive>,
    legacy: Option<u32>,
    findings: &mut Findings,
    steer: impl Fn(&mut E, KeynoteDirection, &mut Findings) -> Option<Steered>,
) {
    let Some(number) = direction_number(animation, legacy) else {
        return;
    };
    let Some((mut direction, _)) = table::direction(number) else {
        findings.unknown_direction = Some(i64::from(number));
        return;
    };
    let rtl = animation.and_then(|a| a.writing_direction_is_rtl) == Some(true);
    if let (true, KeynoteDirection::Motion(dir)) = (rtl, direction) {
        direction = KeynoteDirection::Motion(mirrored(dir));
        findings.approximate(format!(
            "direction {number} is mirrored for right-to-left text by inference"
        ));
    }
    let Some(steered) = steer(effect, direction, findings) else {
        findings.unknown_direction = Some(i64::from(number));
        return;
    };
    if table::direction_certainty(number, steered) == Certainty::Inferred {
        findings.approximate(format!("direction {number} is read by inference"));
    }
}

/// Steers a build; an exit reads its number as the table says an exit's does ([`KeynoteDirection::on_exit`]).
fn steer_build(
    effect: &mut BuildEffect,
    direction: KeynoteDirection,
    exit: bool,
) -> Option<Steered> {
    use KeynoteDirection::{Motion, Rotation, Zoom};
    let direction = if exit { direction.on_exit() } else { direction };
    Some(match (effect, direction) {
        (BuildEffect::FlyIn(p), Motion(dir)) => {
            p.dir = dir;
            Steered::Motion
        }
        (BuildEffect::FadeAndMove(p) | BuildEffect::DriftAndScale(p), Motion(dir)) => {
            p.dir = dir;
            Steered::Motion
        }
        (BuildEffect::Wipe(p), Motion(dir)) => {
            p.dir = dir;
            Steered::Motion
        }
        (BuildEffect::SpinIn(p), Rotation { cw }) => {
            p.cw = cw;
            Steered::Rotation
        }
        (BuildEffect::Spin(p), Rotation { cw }) => {
            p.cw = cw;
            Steered::Rotation
        }
        (BuildEffect::Clock(p), Rotation { cw }) => {
            p.cw = cw;
            Steered::Rotation
        }
        (BuildEffect::Iris(p), Zoom(in_out)) => {
            p.in_out = in_out;
            Steered::Iris
        }
        (BuildEffect::Zoom(p), Zoom(in_out)) => {
            p.in_out = in_out;
            Steered::Zoom
        }
        _ => return None,
    })
}

fn steer_transition(
    effect: &mut TransitionEffect,
    direction: KeynoteDirection,
    _: &mut Findings,
) -> Option<Steered> {
    use KeynoteDirection::{Motion, Rotation, Zoom};
    let straight = |dir: Dir| dir.as_dir4();
    Some(match (effect, direction) {
        (
            TransitionEffect::Push(p)
            | TransitionEffect::Cube(p)
            | TransitionEffect::Flip(p)
            | TransitionEffect::PageFlip(p)
            | TransitionEffect::Gallery(p)
            | TransitionEffect::Rotate3d(p)
            | TransitionEffect::Swap(p)
            | TransitionEffect::Twist(p)
            | TransitionEffect::Mosaic(p),
            Motion(dir),
        ) => {
            p.dir = straight(dir)?;
            Steered::Motion
        }
        (TransitionEffect::Cover(p) | TransitionEffect::Uncover(p), Motion(dir)) => {
            p.dir = dir;
            Steered::Motion
        }
        (TransitionEffect::Wipe(p), Motion(dir)) => {
            p.dir = dir;
            Steered::Motion
        }
        (TransitionEffect::Iris(p), Zoom(in_out)) => {
            p.in_out = in_out;
            Steered::Iris
        }
        (TransitionEffect::Zoom(p), Zoom(in_out)) => {
            p.in_out = in_out;
            Steered::Zoom
        }
        (TransitionEffect::Clock(p), Rotation { cw }) => {
            p.cw = cw;
            Steered::Rotation
        }
        _ => return None,
    })
}

/// A curve stored as one cubic from (0, 0) to its end, scaled so the end is (1, 1): CSS `cubic-bezier()`.
fn bezier(source: Option<&tsd::PathSourceArchive>) -> Option<Easing> {
    let path = source?.bezier_path_source.as_ref()?.path.as_ref()?;
    let [start, end] = path.elements.as_slice() else {
        return None;
    };
    let point = |p: &tsp::Point| Some((wide(p.x)?, wide(p.y)?));
    let [origin] = start.points.as_slice() else {
        return None;
    };
    let [c1, c2, last] = end.points.as_slice() else {
        return None;
    };
    if start.r#type != Some(motion_path::MOVE_TO) || end.r#type != Some(motion_path::CURVE_TO) {
        return None;
    }
    let (origin, c1, c2, last) = (point(origin)?, point(c1)?, point(c2)?, point(last)?);
    let (w, h) = (last.0 - origin.0, last.1 - origin.1);
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h != 0.0) {
        return None;
    }
    let x = |p: (f64, f64)| (p.0 - origin.0) / w;
    let y = |p: (f64, f64)| (p.1 - origin.1) / h;
    let (x1, y1, x2, y2) = (x(c1), y(c1), x(c2), y(c2));
    let in_x = |v: f64| (0.0..=1.0).contains(&v);
    let in_y = |v: f64| (-limits::BEZIER_Y..=limits::BEZIER_Y).contains(&v);
    if !(in_x(x1) && in_x(x2) && in_y(y1) && in_y(y2)) {
        return None;
    }
    Some(Easing::Bezier(
        Ratio::quantise(x1).ok()?,
        Ratio::quantise(y1).ok()?,
        Ratio::quantise(x2).ok()?,
        Ratio::quantise(y2).ok()?,
    ))
}

/// A custom curve as a bezier, else ease-in-out (KN1408). The step plays one curve: the first; a second or third
/// curve that differs from it is reported (KN1408). The curves' theme names label them in Keynote's editor and
/// change nothing that plays.
fn custom_curve(
    animation: Option<&kn::AnimationAttributesArchive>,
    findings: &mut Findings,
) -> Option<Easing> {
    let first = animation.and_then(|a| a.custom_effect_timing_curve_1.as_ref());
    match bezier(first) {
        Some(easing) => {
            let others = animation.into_iter().flat_map(|a| {
                [
                    a.custom_effect_timing_curve_2.as_ref(),
                    a.custom_effect_timing_curve_3.as_ref(),
                ]
            });
            if others.flatten().any(|curve| Some(curve) != first) {
                findings.easing = Some("its first curve");
            }
            Some(easing)
        }
        None => {
            findings.easing = Some("in-out");
            Some(Easing::InOut)
        }
    }
}

/// A transition's `custom_timing_curve`; linear (KN1408) when unknown. Unset, it is the effect's own curve, `own`
/// (the table's, where Keynote's movie measured it; else none, the player's default for the recipe).
fn timing_curve(
    value: Option<i32>,
    own: Option<Easing>,
    animation: Option<&kn::AnimationAttributesArchive>,
    findings: &mut Findings,
) -> Option<Easing> {
    let Some(value) = value else {
        return own;
    };
    match value {
        curve::LINEAR => Some(Easing::Linear),
        curve::EASE_IN => Some(Easing::In),
        curve::EASE_OUT => Some(Easing::Out),
        curve::EASE_IN_OUT => Some(Easing::InOut),
        curve::CUSTOM => custom_curve(animation, findings),
        _ => {
            findings.easing = Some("linear");
            Some(Easing::Linear)
        }
    }
}

/// A build's `action_acceleration`; none (linear) when unset or none, linear (KN1408) when unknown.
fn build_easing(
    value: Option<i32>,
    animation: Option<&kn::AnimationAttributesArchive>,
    findings: &mut Findings,
) -> Option<Easing> {
    match value? {
        acceleration::NONE => None,
        acceleration::EASE_IN => Some(Easing::In),
        acceleration::EASE_OUT => Some(Easing::Out),
        acceleration::EASE_BOTH => Some(Easing::InOut),
        acceleration::CUSTOM => custom_curve(animation, findings),
        _ => {
            findings.easing = Some("linear");
            None
        }
    }
}

/// A finite value inside `range`, else `None` with an approximation naming `what`.
fn bounded(
    value: Option<f64>,
    range: std::ops::RangeInclusive<f64>,
    what: &str,
    findings: &mut Findings,
) -> Option<f64> {
    let value = value?;
    if value.is_finite() && range.contains(&value) {
        Some(value)
    } else {
        findings.approximate(format!(
            "the {what} {} is out of range; the default plays",
            slidock_canon::to_js_string(value)
        ));
        None
    }
}

/// A repeat count within the vocabulary's bound; 0 is unset, more than the bound is clamped.
fn repeat_count(value: Option<u32>, findings: &mut Findings) -> Option<u32> {
    let value = value.filter(|value| *value > 0)?;
    if value > limits::REPEAT_COUNT {
        findings.approximate(format!(
            "the repeat count {value} is clamped to {}",
            limits::REPEAT_COUNT
        ));
        return Some(limits::REPEAT_COUNT);
    }
    Some(value)
}

/// An action's own values onto its effect: a rotation's angle and direction, a scale, an opacity, a jiggle's
/// intensity, a blink's repeats, a zoom's bounce.
fn read_action(
    effect: &mut BuildEffect,
    attributes: &kn::BuildAttributesArchive,
    findings: &mut Findings,
) {
    match effect {
        BuildEffect::Spin(SpinParams { deg, cw, .. }) => {
            let angle = bounded(
                attributes.action_rotation_angle,
                0.0..=limits::SPIN_DEGREES,
                "rotation angle",
                findings,
            );
            if let Some(angle) = angle.and_then(|angle| Deg::quantise(angle).ok()) {
                *deg = angle;
            }
            match attributes.action_rotation_direction {
                None => {}
                Some(CLOCKWISE) => *cw = true,
                Some(COUNTERCLOCKWISE) => *cw = false,
                Some(other) => findings.unknown_direction = Some(i64::from(other)),
            }
        }
        BuildEffect::GrowShrink(GrowShrinkParams { scale, .. }) => {
            let size = bounded(
                attributes.action_scale_size,
                0.0..=limits::SCALE,
                "scale",
                findings,
            );
            match size.map(|size| (size, Ratio::quantise(size))) {
                None => {}
                Some((_, Ok(ratio))) if ratio.steps() > 0 => *scale = Scale { x: ratio, y: ratio },
                Some((size, _)) => findings.approximate(format!(
                    "the scale {} is out of range; the default plays",
                    slidock_canon::to_js_string(size)
                )),
            }
        }
        BuildEffect::Transparency(TransparencyParams { alpha, .. }) => {
            let value = bounded(
                attributes.action_color_alpha,
                0.0..=1.0,
                "opacity",
                findings,
            )
            .and_then(|value| UnitRatio::quantise(value).ok());
            if let Some(value) = value {
                *alpha = value;
            }
        }
        BuildEffect::Jiggle(IntensityParams { intensity }) => {
            if let Some(value) = attributes.custom_action_jiggle_intensity {
                match JIGGLE_INTENSITY.iter().find(|(raw, _)| *raw == value) {
                    Some((_, steps)) => *intensity = UnitRatio::from_steps(*steps),
                    None => findings.approximate(format!(
                        "the jiggle intensity {value} is unknown; the default plays"
                    )),
                }
            }
        }
        BuildEffect::Blink(RepeatParams { repeat }) => {
            if let Some(count) = repeat_count(attributes.custom_action_repeat_count, findings) {
                *repeat = count;
            }
        }
        BuildEffect::Zoom(zoom) if attributes.custom_bounce == Some(true) => zoom.overshoot = true,
        _ => {}
    }
}

/// The build options the vocabulary has no slot for, by name, when the build sets them.
fn unplayed_build_options(
    effect: &BuildEffect,
    attributes: &kn::BuildAttributesArchive,
) -> Vec<&'static str> {
    let set = |value: Option<bool>| value == Some(true);
    let nonzero = |value: Option<f64>| value.is_some_and(|value| value != 0.0);
    let mut out = Vec::new();
    if set(attributes.custom_bounce) && !matches!(effect, BuildEffect::Zoom(_)) {
        out.push("bounce");
    }
    if nonzero(attributes.custom_travel_distance) {
        out.push("travel distance");
    }
    if set(attributes.custom_shine) {
        out.push("shine");
    }
    if set(attributes.custom_cursor) {
        out.push("cursor");
    }
    if set(attributes.custom_include_endpoints) {
        out.push("endpoints");
    }
    if nonzero(attributes.custom_scale_amount) {
        out.push("scale amount");
    }
    if set(attributes.custom_action_decay) {
        out.push("decay");
    }
    if set(attributes.custom_align_to_path) {
        out.push("align to path");
    }
    if nonzero(
        attributes
            .animation_attributes
            .as_ref()
            .and_then(|a| a.custom_detail),
    ) {
        out.push("detail");
    }
    if nonzero(attributes.custom_action_scale) {
        out.push("action scale");
    }
    if attributes.start_offset.is_some_and(|offset| offset != 0)
        || attributes.end_offset.is_some_and(|offset| offset != 0)
    {
        out.push("start and end offsets");
    }
    out
}

/// What a build plays besides its IR fields.
#[derive(Debug, Default)]
pub struct NativeBuild {
    pub native: Option<Box<NativeStep>>,
    pub easing: Option<Easing>,
    pub repeat: Option<Repeat>,
}

/// The emphases that repeat by the step's timing rather than a parameter of their own.
fn repeats_by_timing(effect: &BuildEffect) -> bool {
    matches!(
        effect,
        BuildEffect::Pulse(_)
            | BuildEffect::Pop
            | BuildEffect::Bounce
            | BuildEffect::Flip(_)
            | BuildEffect::Jiggle(_)
    ) || matches!(effect, BuildEffect::Behaviors(_))
}

/// Whether an effect id is an action build's: Keynote's action and emphasis ids and the motion paths.
fn is_action(effect: &str) -> bool {
    effect.starts_with("apple:action-")
        || effect.starts_with("apple:emphasis-")
        || effect == "apple:ca-implied-motion-path"
}

/// At most [`QUOTED`] characters of an untrusted name, for a diagnostic.
fn quoted(name: &str) -> String {
    name.chars().take(QUOTED).collect()
}

/// The kind a build's native effect plays in, from its animation type read strictly (the IR's kind reads any type but
/// `In` and `Out` as an emphasis): `In` enters and `Out` exits with an entrance or exit effect, `Action` emphasises
/// with an action effect, and a media command (the IR's media kind) plays whatever its type. An unknown type, a missing
/// one, or a type the effect does not belong to has no native kind: the reason is returned and the step plays its
/// fallback.
pub fn native_kind(
    animation_type: Option<&str>,
    effect: &str,
    kind: StepKind,
) -> Result<StepKind, String> {
    if kind == StepKind::Media {
        return Ok(kind);
    }
    let Some(named) = animation_type else {
        return Err("the build states no animation type; the fallback plays".to_owned());
    };
    match (named.to_lowercase().as_str(), is_action(effect)) {
        ("in", false) => Ok(StepKind::Entrance),
        ("out", false) => Ok(StepKind::Exit),
        ("action", true) => Ok(StepKind::Emphasis),
        ("in" | "out" | "action", _) => Err(format!(
            "an {} build does not play it; the fallback plays",
            quoted(named)
        )),
        _ => Err(format!(
            "the animation type {} is unknown; the fallback plays",
            quoted(named)
        )),
    }
}

/// A motion path's `curveStyle`: straight plays as stored; curved plays the stored curves, so a curved path whose
/// stored segments are all straight is an approximation; an unknown style is reported and the stored path plays.
fn read_curve_style(attributes: &kn::BuildAttributesArchive, findings: &mut Findings) {
    match attributes.curve_style {
        None | Some(CURVE_STRAIGHT) => {}
        Some(CURVE_CURVED) => {
            if !motion_path::has_curves(attributes) {
                findings.approximate(
                    "the curved path's stored segments are straight and play straight",
                );
            }
        }
        Some(other) => findings.approximate(format!(
            "the path style {other} is unknown; the stored path plays"
        )),
    }
}

/// A build step's native effect and timing: the preset row for `effect` played in the kind its animation type
/// names ([`native_kind`]; the IR's kind is `kind`), refined by the build's attributes. A row with no entry, or an
/// entry that kind cannot play, leaves the step to its fallback, as does a build stored only in the legacy
/// `database_*` fields. Findings go to `findings`, which the caller reports against the effect id; a delivery order
/// the vocabulary does not play is reported here, against the build (KN1406).
#[allow(
    clippy::too_many_arguments,
    reason = "one build's inputs, each its own"
)]
pub fn native_build(
    ctx: &mut Lowering<'_>,
    findings: &mut Findings,
    effect: &str,
    kind: StepKind,
    build: &kn::BuildArchive,
    build_id: u64,
    duration: slidock_ir::units::Ms,
    delivery: Delivery,
) -> NativeBuild {
    let mut out = NativeBuild::default();
    let empty = kn::BuildAttributesArchive::default();
    let attributes = build.attributes.as_ref().unwrap_or(&empty);
    let animation = attributes.animation_attributes.as_ref();
    if delivery != Delivery::All
        && let Some(order) = attributes.custom_delivery_option
        && !DELIVERY_FORWARD.contains(&order)
    {
        let seed = animation.and_then(|animation| animation.random_number_seed);
        ctx.report_motion(
            codes::KN1406,
            DiagnosticSite::new()
                .data("order", delivery_order_name(order, seed))
                .data("build", build_id.to_string()),
        );
    }
    if animation.is_none() {
        if let Some(legacy) = attributes.database_effect.as_deref() {
            findings.approximate(format!(
                "the build is stored in the legacy fields ({}), which are not read; the fallback plays",
                quoted(legacy)
            ));
        }
        return out;
    }
    if table::build(effect, kind).is_none() {
        return out;
    }
    let kind = match native_kind(
        animation.and_then(|a| a.animation_type.as_deref()),
        effect,
        kind,
    ) {
        Ok(kind) => kind,
        Err(reason) => {
            findings.approximate(reason);
            return out;
        }
    };
    let Some(row) = table::build(effect, kind) else {
        return out;
    };
    let Some(mut entry) = row.build_effect().cloned() else {
        return out;
    };
    let exit = kind == StepKind::Exit;
    read_direction(
        &mut entry,
        animation,
        attributes.database_direction,
        findings,
        |effect, direction, _| steer_build(effect, direction, exit),
    );
    read_action(&mut entry, attributes, findings);
    if row.approximate {
        findings.approximate(format!("{} approximates it", entry.name()));
    }
    for option in unplayed_build_options(&entry, attributes) {
        findings.approximate(format!("the {option} option is not played"));
    }
    if matches!(entry, BuildEffect::Path(_)) {
        entry = match motion_path::keyframes(attributes, duration) {
            Ok(behaviors) => {
                read_curve_style(attributes, findings);
                behaviors
            }
            Err(reason) => {
                findings.approximate(reason);
                entry
            }
        };
    }
    if entry.plays_in(kind) {
        out.easing = build_easing(attributes.action_acceleration, animation, findings);
        if repeats_by_timing(&entry)
            && let Some(count) = repeat_count(attributes.custom_action_repeat_count, findings)
            && count > 1
            && let Ok(count) = Ratio::quantise(f64::from(count))
        {
            out.repeat = Some(Repeat::Count(count));
        }
        let native = NativeStep {
            id: table::effect_id(effect),
            effect: entry,
        };
        if validate(&native).is_empty() {
            out.native = Some(Box::new(native));
        } else {
            findings.approximate("its parameters are out of range; the fallback plays");
        }
    }
    out
}

/// A delivery order by name; a random order names the seed Keynote shuffles with.
fn delivery_order_name(order: i32, seed: Option<u32>) -> String {
    match (order, seed) {
        (2, _) => "backward".to_owned(),
        (3, _) => "from center".to_owned(),
        (4, _) => "from edges".to_owned(),
        (DELIVERY_RANDOM, Some(seed)) => format!("random (seed {seed})"),
        (DELIVERY_RANDOM, None) => "random".to_owned(),
        (other, _) => format!("unknown ({other})"),
    }
}

/// A colour's components as `#rrggbb` (the alpha dropped; P3 components read as sRGB).
fn hex(color: &tsp::Color) -> Option<slidock_ir::effects::HexColor> {
    let rgb = crate::paint::color(Some(color))?;
    let byte = |c: UnitRatio| {
        let scaled = slidock_canon::js_round(c.to_f64() * 255.0);
        // A unit ratio scales into [0, 255].
        scaled.clamp(0.0, 255.0) as u8
    };
    Some(slidock_ir::effects::HexColor::rgb(
        byte(rgb.r),
        byte(rgb.g),
        byte(rgb.b),
    ))
}

/// A fade through a colour, by the colour the transition names: black and white are named, any other is `color`.
fn fade_through(color: Option<&tsp::Color>) -> Option<FadeParams> {
    use slidock_ir::effects::HexColor;
    let color = hex(color?)?;
    Some(match color {
        HexColor::BLACK => FadeParams {
            through: Through::Black,
            color: None,
        },
        HexColor::WHITE => FadeParams {
            through: Through::White,
            color: None,
        },
        other => FadeParams {
            through: Through::Color,
            color: Some(other),
        },
    })
}

/// The transition options the vocabulary has no slot for, by name, when the transition sets them.
fn unplayed_transition_options(
    attributes: &kn::TransitionAttributesArchive,
    animation: Option<&kn::AnimationAttributesArchive>,
) -> Vec<&'static str> {
    let mut out = Vec::new();
    if attributes.custom_bounce == Some(true) {
        out.push("bounce");
    }
    if attributes.custom_twist.is_some_and(|twist| twist != 0.0) {
        out.push("twist");
    }
    if attributes.custom_mosaic_size.is_some() || attributes.custom_mosaic_type.is_some() {
        out.push("mosaic");
    }
    if attributes
        .custom_travel_distance
        .is_some_and(|distance| distance != 0.0)
    {
        out.push("travel distance");
    }
    if animation
        .and_then(|a| a.custom_detail)
        .is_some_and(|detail| detail != 0.0)
    {
        out.push("detail");
    }
    out
}

/// What a transition plays besides its IR fields.
#[derive(Debug, Default)]
pub struct NativeTransitionOut {
    pub native: Option<Box<NativeTransition>>,
    pub easing: Option<Easing>,
}

/// A transition's native effect and easing: the preset row for `effect`, refined by the transition's attributes
/// (direction, colour, Magic Move's options).
pub fn native_transition(
    ctx: &mut Lowering<'_>,
    effect: &str,
    attributes: &kn::TransitionAttributesArchive,
) -> NativeTransitionOut {
    let mut findings = Findings::default();
    let mut out = NativeTransitionOut::default();
    let animation = attributes.animation_attributes.as_ref();
    if let Some(row) = table::transition(effect)
        && let Some(mut entry) = row.effect
    {
        read_direction(
            &mut entry,
            animation,
            attributes.database_direction,
            &mut findings,
            steer_transition,
        );
        match &mut entry {
            // Only a fade through a colour reads one: a dissolve is a plain fade whatever colour it stores.
            TransitionEffect::Fade(fade) if row.refine => {
                let color = animation
                    .and_then(|a| a.color.as_ref())
                    .or(attributes.database_color.as_ref());
                if let Some(through) = fade_through(color) {
                    *fade = through;
                }
            }
            TransitionEffect::MagicMove(MagicMoveParams { fade_unmatched }) => {
                if let Some(fade) = attributes.custom_magic_move_fade_unmatched_objects {
                    *fade_unmatched = fade;
                }
                match attributes.custom_text_delivery_type {
                    Some(MATCH_BY_WORD) => {
                        findings.approximate("text matched by word morphs as whole objects")
                    }
                    Some(MATCH_BY_CHARACTER) => {
                        findings.approximate("text matched by character morphs as whole objects")
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        if row.approximate {
            findings.approximate(format!("{} approximates it", entry.name()));
        }
        for option in unplayed_transition_options(attributes, animation) {
            findings.approximate(format!("the {option} option is not played"));
        }
        out.easing = timing_curve(
            attributes.custom_timing_curve,
            row.own_easing,
            animation,
            &mut findings,
        );
        let native = NativeTransition {
            id: table::effect_id(effect),
            effect: entry,
        };
        if validate(&native).is_empty() {
            out.native = Some(Box::new(native));
        } else {
            findings.approximate("its parameters are out of range; the fallback plays");
        }
    }
    findings.report(ctx, effect);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use slidock_core_api::Limits;
    use slidock_core_api::limits::CellBudget;
    use slidock_ir::diagnostics::{Collect, Stage};
    use slidock_ir::effects::{
        ClockParams, ClockStart, Dir4, DirParams, InOut, IrisParams, IrisShape,
    };
    use slidock_ir::ids::{NodeId, Sha256};
    use slidock_ir::units::Ms;

    use crate::context::{DataId, Layer, MediaLookup, Setup};
    use crate::graph::ObjectGraph;

    struct NoMedia;

    impl MediaLookup for NoMedia {
        fn media(&mut self, _: Option<DataId>) -> Option<Sha256> {
            None
        }

        fn is_image(&mut self, _: Option<DataId>) -> bool {
            false
        }
    }

    /// Runs `f` with a slide's lowering on the deck path.
    fn lowering<T>(f: impl FnOnce(&mut Lowering<'_>) -> T) -> T {
        let graph = ObjectGraph::default();
        let limits = Limits::default();
        let mut media = NoMedia;
        let mut sink = Collect::new(Stage::Frontend, "keynote");
        let setup = Setup {
            graph: &graph,
            container: NodeId::new("kn-1").unwrap(),
            layer: Layer::Slide,
            limits: &limits,
            media: &mut media,
            slide_width: 1024.0,
            slide_height: 768.0,
            slide_number: 1,
            native_motion: true,
        };
        let cells = CellBudget { cells: u64::MAX };
        let mut ctx = Lowering::new(setup, &cells, &mut sink);
        f(&mut ctx)
    }

    fn animation(animation_type: &str, direction: Option<u32>) -> kn::AnimationAttributesArchive {
        kn::AnimationAttributesArchive {
            animation_type: Some(animation_type.to_owned()),
            direction,
            ..Default::default()
        }
    }

    /// The frontend plays every row of `slidock-presets`' Keynote table as the table holds it, transitions and
    /// builds alike, and reads every direction number as the table reads it, reporting the readings the table marks
    /// inferred: the table is the one source of Keynote's readings.
    #[test]
    fn every_row_and_direction_plays_as_the_table_holds_it() {
        let mut compared = 0;
        for id in table::IDS {
            if let Some(row) = table::transition(id) {
                let out = lowering(|ctx| {
                    native_transition(ctx, id, &kn::TransitionAttributesArchive::default())
                });
                assert_eq!(out.native.map(|native| native.effect), row.effect, "{id}");
                compared += 1;
            }
            for (kind, animation_type) in [
                (StepKind::Entrance, "In"),
                (StepKind::Exit, "Out"),
                (StepKind::Emphasis, "Action"),
            ] {
                let Some(row) = table::build(id, kind) else {
                    continue;
                };
                let build = kn::BuildArchive {
                    attributes: Some(kn::BuildAttributesArchive {
                        animation_attributes: Some(animation(animation_type, None)),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                let out = lowering(|ctx| {
                    native_build(
                        ctx,
                        &mut Findings::default(),
                        id,
                        kind,
                        &build,
                        1,
                        Ms(500),
                        Delivery::All,
                    )
                });
                let played = out.native.map(|native| native.effect);
                let held = row.build_effect().filter(|effect| {
                    effect.plays_in(kind) && native_kind(Some(animation_type), id, kind).is_ok()
                });
                match held {
                    // A motion path's row is the shape its stored path fills in.
                    Some(BuildEffect::Path(_)) => {}
                    held => assert_eq!(played.as_ref(), held, "{id} {kind:?}"),
                }
                compared += 1;
            }
        }
        assert!(compared > 200, "{compared}");
        let samples: [(TransitionEffect, Steered); 4] = [
            (
                TransitionEffect::Cover(DirParams { dir: Dir::Left }),
                Steered::Motion,
            ),
            (
                TransitionEffect::Clock(ClockParams {
                    cw: true,
                    from: ClockStart::Twelve,
                }),
                Steered::Rotation,
            ),
            (
                TransitionEffect::Iris(IrisParams {
                    shape: IrisShape::Circle,
                    in_out: InOut::Out,
                }),
                Steered::Iris,
            ),
            (
                TransitionEffect::Zoom(slidock_ir::effects::TransitionZoomParams {
                    in_out: InOut::Out,
                    rotate: false,
                }),
                Steered::Zoom,
            ),
        ];
        for number in table::DIRECTIONS {
            let (reading, _) = table::direction(number).unwrap();
            for (sample, steered) in &samples {
                let mut effect = *sample;
                let mut findings = Findings::default();
                let attributes = animation("In", Some(number));
                read_direction(
                    &mut effect,
                    Some(&attributes),
                    None,
                    &mut findings,
                    steer_transition,
                );
                let takes = matches!(
                    (reading, steered),
                    (KeynoteDirection::Motion(_), Steered::Motion)
                        | (KeynoteDirection::Rotation { .. }, Steered::Rotation)
                        | (KeynoteDirection::Zoom(_), Steered::Iris | Steered::Zoom)
                );
                if !takes {
                    assert_eq!(effect, *sample, "{number} {steered:?}");
                    assert_eq!(findings.unknown_direction, Some(i64::from(number)));
                    continue;
                }
                let read = match (&effect, reading) {
                    (TransitionEffect::Cover(p), KeynoteDirection::Motion(dir)) => p.dir == dir,
                    (TransitionEffect::Clock(p), KeynoteDirection::Rotation { cw }) => p.cw == cw,
                    (TransitionEffect::Iris(p), KeynoteDirection::Zoom(in_out)) => {
                        p.in_out == in_out
                    }
                    (TransitionEffect::Zoom(p), KeynoteDirection::Zoom(in_out)) => {
                        p.in_out == in_out
                    }
                    _ => false,
                };
                assert!(read, "{number} {steered:?} {effect:?}");
                let inferred = table::direction_certainty(number, *steered) == Certainty::Inferred;
                assert_eq!(
                    !findings.approximations.is_empty(),
                    inferred,
                    "{number} {steered:?}"
                );
            }
        }
    }

    #[test]
    fn mirrors_only_the_horizontal_part() {
        for (dir, expected) in [
            (Dir::Left, Dir::Right),
            (Dir::RightDown, Dir::LeftDown),
            (Dir::Up, Dir::Up),
        ] {
            assert_eq!(mirrored(dir), expected);
            assert_eq!(mirrored(mirrored(dir)), dir);
        }
    }

    #[test]
    fn push_takes_only_straight_directions() {
        let steer = |effect: &mut TransitionEffect, direction| {
            steer_transition(effect, direction, &mut Findings::default())
        };
        let mut push = TransitionEffect::Push(slidock_ir::effects::Dir4Params { dir: Dir4::Left });
        assert_eq!(
            steer(&mut push, KeynoteDirection::Motion(Dir::Down)),
            Some(Steered::Motion)
        );
        assert_eq!(
            push,
            TransitionEffect::Push(slidock_ir::effects::Dir4Params { dir: Dir4::Down })
        );
        assert_eq!(
            steer(&mut push, KeynoteDirection::Motion(Dir::LeftUp)),
            None
        );
        assert_eq!(
            steer(&mut push, KeynoteDirection::Rotation { cw: true }),
            None
        );
        let mut iris = TransitionEffect::Iris(slidock_ir::effects::IrisParams {
            shape: slidock_ir::effects::IrisShape::Circle,
            in_out: InOut::Out,
        });
        assert_eq!(
            steer(&mut iris, KeynoteDirection::Zoom(InOut::In)),
            Some(Steered::Iris)
        );
    }

    /// An exit's iris or zoom number names what the element does, so it plays the entrance of the other number
    /// reversed, unreported where Keynote's files pin the number (41/42 on an iris, 43/44 on a zoom): its build
    /// movies show exit 41 closing to the centre and exit 44 shrinking. Elsewhere the number stays read by inference.
    #[test]
    fn an_exit_plays_the_reverse_of_the_entrance_of_the_other_number() {
        let play = |id: &str, kind: StepKind, number: u32| {
            let animation_type = if kind == StepKind::Exit { "Out" } else { "In" };
            let build = kn::BuildArchive {
                attributes: Some(kn::BuildAttributesArchive {
                    animation_attributes: Some(animation(animation_type, Some(number))),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let mut findings = Findings::default();
            let out = lowering(|ctx| {
                native_build(
                    ctx,
                    &mut findings,
                    id,
                    kind,
                    &build,
                    1,
                    Ms(500),
                    Delivery::All,
                )
            });
            let in_out = match out.native.map(|native| native.effect) {
                Some(BuildEffect::Iris(p)) => p.in_out,
                Some(BuildEffect::Zoom(p)) => p.in_out,
                other => panic!("{id} {other:?}"),
            };
            let reasons: Vec<String> = findings.approximations;
            (in_out, reasons)
        };
        let iris_edge = vec!["iris approximates it".to_owned()];
        for (id, number, entrance, exit, reasons) in [
            (
                "apple:wipe-iris",
                41,
                InOut::In,
                InOut::Out,
                iris_edge.clone(),
            ),
            (
                "apple:wipe-iris",
                42,
                InOut::Out,
                InOut::In,
                iris_edge.clone(),
            ),
            ("apple:zoom", 43, InOut::In, InOut::Out, Vec::new()),
            ("apple:zoom", 44, InOut::Out, InOut::In, Vec::new()),
        ] {
            assert_eq!(
                play(id, StepKind::Entrance, number),
                (entrance, reasons.clone()),
                "{id} {number}"
            );
            assert_eq!(
                play(id, StepKind::Exit, number),
                (exit, reasons),
                "{id} {number}"
            );
        }
        let (in_out, reasons) = play("apple:zoom", StepKind::Exit, 41);
        assert_eq!(in_out, InOut::Out);
        assert_eq!(
            reasons,
            vec!["direction 41 is read by inference".to_owned()]
        );
    }
}
