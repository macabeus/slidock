//! The `ooo-*` build presets: LibreOffice's names for PowerPoint's presets, which ODP files write
//! (`presentation:preset-id`, `presentation:preset-sub-type`) and LibreOffice maps PPTX `presetID`s onto. One table,
//! keyed by (class, name, sub-type), serves both formats ([`crate::pptx::build`] converts first).
//!
//! Sub-types name a side the content comes from (`from-left`) or goes to (`to-left`), an orientation, in/out, or a
//! number of spokes. A side becomes the direction the content moves: an entrance from the left moves right, an exit
//! to the left moves left; a wipe's side is the edge it starts at, for entrances and exits alike. A zoom exit's
//! sub-type names what the element does, so its row holds the entrance it reverses. An iris's or
//! a split's sub-type names the entrance its filter plays, for exits too: LibreOffice's engine plays `ooo-exit-box
//! in` as the `in` entrance backwards, a hole opening from the centre. An axis is the dimension
//! LibreOffice's `effects.xml` animates (swivel `vertical` turns the width: horizontal), never the sub-type's word. A
//! row is exact only where its entry plays the preset's own motion; otherwise it is marked approximate, and a frontend
//! whose source carries the preset's behaviours plays those instead. The rows give LibreOffice's rendering (a
//! hard-edged wipe); [`crate::pptx::build`] sets PowerPoint's where it differs.

use slidock_ir::effects::{
    Axis, AxisParams, BuildEffect, ClockParams, ClockStart, ColorChangeParams, ColorProperty,
    Delivery, Diagonal, Dir, Dir4, Dir4Params, DirParams, Distance, FloatInParams, FlyInParams,
    GrowShrinkParams, HexColor, InOut, IntensityParams, IrisParams, IrisShape, Orient,
    OrientParams, PathParams, PathSeg, PeekParams, PlayParams, PulseParams, RepeatParams, Scale,
    SpinInParams, SpinParams, SplitParams, StepKind, StripsParams, TransparencyParams, Vertical,
    WheelParams, WipeParams, ZoomOrigin, ZoomParams,
};
use slidock_ir::units::{Deg, Ms, Px, Ratio, UnitRatio};

use crate::{BuildRow, Lookup, Unknown};

/// A preset class, as ODP names it (`presentation:preset-class`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PresetClass {
    Entrance,
    Exit,
    Emphasis,
    MotionPath,
    MediaCall,
    OleAction,
    Custom,
}

impl PresetClass {
    pub const ALL: [PresetClass; 7] = [
        PresetClass::Entrance,
        PresetClass::Exit,
        PresetClass::Emphasis,
        PresetClass::MotionPath,
        PresetClass::MediaCall,
        PresetClass::OleAction,
        PresetClass::Custom,
    ];

    pub const fn token(self) -> &'static str {
        match self {
            PresetClass::Entrance => "entrance",
            PresetClass::Exit => "exit",
            PresetClass::Emphasis => "emphasis",
            PresetClass::MotionPath => "motion-path",
            PresetClass::MediaCall => "media-call",
            PresetClass::OleAction => "ole-action",
            PresetClass::Custom => "custom",
        }
    }

    pub fn parse(token: &str) -> Option<PresetClass> {
        PresetClass::ALL
            .into_iter()
            .find(|class| class.token() == token)
    }

    /// The step kind the class builds; `None` for OLE actions and custom effects, whose kind the behaviours decide.
    pub const fn kind(self) -> Option<StepKind> {
        match self {
            PresetClass::Entrance => Some(StepKind::Entrance),
            PresetClass::Exit => Some(StepKind::Exit),
            PresetClass::Emphasis => Some(StepKind::Emphasis),
            PresetClass::MotionPath => Some(StepKind::Motion),
            PresetClass::MediaCall => Some(StepKind::Media),
            PresetClass::OleAction | PresetClass::Custom => None,
        }
    }

    /// The word preset ids of the class carry (`ooo-entrance-…`, `ooo-motionpath-…`).
    const fn id_word(self) -> Option<&'static str> {
        match self {
            PresetClass::Entrance => Some("entrance"),
            PresetClass::Exit => Some("exit"),
            PresetClass::Emphasis => Some("emphasis"),
            PresetClass::MotionPath => Some("motionpath"),
            PresetClass::MediaCall => Some("media"),
            PresetClass::OleAction | PresetClass::Custom => None,
        }
    }
}

const fn ratio(steps: i64) -> Ratio {
    Ratio::from_steps(steps)
}

const fn unit(steps: i64) -> UnitRatio {
    UnitRatio::from_steps(steps)
}

const fn degrees(deg: i64) -> Deg {
    Deg::from_steps(deg * 1_000)
}

/// The side a `from-…`/`to-…` sub-type names, as a direction pointing at it (`from-top` → `u`).
pub fn side(subtype: &str) -> Option<Dir> {
    let name = subtype
        .strip_prefix("from-")
        .or_else(|| subtype.strip_prefix("to-"))?;
    Some(match name {
        "left" => Dir::Left,
        "right" => Dir::Right,
        "top" => Dir::Up,
        "bottom" => Dir::Down,
        "top-left" => Dir::LeftUp,
        "top-right" => Dir::RightUp,
        "bottom-left" => Dir::LeftDown,
        "bottom-right" => Dir::RightDown,
        _ => return None,
    })
}

/// The direction an element moves when it comes from (entrance) or goes to (exit) `side`.
fn motion(kind: StepKind, side: Dir) -> Dir {
    if kind == StepKind::Exit {
        side
    } else {
        side.opposite()
    }
}

fn in_out(subtype: &str) -> Option<InOut> {
    match subtype {
        "in" => Some(InOut::In),
        "out" => Some(InOut::Out),
        _ => None,
    }
}

/// The orientation venetian blinds and random bars play for a sub-type: LibreOffice's words name the opposite of
/// their filter's (`vertical` plays `blindsWipe horizontal`).
fn filter_orient(subtype: &str) -> Option<Orient> {
    match subtype {
        "vertical" => Some(Orient::Horizontal),
        "horizontal" => Some(Orient::Vertical),
        _ => None,
    }
}

/// Reads a sub-type, or falls back to `default` and records the value it did not know.
struct Reader<'a> {
    subtype: Option<&'a str>,
    unknown: Vec<Unknown>,
}

impl Reader<'_> {
    fn read<T>(&mut self, default: (T, &'static str), parse: impl Fn(&str) -> Option<T>) -> T {
        match self.subtype {
            None => default.0,
            Some(value) => parse(value).unwrap_or_else(|| {
                self.unknown.push(Unknown {
                    option: "presetSubtype",
                    value: value.to_owned(),
                    used: default.1,
                });
                default.0
            }),
        }
    }
}

fn effect(effect: BuildEffect, kind: StepKind) -> BuildRow {
    BuildRow::effect(effect, kind)
}

fn zoom(in_out: InOut, origin: ZoomOrigin, slight: bool) -> BuildEffect {
    BuildEffect::Zoom(ZoomParams {
        in_out,
        origin,
        slight,
        fade: false,
        overshoot: false,
    })
}

/// A zoom sub-type: in or out, from the object or the slide centre, fully or slightly. An exit's sub-type names
/// what the element does (`out` shrinks it to nothing, `in` grows it), so it reverses the entrance of the other
/// name: exit `out` is `zoom{in}` played backwards. LibreOffice also writes two raw PowerPoint numbers for exits:
/// 544 (shrinks to the slide centre, `in-from-screen-center` reversed) and 20 (grows from it).
fn zoom_subtype(kind: StepKind, subtype: &str) -> Option<BuildEffect> {
    let named = |in_out: InOut| {
        if kind == StepKind::Exit {
            in_out.opposite()
        } else {
            in_out
        }
    };
    Some(match subtype {
        "in" => zoom(named(InOut::In), ZoomOrigin::Object, false),
        "out" => zoom(named(InOut::Out), ZoomOrigin::Object, false),
        "in-slightly" => zoom(named(InOut::In), ZoomOrigin::Object, true),
        "out-slightly" => zoom(named(InOut::Out), ZoomOrigin::Object, true),
        "in-from-screen-center" => zoom(named(InOut::In), ZoomOrigin::SlideCenter, false),
        "out-from-screen-center" => zoom(named(InOut::Out), ZoomOrigin::SlideCenter, false),
        "544" => zoom(InOut::In, ZoomOrigin::SlideCenter, false),
        "20" => zoom(InOut::Out, ZoomOrigin::SlideCenter, false),
        _ => return None,
    })
}

fn stretch_axis(subtype: &str) -> Option<Axis> {
    match subtype {
        "across" | "horizontal" | "from-left" | "from-right" => Some(Axis::Horizontal),
        "downward" | "vertical" | "from-top" | "from-bottom" => Some(Axis::Vertical),
        "from-top-left" | "from-top-right" | "from-bottom-left" | "from-bottom-right" => {
            Some(Axis::Both)
        }
        _ => None,
    }
}

fn spin_in() -> BuildEffect {
    BuildEffect::SpinIn(SpinInParams {
        deg: degrees(360),
        cw: true,
    })
}

/// The row of an entrance or exit preset `name` (its id without `ooo-entrance-`/`ooo-exit-`).
fn entrance_exit(name: &str, kind: StepKind, r: &mut Reader<'_>) -> Option<BuildRow> {
    let row = match name {
        "appear" | "disappear" => effect(BuildEffect::Appear, kind),
        "fade-in" | "fade-out" => effect(BuildEffect::Fade, kind),
        "dissolve-in" | "dissolve" => effect(BuildEffect::Dissolve, kind),
        "fly-in" | "fly-out" => {
            let side = r.read((Dir::Down, "from-bottom"), side);
            effect(
                BuildEffect::FlyIn(FlyInParams {
                    dir: motion(kind, side),
                    distance: Distance::OffSlide,
                }),
                kind,
            )
        }
        "fly-in-slow" | "crawl-out" | "peek-in" | "peek-out" => {
            let side = r.read((Dir4::Down, "from-bottom"), |subtype| {
                side(subtype).and_then(Dir::as_dir4)
            });
            let dir = if kind == StepKind::Exit {
                side
            } else {
                side.opposite()
            };
            let slow = matches!(name, "fly-in-slow" | "crawl-out");
            effect(BuildEffect::Peek(PeekParams { dir, slow }), kind)
        }
        "wipe" => {
            let edge = r.read((Dir4::Down, "from-bottom"), |subtype| {
                side(subtype).and_then(Dir::as_dir4)
            });
            // LibreOffice's wipe is a hard-edged `barWipe`, as its slide transition is.
            effect(
                BuildEffect::Wipe(WipeParams {
                    dir: edge.opposite().dir(),
                    soft: false,
                }),
                kind,
            )
        }
        // The orientation is the filter's: LibreOffice's sub-type words name the other one (`vertical` plays
        // `blindsWipe horizontal`).
        "venetian-blinds" => {
            let orient = r.read((Orient::Horizontal, "vertical"), filter_orient);
            effect(BuildEffect::Blinds(OrientParams { orient }), kind)
        }
        "random-bars" => {
            let orient = r.read((Orient::Horizontal, "vertical"), filter_orient);
            effect(BuildEffect::RandomBars(OrientParams { orient }), kind)
        }
        "checkerboard" => {
            let dir = r.read((Dir4::Right, "across"), |subtype| match subtype {
                "across" => Some(Dir4::Right),
                "downward" => Some(Dir4::Down),
                _ => None,
            });
            effect(BuildEffect::Checker(Dir4Params { dir }), kind)
        }
        "box" | "circle" | "oval" | "diamond" | "plus" => {
            let in_out = r.read((InOut::In, "in"), in_out);
            let shape = match name {
                "box" => IrisShape::Box,
                "diamond" => IrisShape::Diamond,
                "plus" => IrisShape::Plus,
                _ => IrisShape::Circle,
            };
            let row = effect(BuildEffect::Iris(IrisParams { shape, in_out }), kind);
            if name == "oval" {
                row.approximate()
            } else {
                row
            }
        }
        "split" => {
            let (orient, in_out) = r.read(
                ((Orient::Horizontal, InOut::Out), "horizontal-out"),
                |subtype| {
                    Some(match subtype {
                        "horizontal-in" => (Orient::Horizontal, InOut::In),
                        "horizontal-out" => (Orient::Horizontal, InOut::Out),
                        "vertical-in" => (Orient::Vertical, InOut::In),
                        "vertical-out" => (Orient::Vertical, InOut::Out),
                        _ => return None,
                    })
                },
            );
            effect(BuildEffect::Split(SplitParams { orient, in_out }), kind)
        }
        // The corner the bands travel toward, as the preset's `waterfallWipe` plays it: entrance left-to-bottom is
        // `horizontalLeft`, toward the bottom right; the exits play their filter out (backwards).
        "diagonal-squares" => {
            let exit = kind == StepKind::Exit;
            let default = if exit {
                Diagonal::LeftUp
            } else {
                Diagonal::RightDown
            };
            let corner = r.read((default, "left-to-bottom"), |subtype| {
                Some(match (subtype, exit) {
                    ("left-to-bottom", false) => Diagonal::RightDown,
                    ("left-to-top", false) => Diagonal::RightUp,
                    ("right-to-bottom", false) => Diagonal::LeftDown,
                    ("right-to-top", false) => Diagonal::LeftUp,
                    ("left-to-bottom", true) => Diagonal::LeftUp,
                    ("left-to-top", true) => Diagonal::LeftDown,
                    ("right-to-bottom", true) => Diagonal::RightUp,
                    ("right-to-top", true) => Diagonal::RightDown,
                    _ => return None,
                })
            });
            effect(BuildEffect::Strips(StripsParams { corner }), kind)
        }
        "wedge" => effect(BuildEffect::Wedge, kind),
        "clock-wipe" => {
            let cw = r.read((true, "clockwise"), |subtype| match subtype {
                "clockwise" => Some(true),
                "counter-clockwise" => Some(false),
                _ => None,
            });
            effect(
                BuildEffect::Clock(ClockParams {
                    cw,
                    from: ClockStart::Twelve,
                }),
                kind,
            )
        }
        "wheel" => {
            let spokes = r.read((1, "1"), |subtype| {
                subtype
                    .parse::<u8>()
                    .ok()
                    .filter(|spokes| matches!(spokes, 1..=8))
            });
            effect(BuildEffect::Wheel(WheelParams { spokes, cw: true }), kind)
        }
        "snake-wipe" | "spiral-wipe" => BuildRow::fallback(kind),
        "random" => effect(BuildEffect::Random, kind),
        "expand" | "contract" => effect(
            BuildEffect::Expand(AxisParams {
                axis: Axis::Horizontal,
            }),
            kind,
        ),
        // LibreOffice's compress narrows the width onto the element (`width+.3` to `width`).
        "compress" => effect(
            BuildEffect::Compress(AxisParams {
                axis: Axis::Horizontal,
            }),
            kind,
        ),
        "stretchy" | "collapse" => {
            let axis = r.read((Axis::Horizontal, "across"), stretch_axis);
            let row = effect(BuildEffect::Stretch(AxisParams { axis }), kind);
            // From a side or a corner, it grows anchored there: `stretch` grows about the centre.
            if r.subtype
                .is_some_and(|subtype| subtype.starts_with("from-"))
            {
                row.approximate()
            } else {
                row
            }
        }
        "unfold" => effect(
            BuildEffect::Stretch(AxisParams {
                axis: Axis::Horizontal,
            }),
            kind,
        )
        .approximate(),
        "fade-in-and-swivel" | "fade-out-and-swivel" => effect(
            BuildEffect::Swivel(AxisParams {
                axis: Axis::Horizontal,
            }),
            kind,
        )
        .approximate(),
        "swivel" => {
            // `vertical` turns about a vertical axis, so the width changes.
            let axis = r.read((Axis::Horizontal, "vertical"), |subtype| match subtype {
                "vertical" => Some(Axis::Horizontal),
                "horizontal" => Some(Axis::Vertical),
                _ => None,
            });
            effect(BuildEffect::Swivel(AxisParams { axis }), kind)
        }
        "flip" => effect(
            BuildEffect::Flip(AxisParams {
                axis: Axis::Horizontal,
            }),
            kind,
        ),
        // Grows in; the exit shrinks away, the same zoom reversed.
        "fade-in-and-zoom" | "fade-out-and-zoom" => effect(
            BuildEffect::Zoom(ZoomParams {
                in_out: InOut::In,
                origin: ZoomOrigin::Object,
                slight: false,
                fade: true,
                overshoot: false,
            }),
            kind,
        ),
        "zoom" => {
            let default = zoom_subtype(kind, "in")?;
            let zoom = r.read((default, "in"), |subtype| zoom_subtype(kind, subtype));
            let row = effect(zoom, kind);
            // LibreOffice's zoom out from the screen centre (and the exit 20 that reverses it) starts large at the
            // slide's bottom centre, below the slide: no zoom origin plays that path.
            match (kind, r.subtype) {
                (StepKind::Entrance, Some("out-from-screen-center"))
                | (StepKind::Exit, Some("20")) => row.approximate(),
                _ => row,
            }
        }
        // LibreOffice's magnify grows from the slide's centre column past its size, then settles into place.
        "magnify" => effect(
            BuildEffect::Zoom(ZoomParams {
                in_out: InOut::In,
                origin: ZoomOrigin::Object,
                slight: false,
                fade: false,
                overshoot: true,
            }),
            kind,
        )
        .approximate(),
        "ascend" => effect(
            BuildEffect::FloatIn(FloatInParams { dir: Vertical::Up }),
            kind,
        ),
        "descend" => effect(
            BuildEffect::FloatIn(FloatInParams {
                dir: Vertical::Down,
            }),
            kind,
        ),
        "float" => effect(
            BuildEffect::FloatIn(FloatInParams { dir: Vertical::Up }),
            kind,
        )
        .approximate(),
        "rise-up" => effect(
            BuildEffect::FloatIn(FloatInParams { dir: Vertical::Up }),
            kind,
        )
        .approximate(),
        "sink-down" => effect(
            BuildEffect::FloatIn(FloatInParams {
                dir: Vertical::Down,
            }),
            kind,
        )
        .approximate(),
        "ease-in" => {
            effect(BuildEffect::FadeAndMove(DirParams { dir: Dir::Up }), kind).approximate()
        }
        "ease-out" => {
            effect(BuildEffect::FadeAndMove(DirParams { dir: Dir::Down }), kind).approximate()
        }
        "curve-up" => {
            effect(BuildEffect::FadeAndMove(DirParams { dir: Dir::Up }), kind).approximate()
        }
        "curve-down" => {
            effect(BuildEffect::FadeAndMove(DirParams { dir: Dir::Down }), kind).approximate()
        }
        "glide" => effect(
            BuildEffect::FadeAndMove(DirParams { dir: Dir::Right }),
            kind,
        )
        .approximate(),
        "turn-and-grow" => effect(BuildEffect::GrowTurn, kind),
        "spin-in" | "spin-out" => effect(spin_in(), kind),
        "center-revolve" | "spiral-in" | "spiral-out" => effect(spin_in(), kind).approximate(),
        "pinwheel" => effect(
            BuildEffect::SpinIn(SpinInParams {
                deg: degrees(720),
                cw: true,
            }),
            kind,
        )
        .approximate(),
        "colored-lettering" => effect(BuildEffect::Appear, kind)
            .delivered(Delivery::ByChar)
            .approximate(),
        // `effects.xml`'s entrance is one visibility switch that does not hold: the element shows while it lasts.
        // That is an appear the frontend hides after the switch's span (`after: hide`).
        "flash-once" if kind == StepKind::Entrance => effect(BuildEffect::Appear, kind),
        "flash-once" => effect(BuildEffect::Blink(RepeatParams { repeat: 1 }), kind),
        "boomerang" => effect(BuildEffect::Boomerang, kind),
        "bounce" => effect(BuildEffect::Bounce, kind),
        "movie-credits" => effect(BuildEffect::Credits, kind),
        "falling-in" => effect(BuildEffect::Drop, kind),
        "sling" => effect(BuildEffect::Sling, kind),
        "whip" => effect(BuildEffect::Whip, kind),
        "swish" => effect(BuildEffect::Swish, kind),
        "fold" | "thread" | "breaks" => BuildRow::behaviors(kind),
        _ => return None,
    };
    Some(row)
}

fn emphasis(name: &str, kind: StepKind) -> Option<BuildRow> {
    let color = |property| {
        effect(
            BuildEffect::ColorChange(ColorChangeParams {
                property,
                to: HexColor::BLACK,
            }),
            kind,
        )
        .refine()
    };
    Some(match name {
        "fill-color" => color(ColorProperty::Fill),
        "font-color" => color(ColorProperty::Text),
        "line-color" => color(ColorProperty::Line),
        "grow-and-shrink" => effect(
            BuildEffect::GrowShrink(GrowShrinkParams {
                scale: Scale {
                    x: ratio(150_000),
                    y: ratio(150_000),
                },
                hold: true,
            }),
            kind,
        )
        .refine(),
        "font-size" => effect(
            BuildEffect::GrowShrink(GrowShrinkParams {
                scale: Scale {
                    x: ratio(150_000),
                    y: ratio(150_000),
                },
                hold: true,
            }),
            kind,
        )
        .refine()
        .approximate(),
        "spin" => effect(
            BuildEffect::Spin(SpinParams {
                deg: degrees(360),
                cw: true,
                hold: true,
            }),
            kind,
        )
        .refine(),
        "transparency" => effect(
            BuildEffect::Transparency(TransparencyParams {
                alpha: unit(50_000),
                hold: true,
            }),
            kind,
        )
        .refine(),
        "bold-flash" => effect(BuildEffect::BoldFlash(RepeatParams { repeat: 1 }), kind),
        "blast" | "flash-bulb" => effect(
            BuildEffect::Pulse(PulseParams {
                scale: ratio(105_000),
            }),
            kind,
        )
        .approximate(),
        "flicker" => effect(BuildEffect::Blink(RepeatParams { repeat: 1 }), kind).approximate(),
        "blink" => effect(BuildEffect::Blink(RepeatParams { repeat: 1 }), kind),
        "teeter" => effect(
            BuildEffect::Teeter(IntensityParams {
                intensity: unit(50_000),
            }),
            kind,
        ),
        "wave" => effect(
            BuildEffect::Wave(IntensityParams {
                intensity: unit(50_000),
            }),
            kind,
        ),
        "font"
        | "font-style"
        | "bold-reveal"
        | "color-over-by-word"
        | "reveal-underline"
        | "color-blend"
        | "color-over-by-letter"
        | "complementary-color"
        | "complementary-color-2"
        | "contrasting-color"
        | "darken"
        | "desaturate"
        | "grow-with-color"
        | "lighten"
        | "style-emphasis"
        | "vertical-highlight"
        | "shimmer" => BuildRow::behaviors(kind),
        _ => return None,
    })
}

/// A motion path's row: the path comes from the effect's own `animateMotion`/`animMotion`.
pub fn motion_path(kind: StepKind) -> BuildRow {
    effect(
        BuildEffect::Path(PathParams {
            d: vec![PathSeg::MoveTo(Px::ZERO, Px::ZERO)],
            auto_rotate: false,
            hold: true,
        }),
        kind,
    )
    .refine()
}

/// The row of a preset, by class, id (`ooo-entrance-fly-in`, `libo-physics-fall`) and sub-type. `None` for an id
/// the table does not know: the caller plays the behaviours or the fallback, and reports it. An unknown sub-type
/// reads as the preset's default and is listed in the lookup.
pub fn build(class: PresetClass, id: &str, subtype: Option<&str>) -> Option<Lookup<BuildRow>> {
    let kind = class.kind()?;
    let mut reader = Reader {
        subtype,
        unknown: Vec::new(),
    };
    if id.starts_with("libo-physics-") {
        return matches!(
            kind,
            StepKind::Entrance | StepKind::Exit | StepKind::Emphasis
        )
        .then(|| Lookup::exact(BuildRow::behaviors(kind)));
    }
    let name = id
        .strip_prefix("ooo-")?
        .strip_prefix(class.id_word()?)?
        .strip_prefix('-')?;
    let row = match class {
        PresetClass::Entrance | PresetClass::Exit => entrance_exit(name, kind, &mut reader)?,
        PresetClass::Emphasis => emphasis(name, kind)?,
        PresetClass::MotionPath => motion_path(kind),
        PresetClass::MediaCall => effect(
            match name {
                "start" => BuildEffect::Play(PlayParams { from: Ms(0) }),
                "stop" => BuildEffect::Stop,
                "toggle-pause" => BuildEffect::Toggle,
                _ => return None,
            },
            kind,
        ),
        PresetClass::OleAction | PresetClass::Custom => return None,
    };
    Some(Lookup {
        row,
        unknown: reader.unknown,
    })
}
