//! PowerPoint's effects (ECMA-376 Part 1 §19.5, [MS-PPTX] for `p14`/`p15`/`p159`):
//! - transitions, by the `p:transition` child's local name and its options, schema defaults filled in;
//! - build presets, by `presetClass` + `presetID` + `presetSubtype`. The id names the preset, and the subtype's bit
//!   fields decode (1 top, 2 right, 4 bottom, 8 left, corners added up, 16 in, 32 out, 5 horizontal, 10 vertical, …),
//!   as LibreOffice's import does: its tables are generated MPL-2.0 data ([`crate::libreoffice`]'s `pptx.rs`). The
//!   row then comes from [`crate::ooo`].
//!
//! OOXML's direction tokens are read as the direction the content moves, and pass through unchanged. LibreOffice's
//! import pins that on the vertical axis (`push dir="d"` is SMIL `pushWipe fromTop`), for the corners (`cover
//! dir="lu"` is `slideWipe fromBottomRight`) and for the wipe on both axes (`wipe dir="l"` is `barWipe
//! leftToRight` reversed). For `l` and `r` on push, cover and pull LibreOffice's import reads the opposite (`l` is
//! `fromLeft`); Keynote settles the horizontal axis: its own PPTX export writes its default Push, Move In and Reveal,
//! whose content its movie export shows moving left, as `push`, `cover` and `pull` with `dir="l"` (and Google Slides
//! writes its "slide from right" as `push dir="l"`). The Office 2010 effects LibreOffice does not import stay
//! [`Certainty::Inferred`] until a probe settles them.
//!
//! PowerPoint feathers its wipes, transition and build alike (`soft: true`); LibreOffice's and Keynote's are
//! hard-edged.

use slidock_ir::effects::{BuildEffect, PlayParams};
use slidock_ir::effects::{
    CutParams, Diagonal, Dir, Dir4, Dir4Params, DirParams, FadeParams, FlythroughParams, InOut,
    IrisParams, IrisShape, MorphBy, MorphParams, NativeId, Orient, OrientParams, RevealParams,
    SplitParams, StepKind, StripsParams, Through, TransitionEffect, TransitionFallback,
    TransitionZoomParams, WheelParams, WipeParams,
};
use slidock_ir::formats::FileFormat;
use slidock_ir::units::Ms;

use crate::libreoffice;
use crate::ooo::{self, PresetClass};
use crate::{BuildPlay, BuildRow, Certainty, Lookup, TransitionRow, Unknown};

/// The native id of a transition: `pptx:<local name>`, and `pptx:prstTrans/<prst>` for Office 2013's presets.
pub fn transition_id(element: &str, prst: Option<&str>) -> NativeId {
    match prst {
        Some(prst) => NativeId::new(FileFormat::Pptx.into(), &format!("{element}/{prst}")),
        None => NativeId::new(FileFormat::Pptx.into(), element),
    }
}

/// The native id of a build preset: `pptx:<class>.<id>`, then `/<subtype>` when the subtype is not 0.
pub fn build_id(class: &str, preset_id: u32, subtype: u32) -> NativeId {
    let name = if subtype == 0 {
        format!("{class}.{preset_id}")
    } else {
        format!("{class}.{preset_id}/{subtype}")
    };
    NativeId::new(FileFormat::Pptx.into(), &name)
}

/// Reads options, falling back to the schema default and recording the value it did not know.
struct Options<'a, F: Fn(&str) -> Option<&'a str>> {
    attr: F,
    unknown: Vec<Unknown>,
}

impl<'a, F: Fn(&str) -> Option<&'a str>> Options<'a, F> {
    fn read<T>(
        &mut self,
        name: &'static str,
        default: (T, &'static str),
        parse: impl Fn(&str) -> Option<T>,
    ) -> T {
        let Some(value) = (self.attr)(name) else {
            return default.0;
        };
        parse(value).unwrap_or_else(|| {
            self.unknown.push(Unknown {
                option: name,
                value: value.to_owned(),
                used: default.1,
            });
            default.0
        })
    }

    fn flag(&mut self, name: &'static str) -> bool {
        self.read(name, (false, "0"), |value| match value {
            "1" | "true" => Some(true),
            "0" | "false" => Some(false),
            _ => None,
        })
    }

    fn side(&mut self, default: (Dir4, &'static str)) -> Dir4 {
        self.read("dir", default, |value| {
            Dir::from_token(value).and_then(Dir::as_dir4)
        })
    }

    fn left_right(&mut self) -> Dir4 {
        self.read("dir", (Dir4::Left, "l"), |value| match value {
            "l" => Some(Dir4::Left),
            "r" => Some(Dir4::Right),
            _ => None,
        })
    }

    fn eight(&mut self) -> Dir {
        self.read("dir", (Dir::Left, "l"), Dir::from_token)
    }

    fn orient(&mut self, name: &'static str) -> Orient {
        self.read(name, (Orient::Horizontal, "horz"), |value| match value {
            "horz" => Some(Orient::Horizontal),
            "vert" => Some(Orient::Vertical),
            _ => None,
        })
    }

    fn in_out(&mut self) -> InOut {
        self.read("dir", (InOut::Out, "out"), |value| match value {
            "in" => Some(InOut::In),
            "out" => Some(InOut::Out),
            _ => None,
        })
    }

    fn spokes(&mut self) -> u8 {
        self.read("spokes", (4, "4"), |value| {
            value
                .parse::<u8>()
                .ok()
                .filter(|spokes| matches!(spokes, 1..=8))
        })
    }
}

/// The Office 2013 transition presets (`p15:prstTrans@prst`); none has a player recipe.
pub const PRST_TRANS: [&str; 12] = [
    "fallOver",
    "drape",
    "curtains",
    "wind",
    "prestige",
    "fracture",
    "crush",
    "peelOff",
    "pageCurlDouble",
    "pageCurlSingle",
    "airplane",
    "origami",
];

/// A transition's row, by the local name of the `p:transition` child (after resolving `mc:AlternateContent`) and
/// its attributes. `None` for an element the table does not know (the caller plays a dissolve and reports
/// PX2402); an option value it does not know reads as the schema default and is listed.
pub fn transition<'a>(
    element: &str,
    attr: impl Fn(&str) -> Option<&'a str>,
) -> Option<Lookup<TransitionRow>> {
    use TransitionEffect as T;
    let mut o = Options {
        attr,
        unknown: Vec::new(),
    };
    let row = match element {
        "cut" => TransitionRow::effect(T::Cut(CutParams {
            through_black: o.flag("thruBlk"),
        })),
        "fade" => {
            let through = if o.flag("thruBlk") {
                Through::Black
            } else {
                Through::None
            };
            TransitionRow::effect(T::Fade(FadeParams {
                through,
                color: None,
            }))
        }
        "dissolve" => TransitionRow::effect(T::Dissolve),
        "push" => {
            let dir = o.side((Dir4::Left, "l"));
            TransitionRow::effect(T::Push(Dir4Params { dir }))
        }
        "cover" => {
            let dir = o.eight();
            TransitionRow::effect(T::Cover(DirParams { dir }))
        }
        "pull" => {
            let dir = o.eight();
            TransitionRow::effect(T::Uncover(DirParams { dir }))
        }
        "wipe" => TransitionRow::effect(T::Wipe(WipeParams {
            dir: o.side((Dir4::Left, "l")).dir(),
            soft: true,
        })),
        "split" => {
            let orient = o.orient("orient");
            TransitionRow::effect(T::Split(SplitParams {
                orient,
                in_out: o.in_out(),
            }))
        }
        "circle" | "diamond" | "plus" => TransitionRow::effect(T::Iris(IrisParams {
            shape: match element {
                "circle" => IrisShape::Circle,
                "diamond" => IrisShape::Diamond,
                _ => IrisShape::Plus,
            },
            in_out: InOut::Out,
        })),
        "zoom" => TransitionRow::effect(T::Zoom(TransitionZoomParams {
            in_out: o.in_out(),
            rotate: false,
        })),
        "newsflash" => TransitionRow::effect(T::Zoom(TransitionZoomParams {
            in_out: InOut::In,
            rotate: true,
        })),
        "wheel" => TransitionRow::effect(T::Wheel(WheelParams {
            spokes: o.spokes(),
            cw: true,
        })),
        "wheelReverse" => TransitionRow::effect(T::Wheel(WheelParams {
            spokes: o.spokes(),
            cw: false,
        })),
        "wedge" => TransitionRow::effect(T::Wedge),
        "blinds" => TransitionRow::effect(T::Blinds(OrientParams {
            orient: o.orient("dir"),
        })),
        "randomBar" => TransitionRow::effect(T::RandomBars(OrientParams {
            orient: o.orient("dir"),
        })),
        "comb" => TransitionRow::effect(T::Comb(OrientParams {
            orient: o.orient("dir"),
        })),
        "checker" => {
            let dir = match o.orient("dir") {
                Orient::Horizontal => Dir4::Right,
                Orient::Vertical => Dir4::Down,
            };
            TransitionRow::effect(T::Checker(Dir4Params { dir }))
        }
        "strips" => {
            let corner = o.read("dir", (Diagonal::LeftUp, "lu"), |value| {
                Dir::from_token(value).and_then(Dir::as_diagonal)
            });
            TransitionRow::effect(T::Strips(StripsParams { corner }))
        }
        "random" => TransitionRow::effect(T::Random),
        "flash" => TransitionRow::effect(T::Fade(FadeParams {
            through: Through::White,
            color: None,
        })),
        "reveal" => {
            let through_black = o.flag("thruBlk");
            TransitionRow::effect(T::Reveal(RevealParams {
                dir: o.left_right(),
                through_black,
            }))
            .inferred()
        }
        "flip" => TransitionRow::effect(T::Flip(Dir4Params {
            dir: o.left_right(),
        }))
        .inferred(),
        "switch" => TransitionRow::effect(T::Switch(Dir4Params {
            dir: o.left_right(),
        }))
        .inferred(),
        "gallery" => TransitionRow::effect(T::Gallery(Dir4Params {
            dir: o.left_right(),
        }))
        .inferred(),
        "conveyor" => TransitionRow::effect(T::Conveyor(Dir4Params {
            dir: o.left_right(),
        }))
        .inferred(),
        "ferris" => TransitionRow::effect(T::Ferris(Dir4Params {
            dir: o.left_right(),
        }))
        .inferred(),
        "pan" => TransitionRow::effect(T::Pan(Dir4Params {
            dir: o.side((Dir4::Up, "u")),
        }))
        .inferred(),
        "doors" => TransitionRow::effect(T::Doors(OrientParams {
            orient: o.orient("dir"),
        })),
        "window" => TransitionRow::effect(T::Window(OrientParams {
            orient: o.orient("dir"),
        })),
        "flythrough" => {
            let in_out = o.in_out();
            TransitionRow::effect(T::Flythrough(FlythroughParams {
                in_out,
                bounce: o.flag("hasBounce"),
            }))
        }
        "warp" => TransitionRow::effect(T::Zoom(TransitionZoomParams {
            in_out: o.in_out(),
            rotate: false,
        }))
        .approximate(),
        "vortex" | "ripple" | "glitter" | "honeycomb" | "shred" | "prism" => {
            TransitionRow::fallback(TransitionFallback::Dissolve)
        }
        "prstTrans" => {
            o.read("prst", ((), "fallOver"), |value| {
                PRST_TRANS.contains(&value).then_some(())
            });
            TransitionRow::fallback(TransitionFallback::Dissolve)
        }
        "morph" => {
            let by = o.read(
                "option",
                (MorphBy::Object, "byObject"),
                |value| match value {
                    "byObject" => Some(MorphBy::Object),
                    "byWord" => Some(MorphBy::Word),
                    "byChar" => Some(MorphBy::Char),
                    _ => None,
                },
            );
            TransitionRow::effect(T::Morph(MorphParams { by }))
        }
        _ => return None,
    };
    Some(Lookup {
        row,
        unknown: o.unknown,
    })
}

/// A `presetClass` value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PptxClass {
    Entrance,
    Exit,
    Emphasis,
    Path,
    Verb,
    MediaCall,
}

impl PptxClass {
    pub const ALL: [PptxClass; 6] = [
        PptxClass::Entrance,
        PptxClass::Exit,
        PptxClass::Emphasis,
        PptxClass::Path,
        PptxClass::Verb,
        PptxClass::MediaCall,
    ];

    pub const fn token(self) -> &'static str {
        match self {
            PptxClass::Entrance => "entr",
            PptxClass::Exit => "exit",
            PptxClass::Emphasis => "emph",
            PptxClass::Path => "path",
            PptxClass::Verb => "verb",
            PptxClass::MediaCall => "mediacall",
        }
    }

    pub fn parse(token: &str) -> Option<PptxClass> {
        PptxClass::ALL
            .into_iter()
            .find(|class| class.token() == token)
    }

    pub const fn kind(self) -> StepKind {
        match self {
            PptxClass::Entrance => StepKind::Entrance,
            PptxClass::Exit => StepKind::Exit,
            PptxClass::Emphasis | PptxClass::Verb => StepKind::Emphasis,
            PptxClass::Path => StepKind::Motion,
            PptxClass::MediaCall => StepKind::Media,
        }
    }
}

/// The highest motion-path preset id (1 to 64; 0 is a custom path).
pub const PATHS: u32 = 64;

/// The class word LibreOffice's preset table files a class under.
fn table_class(class: PptxClass) -> Option<&'static str> {
    match class {
        PptxClass::Entrance => Some("entrance"),
        PptxClass::Exit => Some("exit"),
        PptxClass::Emphasis => Some("emphasis"),
        PptxClass::Path | PptxClass::Verb | PptxClass::MediaCall => None,
    }
}

/// The entrance, exit or emphasis presets of a class: each `presetID` and the `ooo-*` preset it is, in
/// LibreOffice's order (`entr` 2 → `ooo-entrance-fly-in`).
pub fn presets(class: PptxClass) -> impl Iterator<Item = (u32, &'static str)> {
    let word = table_class(class);
    libreoffice::pptx::PRESETS
        .iter()
        .filter(move |preset| Some(preset.class) == word)
        .map(|preset| (preset.id, preset.name))
}

/// The `ooo-*` preset an entrance, exit or emphasis preset id is (`entr` 2 → `ooo-entrance-fly-in`).
pub fn preset_name(class: PptxClass, preset_id: u32) -> Option<&'static str> {
    presets(class)
        .find(|(id, _)| *id == preset_id)
        .map(|(_, name)| name)
}

/// A `presetSubtype` as the `ooo-*` sub-type LibreOffice converts it to: for entrances and exits by its table (and
/// the presets that convert their own way); a number it has no name for, and any other class's, stays a number,
/// which the table then reports.
pub fn subtype_token(class: PptxClass, preset_id: u32, subtype: u32) -> Option<String> {
    use libreoffice::pptx::{SUBTYPE_EXCEPTIONS, SUBTYPES, SUBTYPES_AS_NUMBERS};
    if subtype == 0 {
        return None;
    }
    let named = matches!(class, PptxClass::Entrance | PptxClass::Exit)
        && !SUBTYPES_AS_NUMBERS.contains(&preset_id);
    let name = named
        .then(|| {
            SUBTYPE_EXCEPTIONS
                .iter()
                .find(|(id, number, _)| *id == preset_id && *number == subtype)
                .map(|(_, _, name)| *name)
                .or_else(|| {
                    SUBTYPES
                        .iter()
                        .find(|(number, _)| *number == subtype)
                        .map(|(_, name)| *name)
                })
        })
        .flatten();
    Some(name.map_or_else(|| subtype.to_string(), str::to_owned))
}

/// A build preset's row, by `presetClass`, `presetID` and `presetSubtype`. `None` for a class or id the table does
/// not know (a custom effect, `presetID` 0 outside paths): the caller plays the behaviours or the fallback, and
/// reports it. An unknown subtype reads as the preset's default and is listed.
pub fn build(class: &str, preset_id: u32, subtype: u32) -> Option<Lookup<BuildRow>> {
    let class = PptxClass::parse(class)?;
    let kind = class.kind();
    match class {
        PptxClass::Path => (preset_id <= PATHS).then(|| Lookup::exact(ooo::motion_path(kind))),
        PptxClass::Verb => Some(Lookup::exact(BuildRow::fallback(kind))),
        PptxClass::MediaCall => {
            // PowerPoint's media presets: Play, Pause, Stop. The `p:cmd` beside them names the same command.
            let effect = match preset_id {
                1 => BuildEffect::Play(PlayParams { from: Ms(0) }),
                2 => BuildEffect::Toggle,
                3 => BuildEffect::Stop,
                _ => return None,
            };
            Some(Lookup::exact(BuildRow {
                certainty: Certainty::Inferred,
                ..BuildRow::effect(effect, kind)
            }))
        }
        PptxClass::Entrance | PptxClass::Exit | PptxClass::Emphasis => {
            let name = preset_name(class, preset_id)?;
            let ooo_class = match class {
                PptxClass::Entrance => PresetClass::Entrance,
                PptxClass::Exit => PresetClass::Exit,
                _ => PresetClass::Emphasis,
            };
            let token = subtype_token(class, preset_id, subtype);
            let mut lookup = ooo::build(ooo_class, name, token.as_deref())?;
            for unknown in &mut lookup.unknown {
                unknown.value = subtype.to_string();
            }
            if let BuildPlay::Effect(BuildEffect::Wipe(wipe)) = &mut lookup.row.play {
                wipe.soft = true;
            }
            Some(lookup)
        }
    }
}
