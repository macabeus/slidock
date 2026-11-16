//! Tier 1: a preset (`presentation:preset-class` + `preset-id` + `preset-sub-type`) maps through
//! the shared `ooo-*` table ([`slidock_presets::ooo::build`], the one PPTX reads too), then takes from its own
//! behaviours what they state exactly:
//! - the clip of its `transitionFilter`, when the row plays the same clip: the SMIL filter decides a clip's
//!   orientation, corner and direction, never the sub-type's label (LibreOffice's venetian-blinds `vertical` plays
//!   `blindsWipe horizontal`);
//! - the direction its `x`/`y` motion takes, for an entry that names one the sub-type does not (`fadeAndMove`,
//!   `floatIn`);
//! - the values a row leaves to the source (`refine`): a colour change's colour, a scale, a spin's angle, a
//!   transparency, a motion path.
//!
//! A row that plays the behaviours, a row the table marks approximate (its entry does not play the preset's motion),
//! and a preset the table does not know, compile to tier 2 ([`super::smil`]); an approximate row whose behaviours
//! compile to nothing plays its entry, flagged `approximate`.

use std::mem::discriminant;

use slidock_ir::effects::{
    BuildEffect, ColorChangeParams, Delivery, Dir, FloatInParams, GrowShrinkParams, HexColor,
    PathParams, PeekParams, SpinParams, TransparencyParams, Vertical,
};
use slidock_ir::units::{Deg, Ratio, UnitRatio};
use slidock_presets::ooo::PresetClass;
use slidock_presets::{BuildPlay, Unknown};

use super::expr::{Expr, Vars};
use super::smil::{self, filter_effect, motion_path};
use super::value;
use super::{Geometry, Leaf, LeafKind};
use crate::names::qn;

/// What an effect plays.
pub(crate) struct Mapped {
    /// The vocabulary entry or tier-2 keyframes; `None` plays the fallback.
    pub effect: Option<BuildEffect>,
    /// The delivery the preset implies.
    pub delivery: Option<Delivery>,
    /// What tier 2 left out of the behaviours.
    pub lost: Option<String>,
    /// Whether the vocabulary entry plays as an approximation (the table's row says so, and the behaviours compile
    /// to nothing).
    pub approximate: bool,
}

/// The effect a preset plays.
pub(crate) fn map(
    class: PresetClass,
    preset_id: Option<&str>,
    subtype: Option<&str>,
    leaves: &[Leaf<'_>],
    geometry: &Geometry,
    unknown: &mut Vec<Unknown>,
) -> Mapped {
    let lookup = preset_id.and_then(|id| slidock_presets::ooo::build(class, id, subtype));
    let Some(lookup) = lookup else {
        return tier2(leaves, geometry, None);
    };
    unknown.extend(lookup.unknown);
    let row = lookup.row;
    match row.play {
        // An approximation yields to the preset's own behaviours, which play its motion.
        BuildPlay::Effect(effect) if row.approximate => {
            let compiled = tier2(leaves, geometry, row.delivery);
            if compiled.effect.is_some() {
                return compiled;
            }
            Mapped {
                effect: Some(refine(effect, row.refine, leaves, geometry, unknown)),
                delivery: row.delivery,
                lost: None,
                approximate: true,
            }
        }
        BuildPlay::Effect(effect) => Mapped {
            effect: Some(refine(effect, row.refine, leaves, geometry, unknown)),
            delivery: row.delivery,
            lost: None,
            approximate: false,
        },
        BuildPlay::Behaviors => tier2(leaves, geometry, row.delivery),
        BuildPlay::Fallback => Mapped {
            effect: None,
            delivery: row.delivery,
            lost: None,
            approximate: false,
        },
    }
}

fn tier2(leaves: &[Leaf<'_>], geometry: &Geometry, delivery: Option<Delivery>) -> Mapped {
    let compiled = smil::compile(leaves, geometry);
    if compiled.tracks.is_empty() {
        return Mapped {
            effect: None,
            delivery,
            lost: None,
            approximate: false,
        };
    }
    Mapped {
        effect: Some(BuildEffect::Behaviors(compiled.tracks)),
        delivery,
        lost: (!compiled.lost.is_empty()).then(|| compiled.lost.join(", ")),
        approximate: false,
    }
}

/// The direction an effect's `x`/`y` animations move it, from their first value to their last; `None` when they
/// do not move it.
fn motion(leaves: &[Leaf<'_>], geometry: &Geometry) -> Option<Dir> {
    let vars = Vars {
        x: geometry.x,
        y: geometry.y,
        width: geometry.width,
        height: geometry.height,
        dollar: 0.0,
    };
    let travel = |axis: &str| -> f64 {
        leaves
            .iter()
            .filter(|leaf| leaf.kind == LeafKind::Animate && leaf.attribute() == Some(axis))
            .find_map(|leaf| {
                let values = leaf.element.attr(qn!("smil:values"))?;
                let eval = |text: &str| Expr::parse(text).ok()?.eval(&vars).ok();
                let first = eval(values.split(';').next()?)?;
                let last = eval(values.split(';').next_back()?)?;
                Some(last - first)
            })
            .unwrap_or(0.0)
    };
    const STILL: f64 = 1e-9;
    let sign = |d: f64| {
        if d > STILL {
            1
        } else if d < -STILL {
            -1
        } else {
            0
        }
    };
    Some(match (sign(travel("x")), sign(travel("y"))) {
        (1, 0) => Dir::Right,
        (-1, 0) => Dir::Left,
        (0, 1) => Dir::Down,
        (0, -1) => Dir::Up,
        (1, 1) => Dir::RightDown,
        (1, -1) => Dir::RightUp,
        (-1, 1) => Dir::LeftDown,
        (-1, -1) => Dir::LeftUp,
        _ => return None,
    })
}

/// The first leaf of `kind` animating one of `attributes` (any attribute when empty).
fn leaf<'l, 'a>(
    leaves: &'l [Leaf<'a>],
    kind: LeafKind,
    attributes: &[&str],
) -> Option<&'l Leaf<'a>> {
    leaves.iter().find(|leaf| {
        leaf.kind == kind
            && (attributes.is_empty() || leaf.attribute().is_some_and(|a| attributes.contains(&a)))
    })
}

/// The value a leaf ends at: `smil:to`, else its last `smil:values`, else `smil:by`.
fn end_value<'a>(leaf: &Leaf<'a>) -> Option<&'a str> {
    let element = leaf.element;
    element
        .attr(qn!("smil:to"))
        .or_else(|| {
            element
                .attr(qn!("smil:values"))
                .and_then(|v| v.split(';').next_back())
        })
        .or_else(|| element.attr(qn!("smil:by")))
}

/// Records a source value the row cannot take, which reads as the row's default.
fn bad(unknown: &mut Vec<Unknown>, option: &'static str, value: &str, used: &'static str) {
    unknown.push(Unknown {
        option,
        value: value.to_owned(),
        used,
    });
}

fn refine(
    effect: BuildEffect,
    values: bool,
    leaves: &[Leaf<'_>],
    geometry: &Geometry,
    unknown: &mut Vec<Unknown>,
) -> BuildEffect {
    let clip = leaf(leaves, LeafKind::TransitionFilter, &[]).and_then(filter_effect);
    let effect = match (effect, clip) {
        (effect, Some(clip)) if discriminant(&effect) == discriminant(&clip) && is_clip(&clip) => {
            clip
        }
        (BuildEffect::Peek(peek), Some(BuildEffect::FlyIn(fly))) => BuildEffect::Peek(PeekParams {
            dir: fly.dir.as_dir4().unwrap_or(peek.dir),
            slow: peek.slow,
        }),
        (effect, _) => effect,
    };
    let effect = match effect {
        BuildEffect::FadeAndMove(mut p) => {
            if let Some(dir) = motion(leaves, geometry) {
                p.dir = dir;
            }
            BuildEffect::FadeAndMove(p)
        }
        BuildEffect::FloatIn(p) => BuildEffect::FloatIn(FloatInParams {
            dir: match motion(leaves, geometry) {
                Some(Dir::Up | Dir::LeftUp | Dir::RightUp) => Vertical::Up,
                Some(Dir::Down | Dir::LeftDown | Dir::RightDown) => Vertical::Down,
                _ => p.dir,
            },
        }),
        effect => effect,
    };
    if !values {
        return effect;
    }
    match effect {
        BuildEffect::ColorChange(p) => {
            let source = leaf(
                leaves,
                LeafKind::AnimateColor,
                &[
                    "fill-color",
                    "color",
                    "stroke-color",
                    "line-color",
                    "char-color",
                ],
            )
            .and_then(end_value);
            let to = match source.map(|text| (text, HexColor::parse(text))) {
                None => p.to,
                Some((_, Ok(color))) => color,
                Some((text, Err(_))) => {
                    bad(unknown, "smil:to", text, "#000000");
                    p.to
                }
            };
            BuildEffect::ColorChange(ColorChangeParams {
                property: p.property,
                to,
            })
        }
        BuildEffect::GrowShrink(p) => {
            let source = leaf(leaves, LeafKind::AnimateTransform, &[])
                .filter(|leaf| leaf.element.attr(qn!("svg:type")) == Some("scale"))
                .and_then(end_value);
            let scale = source.map(|text| {
                let mut parts = text.split([',', ' ']).filter(|part| !part.is_empty());
                let x = parts.next().and_then(value::number);
                let y = parts.next().map_or(x, value::number);
                let ok = |v: f64| (0.0..=slidock_ir::effects::limits::SCALE).contains(&v);
                match (x, y) {
                    (Some(x), Some(y)) if ok(x) && ok(y) => Ratio::quantise(x)
                        .ok()
                        .zip(Ratio::quantise(y).ok())
                        .ok_or(text),
                    _ => Err(text),
                }
            });
            let scale = match scale {
                None => p.scale,
                Some(Ok((x, y))) => slidock_ir::effects::Scale { x, y },
                Some(Err(text)) => {
                    bad(unknown, "smil:by", text, "1.5,1.5");
                    p.scale
                }
            };
            BuildEffect::GrowShrink(GrowShrinkParams {
                scale,
                hold: p.hold,
            })
        }
        BuildEffect::Spin(p) => {
            let source = leaf(leaves, LeafKind::AnimateTransform, &[])
                .filter(|leaf| leaf.element.attr(qn!("svg:type")) == Some("rotate"))
                .and_then(end_value);
            match source.map(|text| (text, value::number(text))) {
                None => BuildEffect::Spin(p),
                Some((_, Some(deg)))
                    if deg != 0.0 && deg.abs() <= slidock_ir::effects::limits::SPIN_DEGREES =>
                {
                    match Deg::quantise(deg.abs()) {
                        Ok(abs) => BuildEffect::Spin(SpinParams {
                            deg: abs,
                            cw: deg > 0.0,
                            hold: p.hold,
                        }),
                        Err(_) => BuildEffect::Spin(p),
                    }
                }
                Some((text, _)) => {
                    bad(unknown, "smil:by", text, "360");
                    BuildEffect::Spin(p)
                }
            }
        }
        BuildEffect::Transparency(p) => {
            let source = leaf(leaves, LeafKind::Set, &["opacity"])
                .or_else(|| leaf(leaves, LeafKind::Animate, &["opacity"]))
                .and_then(end_value);
            let alpha = match source.map(|text| (text, value::number(text))) {
                None => p.alpha,
                Some((text, Some(alpha))) => match UnitRatio::quantise(alpha) {
                    Ok(alpha) if (0.0..=1.0).contains(&alpha.to_f64()) => alpha,
                    _ => {
                        bad(unknown, "smil:to", text, "0.5");
                        p.alpha
                    }
                },
                Some((text, None)) => {
                    bad(unknown, "smil:to", text, "0.5");
                    p.alpha
                }
            };
            BuildEffect::Transparency(TransparencyParams {
                alpha,
                hold: p.hold,
            })
        }
        BuildEffect::Path(p) => {
            let source = leaf(leaves, LeafKind::AnimateMotion, &[])
                .and_then(|leaf| leaf.element.attr(qn!("svg:path")));
            let d = match source.map(|text| (text, motion_path(text, geometry))) {
                None => p.d,
                Some((_, Some(d))) => d,
                Some((text, None)) => {
                    bad(unknown, "svg:path", text, "no motion");
                    p.d
                }
            };
            BuildEffect::Path(PathParams {
                d,
                auto_rotate: p.auto_rotate,
                hold: p.hold,
            })
        }
        effect => effect,
    }
}

/// Whether an effect is a clip a `transitionFilter` plays.
fn is_clip(effect: &BuildEffect) -> bool {
    matches!(
        effect,
        BuildEffect::Wipe(_)
            | BuildEffect::Split(_)
            | BuildEffect::Iris(_)
            | BuildEffect::Wheel(_)
            | BuildEffect::Wedge
            | BuildEffect::Clock(_)
            | BuildEffect::Blinds(_)
            | BuildEffect::Checker(_)
            | BuildEffect::RandomBars(_)
            | BuildEffect::Strips(_)
    )
}
