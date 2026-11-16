//! ODP's effects: slide transitions as SMIL filters (`smil:type`, `smil:subtype`, `smil:direction`,
//! `smil:fadeColor`, on the drawing-page style or the timing root's `anim:transitionFilter`), and build presets by
//! their `ooo-*` ids ([`crate::ooo`]). LibreOffice's own catalogs ([`crate::libreoffice_effects`],
//! [`crate::libreoffice_transitions`]) list what its UI writes; every row of them maps.

use slidock_ir::effects::{
    ClockParams, ClockStart, CutParams, Diagonal, Dir, Dir4, Dir4Params, DirParams, FadeParams,
    HexColor, InOut, IrisParams, IrisShape, NativeId, Orient, OrientParams, SplitParams,
    StripsParams, Through, TransitionEffect, TransitionFallback, TransitionZoomParams, WheelParams,
    WipeParams,
};
use slidock_ir::formats::FileFormat;

use crate::ooo::{self, PresetClass};
use crate::{BuildRow, Lookup, TransitionRow, Unknown};

/// The native id of a build preset: `odp:<preset-id>`.
pub fn build_id(preset_id: &str) -> NativeId {
    NativeId::new(FileFormat::Odp.into(), preset_id)
}

/// The native id of a transition: `odp:<type>/<subtype>`, with `/reverse` when reversed.
pub fn transition_id(smil_type: &str, subtype: Option<&str>, reverse: bool) -> NativeId {
    let mut name = smil_type.to_owned();
    if let Some(subtype) = subtype {
        name.push('/');
        name.push_str(subtype);
    }
    if reverse {
        name.push_str("/reverse");
    }
    NativeId::new(FileFormat::Odp.into(), &name)
}

/// The transition preset LibreOffice migrates an ODF 1.0 `presentation:transition-style` to
/// (`EffectMigration::SetFadeEffect`): the style is a `FadeEffect` name in lower case with `-` for `_`
/// (`fade-from-upperleft` is `FADE_FROM_UPPERLEFT`), and its first row in the migration table names the preset.
/// `None` for a style with no row (interlocking bars, `fly-away`) or a preset `transitions.xml` does not hold.
pub fn legacy_transition(style: &str) -> Option<&'static crate::TransitionPreset> {
    if style.is_empty() || !style.bytes().all(|b| b.is_ascii_lowercase() || b == b'-') {
        return None;
    }
    let fade_effect = style.to_ascii_uppercase().replace('-', "_");
    let (_, preset) = crate::libreoffice::migration::FADE_EFFECTS
        .iter()
        .find(|(effect, _)| *effect == fade_effect)?;
    crate::libreoffice_transitions()
        .iter()
        .find(|transition| transition.id == *preset)
}

/// A build preset's row, by its class token (`entrance`), id and sub-type; `None` for a class or id the table does
/// not know.
pub fn build(class: &str, preset_id: &str, subtype: Option<&str>) -> Option<Lookup<BuildRow>> {
    ooo::build(PresetClass::parse(class)?, preset_id, subtype)
}

/// The direction the content moves for a `from…` sub-type (`fromTop` moves down).
fn from_side(subtype: &str) -> Option<Dir> {
    Some(match subtype {
        "fromLeft" => Dir::Right,
        "fromRight" => Dir::Left,
        "fromTop" => Dir::Down,
        "fromBottom" => Dir::Up,
        "fromTopLeft" => Dir::RightDown,
        "fromTopRight" => Dir::LeftDown,
        "fromBottomLeft" => Dir::RightUp,
        "fromBottomRight" => Dir::LeftUp,
        _ => return None,
    })
}

fn orient(subtype: &str) -> Option<Orient> {
    match subtype {
        "horizontal" => Some(Orient::Horizontal),
        "vertical" => Some(Orient::Vertical),
        _ => None,
    }
}

struct Reader<'a> {
    subtype: Option<&'a str>,
    unknown: Vec<Unknown>,
}

impl Reader<'_> {
    fn read<T>(&mut self, default: (T, &'static str), parse: impl Fn(&str) -> Option<T>) -> T {
        let Some(value) = self.subtype else {
            return default.0;
        };
        parse(value).unwrap_or_else(|| {
            self.unknown.push(Unknown {
                option: "smil:subtype",
                value: value.to_owned(),
                used: default.1,
            });
            default.0
        })
    }
}

fn reversed(dir: Dir, reverse: bool) -> Dir {
    if reverse { dir.opposite() } else { dir }
}

fn flip(in_out: InOut, reverse: bool) -> InOut {
    match (in_out, reverse) {
        (InOut::Out, true) => InOut::In,
        (InOut::In, true) => InOut::Out,
        (in_out, false) => in_out,
    }
}

/// The colour a fade passes through: SMIL's default, black, when the file names none.
fn through(color: Option<&str>, unknown: &mut Vec<Unknown>) -> (Through, Option<HexColor>) {
    match color.map(|color| (color, HexColor::parse(color))) {
        None | Some((_, Ok(HexColor::BLACK))) => (Through::Black, None),
        Some((_, Ok(HexColor::WHITE))) => (Through::White, None),
        Some((_, Ok(color))) => (Through::Color, Some(color)),
        Some((value, Err(_))) => {
            unknown.push(Unknown {
                option: "smil:fadeColor",
                value: value.to_owned(),
                used: "#000000",
            });
            (Through::Black, None)
        }
    }
}

/// A transition's row, from its SMIL filter. `None` for a type the table does not know; an unknown sub-type reads
/// as the type's default and is listed. LibreOffice's 3D transitions (`miscShapeWipe`) and the shape wipes the
/// player has no recipe for keep their canonical fallback.
pub fn transition(
    smil_type: &str,
    subtype: Option<&str>,
    reverse: bool,
    fade_color: Option<&str>,
) -> Option<Lookup<TransitionRow>> {
    use TransitionEffect as T;
    let mut r = Reader {
        subtype,
        unknown: Vec::new(),
    };
    let row = match smil_type {
        "barWipe" => {
            if subtype == Some("fadeOverColor") {
                TransitionRow::effect(T::Cut(CutParams {
                    through_black: true,
                }))
            } else {
                let dir = r.read((Dir::Right, "leftToRight"), |subtype| match subtype {
                    "leftToRight" => Some(Dir::Right),
                    "topToBottom" => Some(Dir::Down),
                    _ => None,
                });
                TransitionRow::effect(T::Wipe(WipeParams {
                    dir: reversed(dir, reverse),
                    soft: false,
                }))
            }
        }
        "pushWipe" => match subtype {
            Some("combHorizontal") => TransitionRow::effect(T::Comb(OrientParams {
                orient: Orient::Horizontal,
            })),
            Some("combVertical") => TransitionRow::effect(T::Comb(OrientParams {
                orient: Orient::Vertical,
            })),
            _ => {
                let dir = r.read((Dir4::Right, "fromLeft"), |subtype| {
                    from_side(subtype).and_then(Dir::as_dir4)
                });
                let dir = if reverse { dir.opposite() } else { dir };
                TransitionRow::effect(T::Push(Dir4Params { dir }))
            }
        },
        "slideWipe" => {
            let dir = r.read((Dir::Right, "fromLeft"), from_side);
            let params = DirParams { dir };
            TransitionRow::effect(if reverse {
                T::Uncover(params)
            } else {
                T::Cover(params)
            })
        }
        "barnDoorWipe" => {
            let orient = r.read((Orient::Vertical, "vertical"), orient);
            TransitionRow::effect(T::Split(SplitParams {
                orient,
                in_out: flip(InOut::Out, reverse),
            }))
        }
        "irisWipe" => {
            let shape = r.read((IrisShape::Box, "rectangle"), |subtype| match subtype {
                "rectangle" => Some(IrisShape::Box),
                "diamond" => Some(IrisShape::Diamond),
                _ => None,
            });
            TransitionRow::effect(T::Iris(IrisParams {
                shape,
                in_out: flip(InOut::Out, reverse),
            }))
        }
        "ellipseWipe" => {
            let exact = r.read((true, "circle"), |subtype| match subtype {
                "circle" => Some(true),
                "horizontal" | "vertical" => Some(false),
                _ => None,
            });
            let row = TransitionRow::effect(T::Iris(IrisParams {
                shape: IrisShape::Circle,
                in_out: flip(InOut::Out, reverse),
            }));
            if exact { row } else { row.approximate() }
        }
        "fourBoxWipe" => {
            let in_out = r.read((InOut::Out, "cornersOut"), |subtype| match subtype {
                "cornersOut" => Some(InOut::Out),
                "cornersIn" => Some(InOut::In),
                _ => None,
            });
            TransitionRow::effect(T::Iris(IrisParams {
                shape: IrisShape::Plus,
                in_out: flip(in_out, reverse),
            }))
        }
        "pinWheelWipe" => {
            let spokes = r.read((4, "fourBlade"), |subtype| match subtype {
                "oneBlade" => Some(1),
                "twoBladeVertical" | "twoBladeHorizontal" => Some(2),
                "threeBlade" => Some(3),
                "fourBlade" => Some(4),
                "eightBlade" => Some(8),
                _ => None,
            });
            TransitionRow::effect(T::Wheel(WheelParams {
                spokes,
                cw: !reverse,
            }))
        }
        "fanWipe" => {
            let exact = r.read((true, "centerTop"), |subtype| match subtype {
                "centerTop" => Some(true),
                "centerRight" | "top" | "right" | "bottom" | "left" => Some(false),
                _ => None,
            });
            let row = TransitionRow::effect(T::Wedge);
            if exact { row } else { row.approximate() }
        }
        "clockWipe" => {
            let from = r.read(
                (ClockStart::Twelve, "clockwiseTwelve"),
                |subtype| match subtype {
                    "clockwiseTwelve" => Some(ClockStart::Twelve),
                    "clockwiseThree" => Some(ClockStart::Three),
                    "clockwiseSix" => Some(ClockStart::Six),
                    "clockwiseNine" => Some(ClockStart::Nine),
                    _ => None,
                },
            );
            TransitionRow::effect(T::Clock(ClockParams { cw: !reverse, from }))
        }
        "blindsWipe" => {
            let orient = r.read((Orient::Horizontal, "horizontal"), orient);
            TransitionRow::effect(T::Blinds(OrientParams { orient }))
        }
        "randomBarWipe" => {
            let orient = r.read((Orient::Horizontal, "horizontal"), orient);
            TransitionRow::effect(T::RandomBars(OrientParams { orient }))
        }
        "checkerBoardWipe" => {
            let dir = r.read((Dir4::Right, "across"), |subtype| match subtype {
                "across" => Some(Dir4::Right),
                "down" => Some(Dir4::Down),
                _ => None,
            });
            TransitionRow::effect(T::Checker(Dir4Params { dir }))
        }
        "waterfallWipe" => {
            let corner = r.read((Diagonal::LeftDown, "horizontalRight"), |subtype| {
                Some(match (subtype, reverse) {
                    ("horizontalRight" | "verticalRight", false) => Diagonal::LeftDown,
                    ("horizontalLeft" | "verticalLeft", false) => Diagonal::RightDown,
                    ("horizontalLeft" | "verticalLeft", true) => Diagonal::LeftUp,
                    ("horizontalRight" | "verticalRight", true) => Diagonal::RightUp,
                    _ => return None,
                })
            });
            TransitionRow::effect(T::Strips(StripsParams { corner }))
        }
        "dissolve" => TransitionRow::effect(T::Dissolve),
        "fade" => {
            let over_color = r.read((false, "crossfade"), |subtype| match subtype {
                "crossfade" => Some(false),
                "fadeOverColor" | "fadeToColor" | "fadeFromColor" => Some(true),
                _ => None,
            });
            let (through, color) = if over_color {
                through(fade_color, &mut r.unknown)
            } else {
                (Through::None, None)
            };
            TransitionRow::effect(T::Fade(FadeParams { through, color }))
        }
        "zoom" => {
            let (in_out, rotate) = r.read(((InOut::In, false), "zoomIn"), |subtype| {
                Some(match subtype {
                    "rotateIn" => (InOut::In, true),
                    "rotateOut" => (InOut::Out, true),
                    "zoomIn" | "in" => (InOut::In, false),
                    "zoomOut" | "out" => (InOut::Out, false),
                    _ => return None,
                })
            });
            TransitionRow::effect(T::Zoom(TransitionZoomParams {
                in_out: flip(in_out, reverse),
                rotate,
            }))
        }
        "random" => TransitionRow::effect(T::Random),
        "snakeWipe" | "spiralWipe" | "zigZagWipe" | "bowTieWipe" | "veeWipe" | "starWipe"
        | "boxSnakesWipe" | "parallelSnakesWipe" | "doubleFanWipe" | "doubleSweepWipe"
        | "saloonDoorWipe" | "windshieldWipe" | "singleSweepWipe" | "arrowHeadWipe"
        | "triangleWipe" | "pentagonWipe" | "hexagonWipe" | "eyeWipe" | "roundRectWipe"
        | "boxWipe" | "miscDiagonalWipe" => TransitionRow::fallback(TransitionFallback::Wipe),
        "miscShapeWipe" => TransitionRow::fallback(TransitionFallback::Dissolve),
        _ => return None,
    };
    Some(Lookup {
        row,
        unknown: r.unknown,
    })
}
