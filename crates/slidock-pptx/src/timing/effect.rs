//! What one effect plays: its preset (`presetClass` + `presetID` +
//! `presetSubtype`) through the shared preset table, the parameters the table leaves to the source (a spin's angle,
//! a scale, a colour, a motion path) from the effect's own behaviours, and otherwise, for a preset the table does
//! not name or plays only approximately, the keyframes its behaviours compile to (an approximate entry plays when they
//! compile to nothing).

use slidock_ir::effects::limits::{SCALE, SPIN_DEGREES};
use slidock_ir::effects::{
    BuildEffect, Delivery, Dir, Dir4Params, DirParams, FloatInParams, NativeId, OrientParams,
    Reveal, Scale, StepFallback, StepKind, Vertical,
};
use slidock_ir::units::{Deg, Ratio, UnitRatio};
use slidock_presets::pptx::{PptxClass, build, build_id};
use slidock_presets::{BuildPlay, Unknown, kind_fallback};

use super::behavior::{Amounts, Behavior, Value};
use super::fmla::{self, Env, Rng};
use super::keyframes::{self, Clip, Stage};
use crate::xml::Element;

/// An effect's preset attributes as written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Preset {
    pub class: Option<String>,
    pub id: Option<String>,
    pub subtype: Option<String>,
}

impl Preset {
    pub fn of(ctn: &Element) -> Preset {
        Preset {
            class: ctn.attr("presetClass").map(str::to_owned),
            id: ctn.attr("presetID").map(str::to_owned),
            subtype: ctn.attr("presetSubtype").map(str::to_owned),
        }
    }

    pub fn is_effect(&self) -> bool {
        self.class.is_some()
    }
}

/// Why a step plays its fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unmapped {
    /// The preset has no vocabulary entry and its behaviours compile to nothing.
    NoEntry,
    /// The preset's entry needs a value its behaviours do not give (a colour, a path).
    MissingValue,
}

/// What an effect plays.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Played {
    pub id: NativeId,
    pub kind: StepKind,
    /// The vocabulary entry (tier 1) or the compiled behaviours (tier 2); `None` plays the fallback.
    pub effect: Option<BuildEffect>,
    pub fallback: StepFallback,
    /// The delivery the preset implies (Colored Lettering, by character).
    pub delivery: Option<Delivery>,
    pub unmapped: Option<Unmapped>,
    /// The effect is the preset's approximate entry, played because its behaviours compile to nothing.
    pub approximated: bool,
    /// Option values the table did not know, read as defaults.
    pub unknown: Vec<Unknown>,
    /// What the compiled behaviours leave out, named (tier 2 only).
    pub lost: Vec<String>,
    /// Why the behaviours play instead of the preset's entry: the clip they state is one the entry cannot play.
    pub conflict: Option<String>,
}

/// The kind of a custom effect, from its behaviours: one that shows its target enters, one that hides it exits, a
/// motion path moves, a command drives media, anything else emphasises.
fn behaviour_kind(behaviors: &[Behavior]) -> StepKind {
    if behaviors.iter().any(|b| b.visibility() == Some(true)) {
        StepKind::Entrance
    } else if behaviors.iter().any(|b| b.visibility() == Some(false)) {
        StepKind::Exit
    } else if behaviors
        .iter()
        .any(|b| matches!(b, Behavior::Motion { .. }))
    {
        StepKind::Motion
    } else if behaviors
        .iter()
        .any(|b| matches!(b, Behavior::Command { .. }))
    {
        StepKind::Media
    } else {
        StepKind::Emphasis
    }
}

fn number(text: &str) -> Option<u32> {
    let text = text.trim();
    (!text.is_empty() && text.len() <= 9 && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

/// A number a `p:set` or `p:anim` gives, a formula evaluated over nothing.
fn plain(value: &Value) -> Option<f64> {
    match value {
        Value::Num(x) => Some(*x),
        Value::Str(text) => fmla::parse(text)
            .ok()?
            .eval(&Env::default(), &mut Rng::new(0))
            .ok(),
        Value::Bool(_) | Value::Color(_) => None,
    }
}

/// The parameters a preset's entry leaves to the source, from its behaviours; `None` when a value it needs is
/// missing or out of the vocabulary's range.
fn refine(effect: BuildEffect, behaviors: &[Behavior], stage: &Stage) -> Option<BuildEffect> {
    Some(match effect {
        BuildEffect::ColorChange(mut params) => {
            params.to = behaviors.iter().find_map(|b| match b {
                Behavior::Color {
                    to: Some(Value::Color(Some(hex))),
                    ..
                } => Some(*hex),
                _ => None,
            })?;
            BuildEffect::ColorChange(params)
        }
        BuildEffect::GrowShrink(mut params) => {
            let found = behaviors.iter().find_map(|b| match b {
                Behavior::Scale {
                    amounts: Amounts { by, to, .. },
                    ..
                } => to.or(*by),
                _ => None,
            });
            if let Some((x, y)) = found {
                let ratio = |v: f64| {
                    (0.0..=SCALE)
                        .contains(&v)
                        .then(|| Ratio::quantise(v).ok())
                        .flatten()
                };
                params.scale = Scale {
                    x: ratio(x)?,
                    y: ratio(y)?,
                };
            }
            BuildEffect::GrowShrink(params)
        }
        BuildEffect::Spin(mut params) => {
            let by = behaviors.iter().find_map(|b| match b {
                Behavior::Rotate {
                    amounts: Amounts { by: Some(by), .. },
                    ..
                } => Some(*by),
                _ => None,
            });
            if let Some(by) = by {
                if by == 0.0 || by.abs() > SPIN_DEGREES {
                    return None;
                }
                params.deg = Deg::quantise(by.abs()).ok()?;
                params.cw = by > 0.0;
            }
            BuildEffect::Spin(params)
        }
        BuildEffect::Transparency(mut params) => {
            let alpha = behaviors.iter().find_map(|b| match b {
                Behavior::Set { to: Some(to), .. } if b.attr() == Some("style.opacity") => {
                    plain(to)
                }
                _ => None,
            });
            if let Some(alpha) = alpha {
                params.alpha = (0.0..=1.0)
                    .contains(&alpha)
                    .then(|| UnitRatio::quantise(alpha).ok())
                    .flatten()?;
            }
            BuildEffect::Transparency(params)
        }
        BuildEffect::Path(mut params) => {
            params.d = behaviors.iter().find_map(|b| match b {
                Behavior::Motion {
                    path: Some(path), ..
                } => keyframes::path(path, stage),
                _ => None,
            })?;
            BuildEffect::Path(params)
        }
        other => other,
    })
}

/// A clip preset as the effect's own `animEffect` states it: the filter and the transition decide, as for tier 2 and
/// transitions (LibreOffice's sub-type labels name some clips the other way round), and the preset's entry only
/// where no clip filter is stated. A stated clip the entry cannot play, a filter of another clip or one run the
/// other way than the step (an exit's filter run `in`, an entrance's `out`, an unknown `transition`), is `Err` with
/// the reason: the behaviours play it.
fn clip_from_filter(
    effect: BuildEffect,
    behaviors: &[Behavior],
    exiting: bool,
) -> Result<BuildEffect, String> {
    let Some((filter, transition, reveal)) = behaviors.iter().find_map(|b| match b {
        Behavior::Effect {
            filter: Some(filter),
            transition,
            ..
        } => match keyframes::filter(filter, exiting || transition == "out") {
            Some(Clip::Reveal(reveal)) => Some((filter, transition, reveal)),
            _ => None,
        },
        _ => None,
    }) else {
        return Ok(effect);
    };
    let stated = match (&effect, reveal) {
        (BuildEffect::Wipe(_), Reveal::Wipe(params)) => Some(BuildEffect::Wipe(params)),
        (BuildEffect::Split(_), Reveal::Split(params)) => Some(BuildEffect::Split(params)),
        (BuildEffect::Iris(_), Reveal::Iris(params)) => Some(BuildEffect::Iris(params)),
        (BuildEffect::Wheel(_), Reveal::Wheel(params)) => Some(BuildEffect::Wheel(params)),
        (BuildEffect::Blinds(_), Reveal::Blinds(OrientParams { orient })) => {
            Some(BuildEffect::Blinds(OrientParams { orient }))
        }
        (BuildEffect::RandomBars(_), Reveal::RandomBars(OrientParams { orient })) => {
            Some(BuildEffect::RandomBars(OrientParams { orient }))
        }
        (BuildEffect::Checker(_), Reveal::Checker(Dir4Params { dir })) => {
            Some(BuildEffect::Checker(Dir4Params { dir }))
        }
        (BuildEffect::Strips(_), Reveal::Strips(params)) => Some(BuildEffect::Strips(params)),
        (BuildEffect::Dissolve, Reveal::Dissolve) => Some(BuildEffect::Dissolve),
        (BuildEffect::Wedge, Reveal::Wedge) => Some(BuildEffect::Wedge),
        (
            BuildEffect::Wipe(_)
            | BuildEffect::Split(_)
            | BuildEffect::Iris(_)
            | BuildEffect::Wheel(_)
            | BuildEffect::Blinds(_)
            | BuildEffect::RandomBars(_)
            | BuildEffect::Checker(_)
            | BuildEffect::Strips(_)
            | BuildEffect::Dissolve
            | BuildEffect::Wedge,
            _,
        ) => {
            return Err(format!(
                "its filter {filter} is not the {} its preset plays; its behaviours play",
                effect.name()
            ));
        }
        _ => None,
    };
    let Some(stated) = stated else {
        return Ok(effect);
    };
    let runs = if exiting { "out" } else { "in" };
    if transition != runs {
        return Err(format!(
            "its filter {filter} runs {transition}, the {} its preset plays runs {runs}; its behaviours play",
            effect.name()
        ));
    }
    Ok(stated)
}

/// The way the effect's `ppt_x`/`ppt_y` animations move the shape, in slide pixels from their first value to their
/// last.
fn net_motion(behaviors: &[Behavior], stage: &Stage) -> (f64, f64) {
    let shape = stage.shape.unwrap_or(keyframes::ShapeBox {
        x: 0.5,
        y: 0.5,
        w: 0.25,
        h: 0.25,
        rotation: 0.0,
    });
    let env = Env {
        value: 0.0,
        x: shape.x,
        y: shape.y,
        w: shape.w,
        h: shape.h,
    };
    let mut motion = (0.0, 0.0);
    for behavior in behaviors {
        let Behavior::Anim { tavs, .. } = behavior else {
            continue;
        };
        let value = |index: usize| -> Option<f64> {
            match tavs.get(index)?.value.as_ref()? {
                Value::Num(x) => Some(*x),
                Value::Str(text) => fmla::parse(text).ok()?.eval(&env, &mut Rng::new(0)).ok(),
                _ => None,
            }
        };
        let (Some(first), Some(last)) = (value(0), tavs.len().checked_sub(1).and_then(value))
        else {
            continue;
        };
        match behavior.attr() {
            Some("ppt_x") => motion.0 += (last - first) * stage.w_px,
            Some("ppt_y") => motion.1 += (last - first) * stage.h_px,
            _ => {}
        }
    }
    motion
}

/// A drifting preset's direction from the way its behaviours move the shape, when they do: the table names no
/// direction for these (`presetSubtype` 0).
fn direction_from_motion(
    effect: BuildEffect,
    behaviors: &[Behavior],
    stage: &Stage,
) -> BuildEffect {
    let (dx, dy) = net_motion(behaviors, stage);
    match effect {
        BuildEffect::FloatIn(_) if dy != 0.0 => BuildEffect::FloatIn(FloatInParams {
            dir: if dy < 0.0 {
                Vertical::Up
            } else {
                Vertical::Down
            },
        }),
        BuildEffect::FadeAndMove(params) => {
            let horizontal = if dx < 0.0 { Dir::Left } else { Dir::Right };
            let vertical = if dy < 0.0 { Dir::Up } else { Dir::Down };
            let dir = if dx == 0.0 && dy == 0.0 {
                params.dir
            } else if dy.abs() * 2.0 <= dx.abs() {
                horizontal
            } else if dx.abs() * 2.0 <= dy.abs() {
                vertical
            } else {
                match (horizontal, vertical) {
                    (Dir::Left, Dir::Up) => Dir::LeftUp,
                    (Dir::Left, _) => Dir::LeftDown,
                    (_, Dir::Up) => Dir::RightUp,
                    _ => Dir::RightDown,
                }
            };
            BuildEffect::FadeAndMove(DirParams { dir })
        }
        other => other,
    }
}

/// What an effect with `preset` and `behaviors` plays, on `stage`; `seed` serves its formulas' `rand`.
pub(crate) fn play(
    preset: &Preset,
    behaviors: &[Behavior],
    duration: slidock_ir::units::Ms,
    stage: &Stage,
    seed: u64,
) -> Played {
    let mut unknown = Vec::new();
    let class = match preset.class.as_deref() {
        Some(token) => {
            let class = PptxClass::parse(token);
            if class.is_none() {
                unknown.push(Unknown {
                    option: "presetClass",
                    value: token.to_owned(),
                    used: "custom",
                });
            }
            class
        }
        None => None,
    };
    let kind = class.map_or_else(|| behaviour_kind(behaviors), PptxClass::kind);
    let id = preset.id.as_deref().map(|text| match number(text) {
        Some(id) => id,
        None => {
            unknown.push(Unknown {
                option: "presetID",
                value: text.to_owned(),
                used: "0",
            });
            0
        }
    });
    let subtype = match preset.subtype.as_deref() {
        None => 0,
        Some(text) => number(text).unwrap_or_else(|| {
            unknown.push(Unknown {
                option: "presetSubtype",
                value: text.to_owned(),
                used: "0",
            });
            0
        }),
    };
    let id_number = id.unwrap_or(0);
    let native = build_id(class.map_or("custom", PptxClass::token), id_number, subtype);
    let lookup = class.and_then(|class| build(class.token(), id_number, subtype));
    let mut delivery = None;
    let mut unmapped = None;
    let mut approximated = false;
    let mut lost = Vec::new();
    let effect = match lookup {
        Some(lookup) => {
            unknown.extend(lookup.unknown);
            delivery = lookup.row.delivery;
            // An approximate entry yields to the preset's own behaviours, which play its motion.
            let compiled = lookup
                .row
                .approximate
                .then(|| behaviors_effect(behaviors, kind, duration, stage, seed, &mut lost))
                .flatten();
            match lookup.row.play {
                BuildPlay::Effect(_) if compiled.is_some() => compiled,
                BuildPlay::Effect(effect) if lookup.row.refine => {
                    approximated = lookup.row.approximate;
                    let refined = refine(effect, behaviors, stage);
                    if refined.is_none() {
                        unmapped = Some(Unmapped::MissingValue);
                    }
                    refined
                }
                BuildPlay::Effect(effect) => {
                    approximated = lookup.row.approximate;
                    Some(effect)
                }
                BuildPlay::Behaviors => None,
                BuildPlay::Fallback => {
                    unmapped = Some(Unmapped::NoEntry);
                    None
                }
            }
            .or_else(|| {
                (unmapped.is_none())
                    .then(|| behaviors_effect(behaviors, kind, duration, stage, seed, &mut lost))
                    .flatten()
            })
        }
        None => behaviors_effect(behaviors, kind, duration, stage, seed, &mut lost),
    };
    // A font family or size change moves the layout, which no effect plays.
    let effect = if behaviors
        .iter()
        .any(|b| b.attr().is_some_and(keyframes::changes_layout))
    {
        unmapped = Some(Unmapped::NoEntry);
        None
    } else {
        effect
    };
    let exiting = kind == StepKind::Exit;
    let mut conflict = None;
    let effect = effect
        .and_then(
            |effect| match clip_from_filter(effect, behaviors, exiting) {
                Ok(effect) => Some(effect),
                Err(reason) => {
                    approximated = false;
                    conflict = Some(reason);
                    behaviors_effect(behaviors, kind, duration, stage, seed, &mut lost)
                }
            },
        )
        .map(|effect| direction_from_motion(effect, behaviors, stage))
        .filter(|effect| effect.plays_in(kind));
    if effect.is_none() && unmapped.is_none() {
        unmapped = Some(Unmapped::NoEntry);
    }
    let approximated = approximated && effect.is_some();
    let fallback = match &effect {
        Some(BuildEffect::Behaviors(_)) | None => kind_fallback(kind),
        Some(effect) => effect.fallback(kind),
    };
    if !matches!(effect, Some(BuildEffect::Behaviors(_))) {
        lost.clear();
    }
    Played {
        id: native,
        kind,
        effect,
        fallback,
        delivery,
        unmapped,
        approximated,
        unknown,
        lost,
        conflict,
    }
}

fn behaviors_effect(
    behaviors: &[Behavior],
    kind: StepKind,
    duration: slidock_ir::units::Ms,
    stage: &Stage,
    seed: u64,
    lost: &mut Vec<String>,
) -> Option<BuildEffect> {
    let compiled = keyframes::compile(behaviors, kind == StepKind::Exit, duration, stage, seed);
    *lost = compiled.lost;
    (!compiled.tracks.is_empty()).then_some(BuildEffect::Behaviors(compiled.tracks))
}
