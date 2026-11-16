//! The motion probe catalog: one ordered list of slides shared by the PPTX and ODP decks. A slide, shape or step
//! that one format can't express is left out of that deck only, so both decks keep the same order and labels.
//!
//! Sources: the effect vocabulary and its direction convention (`docs/format.md` §5.2), ECMA-376's transitions and
//! timing, ODF 1.3's `smil:` and `anim:` attributes, and LibreOffice's tables in `probes_libreoffice.rs` (MPL-2.0):
//! the ODF presets and transitions, the PowerPoint id and sub-type mapping, the clip each preset plays and
//! PowerPoint's filter names.
//! A preset's clip is LibreOffice's; every other behaviour written into the decks is our own.
//!
//! One rule decides a clip's vocabulary params everywhere: the SMIL filter (`clip_vocab`). Preset sub-type labels
//! never do, because LibreOffice's labels for venetian blinds, random bars and diagonal squares name the opposite
//! orientation or corner to the filter they play.

use super::probes_json::Json;
pub use super::probes_libreoffice::{
    LO_FLASH_BULB_FILTER, ODF_PRESET_FILTERS, ODF_PRESETS, ODF_TRANSITIONS, PPTX_FILTERS,
    pptx_preset_id, pptx_subtype,
};

/// Which decks carry an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Formats {
    pub pptx: bool,
    pub odp: bool,
}

pub const BOTH: Formats = Formats {
    pptx: true,
    odp: true,
};
pub const PPTX: Formats = Formats {
    pptx: true,
    odp: false,
};
pub const ODP: Formats = Formats {
    pptx: false,
    odp: true,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fmt {
    Pptx,
    Odp,
}

impl Formats {
    pub fn has(self, fmt: Fmt) -> bool {
        match fmt {
            Fmt::Pptx => self.pptx,
            Fmt::Odp => self.odp,
        }
    }
}

pub struct Slide {
    pub key: String,
    pub title: String,
    pub formats: Formats,
    pub transition: Option<Trans>,
    pub shapes: Vec<Shape>,
    pub steps: Vec<Step>,
    /// A shape whose click starts the slide's interactive sequence (step indices in `interactive_steps`).
    pub trigger_shape: Option<usize>,
    /// Catalog rows this slide covers (`12.1/…` PowerPoint transitions, `12.2/…` PowerPoint build presets, `12.3/…`
    /// ODP).
    pub rows: Vec<String>,
}

pub struct Shape {
    pub paras: Vec<Para>,
    pub media: bool,
    /// A straight line with an arrowhead, across the middle of the shape's box: a box of zero height its arrowhead
    /// and stroke overflow. Its first paragraph names it (`descr`, `svg:title`) and is not drawn.
    pub line: bool,
    pub color: &'static str,
    pub formats: Formats,
}

pub struct Para {
    pub text: String,
    /// 0 for a first-level paragraph, 1 for a second-level child.
    pub level: u8,
}

/// The line a shape draws when a step colours it (`1F2937`, 1.5 pt): what a line colour animation needs to show.
pub const OUTLINE_COLOR: &str = "1F2937";

impl Slide {
    /// Whether a step of the slide colours shape `shape`'s line: the shape then draws one.
    pub fn outlined(&self, shape: usize) -> bool {
        self.steps.iter().any(|step| {
            step.shape == shape
                && step.effect.bhv.iter().any(|b| {
                    matches!(
                        b,
                        Bhv::Color("line", _)
                            | Bhv::ColorBy("line", ..)
                            | Bhv::SetColor("line", ..)
                    )
                })
        })
    }
}

impl Shape {
    pub fn label(&self) -> String {
        self.paras
            .first()
            .map(|p| p.text.clone())
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Click,
    With,
    After,
}

impl Trigger {
    pub fn vocab(self) -> &'static str {
        match self {
            Trigger::Click => "click",
            Trigger::With => "withPrevious",
            Trigger::After => "afterPrevious",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Entr,
    Exit,
    Emph,
    Path,
    Media,
    /// ODP `custom` (PPTX writes the custom effect with its own class and preset id 0).
    Custom,
}

impl Class {
    pub fn pptx(self) -> &'static str {
        match self {
            Class::Entr | Class::Custom => "entr",
            Class::Exit => "exit",
            Class::Emph => "emph",
            Class::Path => "path",
            Class::Media => "mediacall",
        }
    }

    pub fn odp(self) -> &'static str {
        match self {
            Class::Entr => "entrance",
            Class::Exit => "exit",
            Class::Emph => "emphasis",
            Class::Path => "motion-path",
            Class::Media => "media-call",
            Class::Custom => "custom",
        }
    }

    pub fn kind(self) -> &'static str {
        match self {
            Class::Entr | Class::Custom => "entrance",
            Class::Exit => "exit",
            Class::Emph => "emphasis",
            Class::Path => "motion",
            Class::Media => "media",
        }
    }

    /// The canonical step fallback when no vocabulary effect plays.
    pub fn fallback(self) -> &'static str {
        match self {
            Class::Entr | Class::Exit | Class::Custom => "dissolve",
            Class::Emph | Class::Path | Class::Media => "none",
        }
    }
}

/// A behaviour the writers render in each format's own primitives.
#[derive(Clone, Debug)]
pub enum Bhv {
    /// A clip (`animEffect@filter`, `anim:transitionFilter`). The writers add the visibility switch: visible at the
    /// start of an entrance, hidden at the end of an exit.
    Filter(Clip),
    /// From/to box expressions: PPTX `ppt_x`/`ppt_y` formulas and ODF `x`/`y` values.
    Move { pptx: [String; 4], odp: [String; 4] },
    /// Width and height from 0 (an entrance) or to 0 (an exit).
    GrowFromZero,
    /// Emphasis scale by a percentage (x, y).
    Scale(u32, u32),
    /// A pulse: scale by a percentage (x, y) over half the step, reversing on its own (`autoRev`,
    /// `smil:autoReverse`), so it ends at rest, as LibreOffice's flash bulb scales.
    Pulse(u32, u32),
    /// Rotation by degrees (positive is clockwise).
    Rotate(i32),
    /// Opacity set to a value.
    Opacity(&'static str),
    /// A colour change: `fill`, `text` or `line`, to an sRGB hex.
    Color(&'static str, &'static str),
    /// A colour flash: `fill`, `text` or `line` to an sRGB hex over half the step, reversing on its own, as
    /// LibreOffice's flicker writes it.
    ColorFlash(&'static str, &'static str),
    /// A clip whose progress follows a time filter (PPTX `tmFilter`, LibreOffice's `smil:keySplines` on the leaf):
    /// LibreOffice's flash bulb fades out to half and back.
    FilterOver(Clip, &'static str),
    /// The entrance's visibility switch lasts the step and does not hold (no `fill`), as LibreOffice's Flash Once
    /// writes it: the element shows while the switch lasts, then hides. Replaces the writers' held 1 ms switch.
    ShowWhile,
    /// A colour change by an HSL amount: `fill`, `text` or `line`; hue degrees, saturation and lightness percent.
    ColorBy(&'static str, i32, i32, i32),
    /// A colour set: `fill`, `text` or `line`, to an sRGB hex; `true` when it reverses on its own (`autoReverse` on
    /// the behaviour, as LibreOffice's Color Over by Letter writes it), over half the step.
    SetColor(&'static str, &'static str, bool),
    /// The switch that shows a fill or line a colour animation paints, as LibreOffice writes it beside the colour
    /// (`set fill=solid`, `set stroke=solid`; PPTX `fill.type` solid, `stroke.on` true): `fill` or `line`.
    PaintOn(&'static str),
    /// A font switch set: `bold`, `italic` or `underline`, on or off.
    Font(&'static str, bool),
    /// LibreOffice's bold flash: font weight normal, bold at half the duration, normal again at 60%.
    BoldPulse,
    /// A font family set (the layout moves: no track plays it).
    FontFamily(&'static str),
    /// A font size growth (the layout moves: no track plays it).
    FontSize,
    /// A motion path in slide fractions (PPTX writes it with a trailing `E`).
    Motion(String),
    /// A media command: (PPTX `cmd`, ODF `anim:command`).
    Command(&'static str, &'static str),
    /// A property no vocabulary or tier-2 primitive knows (the fail-closed probe).
    Unknown,
    /// PPTX `p:anim` with an `fmla` on its keyframes / ODF `anim:animate` with an expression in `smil:values`.
    Formula { pptx: String, odp: String },
    /// A `fmla`/value expression nested deeper than any sane parser allows (the hostile probe).
    DeepFormula(usize),
}

/// A clip in SMIL terms, with PowerPoint's name for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clip {
    /// PowerPoint's `animEffect@filter` (`PPTX_FILTERS`); `None` where PowerPoint has no filter for this clip.
    pub pptx: Option<&'static str>,
    pub ty: &'static str,
    /// Empty when the filter has no sub-type (`dissolve` in `effects.xml`).
    pub subtype: &'static str,
    pub reverse: bool,
    /// `transition="out"` / `smil:mode="out"`.
    pub out: bool,
}

impl Clip {
    /// A SMIL filter; the PowerPoint name is the first `PPTX_FILTERS` row that plays it.
    pub fn smil(ty: &'static str, subtype: &'static str, reverse: bool, out: bool) -> Clip {
        let wanted = if subtype.is_empty() {
            "default"
        } else {
            subtype
        };
        let pptx = PPTX_FILTERS
            .iter()
            .find(|&&(_, t, s, forward)| t == ty && s == wanted && forward != reverse)
            .map(|&(name, ..)| name);
        Clip {
            pptx,
            ty,
            subtype,
            reverse,
            out,
        }
    }

    /// A PowerPoint filter name, read as LibreOffice reads it.
    pub fn pptx(name: &str, out: bool) -> Option<Clip> {
        PPTX_FILTERS
            .iter()
            .find(|(n, ..)| *n == name)
            .map(|&(name, ty, subtype, forward)| Clip {
                pptx: Some(name),
                ty,
                subtype,
                reverse: !forward,
                out,
            })
    }
}

/// The clip LibreOffice's `effects.xml` plays for a preset, if its behaviours hold one.
pub fn lo_clip(ooo: &str, sub: &str) -> Option<Clip> {
    ODF_PRESET_FILTERS
        .iter()
        .find(|&&(id, s, ..)| id == ooo && s == sub)
        .map(|&(_, _, ty, subtype, reverse, out)| Clip::smil(ty, subtype, reverse, out))
}

/// The vocabulary clip a SMIL filter plays: the one mapper for build
/// presets, tier 2 and transitions. Wipe-like clips name the way their edge travels whatever the mode; a slide clip
/// moves the content away from its `from` side when it enters and towards it when it leaves (`out`); a split's or an
/// iris's in/out names the entrance it plays, backwards when it leaves, as LibreOffice's engine plays it.
pub fn clip_vocab(
    ty: &str,
    subtype: &str,
    reverse: bool,
    out: bool,
) -> Option<(&'static str, Json)> {
    let flip = |forward: &'static str, back: &'static str| if reverse { back } else { forward };
    let in_out = |grows: bool| if grows { "out" } else { "in" };
    let p = Json::obj;
    Some(match (ty, subtype) {
        ("barWipe", "leftToRight") => ("wipe", p().with("dir", flip("r", "l"))),
        ("barWipe", "topToBottom") => ("wipe", p().with("dir", flip("d", "u"))),
        ("slideWipe", from) => {
            let side = match from {
                "fromLeft" => "l",
                "fromRight" => "r",
                "fromTop" => "u",
                "fromBottom" => "d",
                _ => return None,
            };
            let dir = if out { side } else { opposite(side) };
            ("flyIn", p().with("dir", dir).with("distance", "slight"))
        }
        ("barnDoorWipe", "vertical" | "horizontal") => (
            "split",
            p().with("orient", orient_word(subtype)?)
                .with("inOut", in_out(!reverse)),
        ),
        ("blindsWipe", "vertical" | "horizontal") => {
            ("blinds", p().with("orient", orient_word(subtype)?))
        }
        ("randomBarWipe", "vertical" | "horizontal") => {
            ("randomBars", p().with("orient", orient_word(subtype)?))
        }
        ("checkerBoardWipe", "across") => ("checker", p().with("dir", flip("r", "l"))),
        ("checkerBoardWipe", "down") => ("checker", p().with("dir", flip("d", "u"))),
        ("fourBoxWipe", "cornersIn" | "cornersOut") => (
            "iris",
            p().with("shape", "plus")
                .with("inOut", in_out((subtype == "cornersOut") != reverse)),
        ),
        ("irisWipe", "diamond") => (
            "iris",
            p().with("shape", "diamond").with("inOut", in_out(!reverse)),
        ),
        ("irisWipe", "rectangle") => (
            "iris",
            p().with("shape", "box").with("inOut", in_out(!reverse)),
        ),
        ("ellipseWipe", "circle" | "horizontal" | "vertical") => (
            "iris",
            p().with("shape", "circle").with("inOut", in_out(!reverse)),
        ),
        ("pinWheelWipe", blade) => {
            let spokes: u32 = match blade {
                "oneBlade" => 1,
                "twoBladeVertical" => 2,
                "threeBlade" => 3,
                "fourBlade" => 4,
                "eightBlade" => 8,
                _ => return None,
            };
            ("wheel", p().with("spokes", spokes))
        }
        ("fanWipe", "centerTop") => ("wedge", p()),
        ("clockWipe", "clockwiseTwelve") => ("clock", p().with("cw", !reverse).with("from", 12u32)),
        ("waterfallWipe", "horizontalRight") => ("strips", p().with("corner", flip("ld", "ru"))),
        ("waterfallWipe", "horizontalLeft") => ("strips", p().with("corner", flip("rd", "lu"))),
        ("fade", "crossfade") => ("fade", p()),
        ("dissolve", "" | "default") => ("dissolve", p()),
        _ => return None,
    })
}

fn orient_word(subtype: &str) -> Option<&'static str> {
    match subtype {
        "horizontal" => Some("horizontal"),
        "vertical" => Some("vertical"),
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub struct Effect {
    pub class: Class,
    /// ODF `presentation:preset-id`; `None` writes no preset attributes.
    pub ooo: Option<String>,
    pub sub: Option<String>,
    /// PowerPoint `presetID`; `None` means the preset is not authorable in PPTX.
    pub pptx_id: Option<u32>,
    pub pptx_sub: u32,
    pub bhv: Vec<Bhv>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Word,
    Letter,
    Paragraph,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interval {
    Ms(u32),
    /// PPTX `tmPct`, in 1/1000 % of the duration.
    Pct(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Iterate {
    pub unit: Unit,
    pub interval: Interval,
    pub backwards: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    /// In thousandths (PPTX `repeatCount` units).
    Count(u32),
    UntilEndOfSlide,
    UntilNextClick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum After {
    Hide,
    Dim(&'static str),
}

/// What the vocabulary makes of a step (format-independent).
#[derive(Clone, Debug)]
pub enum Vocab {
    Effect {
        name: &'static str,
        params: Json,
        approximate: bool,
        /// A delivery the effect implies (colored lettering is by character).
        delivery: Option<&'static str>,
    },
    /// Tier 2: the behaviours play; `clip` is what `animEffect@filter` / `transitionFilter` decides.
    Tier2 { clip: Option<(&'static str, Json)> },
    /// No vocabulary entry: the step plays its fallback and the frontend reports it.
    Fallback,
    /// Not in the IR (an interactive sequence): the frontend drops it with a diagnostic.
    Dropped,
    /// Hostile timing the step model can't express (a condition cycle, nesting past the depth limit): the frontend
    /// stays bounded (no panic, no unbounded loop), drops the step and reports it.
    Bounded,
}

/// Timing values written as given instead of from the step's numbers (the malformed-timing probe).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Raw {
    /// PPTX behaviour `dur` (ms) / ODF `smil:dur` (a clock value).
    pub pptx_dur: Option<&'static str>,
    pub odp_dur: Option<&'static str>,
    /// PPTX effect `cond delay` (ms) / ODF effect `smil:begin`.
    pub pptx_delay: Option<&'static str>,
    pub odp_delay: Option<&'static str>,
}

impl Raw {
    pub fn dur(&self, fmt: Fmt) -> Option<&'static str> {
        match fmt {
            Fmt::Pptx => self.pptx_dur,
            Fmt::Odp => self.odp_dur,
        }
    }

    pub fn delay(&self, fmt: Fmt) -> Option<&'static str> {
        match fmt {
            Fmt::Pptx => self.pptx_delay,
            Fmt::Odp => self.odp_delay,
        }
    }
}

pub struct Step {
    pub shape: usize,
    pub paragraphs: Option<(u32, u32)>,
    pub background: bool,
    pub trigger: Trigger,
    pub delay_ms: u32,
    pub dur_ms: u32,
    pub effect: Effect,
    pub iterate: Option<Iterate>,
    pub repeat: Option<Repeat>,
    pub auto_rev: bool,
    /// PPTX `accel`/`decel` in 1/1000 % (50000 = 50%).
    pub accel: Option<u32>,
    pub decel: Option<u32>,
    pub after: Option<After>,
    pub formats: Formats,
    pub vocab: Vocab,
    /// Part of the slide's interactive sequence rather than the main sequence.
    pub interactive: bool,
    /// A start condition naming a step's time node instead of a delay: (step index, `true` for its end, `false`
    /// for its begin). The hostile probe uses it for a self-reference and a two-node cycle.
    pub begin_on: Option<(usize, bool)>,
    /// Extra container levels around the effect (the hostile nesting probe).
    pub nest: u32,
    pub raw: Raw,
}

impl Step {
    fn new(shape: usize, effect: Effect, vocab: Vocab, formats: Formats) -> Step {
        Step {
            shape,
            paragraphs: None,
            background: false,
            trigger: Trigger::Click,
            delay_ms: 0,
            dur_ms: 500,
            effect,
            iterate: None,
            repeat: None,
            auto_rev: false,
            accel: None,
            decel: None,
            after: None,
            formats,
            vocab,
            interactive: false,
            begin_on: None,
            nest: 0,
            raw: Raw::default(),
        }
    }

    /// The effect's length on the timeline: repeats and auto-reverse included (an indefinite repeat counts once).
    pub fn span_ms(&self) -> u32 {
        let reps = match self.repeat {
            Some(Repeat::Count(thousandths)) => thousandths / 1000,
            _ => 1,
        };
        self.dur_ms
            .saturating_mul(reps.max(1))
            .saturating_mul(if self.auto_rev { 2 } else { 1 })
    }

    /// Whether this is an appear/disappear: a lone visibility switch.
    pub fn is_switch(&self) -> bool {
        self.effect.bhv.is_empty() && matches!(self.effect.class, Class::Entr | Class::Exit)
    }

    /// Whether the entrance shows its target only while its visibility switch lasts ([`Bhv::ShowWhile`]).
    pub fn shows_while(&self) -> bool {
        self.effect.bhv.iter().any(|b| matches!(b, Bhv::ShowWhile))
    }
}

/// A transition, in each format's own terms, with its vocabulary expectation.
pub struct Trans {
    pub pptx: Option<PTrans>,
    pub odp: Option<OTrans>,
    /// `Some((effect, params, approximate))`, or `None` when the vocabulary has no entry (the fallback plays).
    pub vocab: Option<(&'static str, Json, bool)>,
    /// ODP params where they differ from PPTX's (PowerPoint's wipe is soft, ODF's barWipe is not).
    pub odp_params: Option<Json>,
    pub fallback: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ext {
    Ecma,
    P14,
    P15,
    P159,
}

pub struct PTrans {
    pub ext: Ext,
    pub elem: String,
    pub attrs: Vec<(String, String)>,
    pub spd: Option<&'static str>,
    pub p14_dur: Option<u32>,
    /// Write an ECMA transition inside `mc:AlternateContent` (with `p14:dur`), as PowerPoint 2010+ does.
    pub alternate: bool,
    pub adv_click: Option<bool>,
    pub adv_tm: Option<u32>,
    /// `advTm` written as given (the malformed-timing probe).
    pub adv_tm_raw: Option<&'static str>,
}

impl PTrans {
    pub fn native(&self) -> String {
        match self.ext {
            Ext::P15 => {
                let prst = self
                    .attrs
                    .iter()
                    .find(|(k, _)| k == "prst")
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("");
                format!("pptx:prstTrans/{prst}")
            }
            _ => format!("pptx:{}", self.elem),
        }
    }

    pub fn duration_ms(&self) -> u32 {
        if let Some(dur) = self.p14_dur {
            return dur;
        }
        match self.spd {
            Some("slow") => 1000,
            Some("med") => 750,
            _ => 500,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OAdv {
    Manual,
    Automatic(Option<u32>),
    SemiAutomatic,
}

pub struct OTrans {
    /// `smil:type`, `smil:subtype`, reverse, `smil:fadeColor`.
    pub smil: Option<(String, String, bool, Option<&'static str>)>,
    /// ODF 1.0 `presentation:transition-style`, written when `smil` is `None`.
    pub legacy: Option<&'static str>,
    pub speed: &'static str,
    /// The timing root's `anim:transitionFilter smil:dur` (exact; wins over the speed).
    pub filter_dur: Option<u32>,
    pub advance: OAdv,
    /// `presentation:duration` written as given (the malformed-timing probe).
    pub duration_raw: Option<&'static str>,
}

impl OTrans {
    pub fn native(&self) -> String {
        match (&self.smil, self.legacy) {
            (Some((ty, sub, reverse, _)), _) => {
                let sub = if sub.is_empty() {
                    String::new()
                } else {
                    format!("/{sub}")
                };
                format!("odp:{ty}{sub}{}", if *reverse { "/reverse" } else { "" })
            }
            (None, Some(style)) => format!("odp:legacy/{style}"),
            _ => "odp:none".to_owned(),
        }
    }

    pub fn duration_ms(&self) -> u32 {
        if let Some(dur) = self.filter_dur {
            return dur;
        }
        match self.speed {
            "slow" => 2000,
            "fast" => 500,
            _ => 1000,
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Directions (`dir` is the direction the content moves, in OOXML tokens).

/// The token of a side named by a preset sub-type word.
fn side_token(word: &str) -> &'static str {
    match word {
        "top" => "u",
        "bottom" => "d",
        "left" => "l",
        "right" => "r",
        "top-left" => "lu",
        "top-right" => "ru",
        "bottom-left" => "ld",
        "bottom-right" => "rd",
        _ => "",
    }
}

fn opposite(token: &str) -> &'static str {
    match token {
        "u" => "d",
        "d" => "u",
        "l" => "r",
        "r" => "l",
        "lu" => "rd",
        "rd" => "lu",
        "ru" => "ld",
        "ld" => "ru",
        _ => "",
    }
}

/// The content-moves direction of a sub-type: an entrance "from-X" moves away from X; an exit names where it goes
/// ("to-X", and "from-X" for the exits LibreOffice names that way: peek-out, the bit flags of `presetSubtype`); a
/// wipe moves its edge away from the side it starts at, entrance or exit.
fn sub_dir(class: Class, name: &str, sub: &str) -> &'static str {
    let (prefix, side) = match sub.split_once('-') {
        Some((p, s)) => (p, s),
        None => return "",
    };
    let token = side_token(side);
    let wipe = name == "wipe";
    match (class, prefix, wipe) {
        (_, "from", true) => opposite(token),
        (Class::Exit, _, false) => token,
        (_, "from", false) => opposite(token),
        _ => token,
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Vocabulary expectations for the presets.

fn params() -> Json {
    Json::obj()
}

fn effect(name: &'static str, params: Json) -> Vocab {
    Vocab::Effect {
        name,
        params,
        approximate: false,
        delivery: None,
    }
}

fn approx(name: &'static str, params: Json) -> Vocab {
    Vocab::Effect {
        name,
        params,
        approximate: true,
        delivery: None,
    }
}

fn num(text: &str) -> Json {
    Json::Num(text.to_owned())
}

/// The colour every colour-change probe uses.
pub const PROBE_COLOR: &str = "E0442E";

fn preset_name(ooo: &str) -> &str {
    ooo.strip_prefix("ooo-entrance-")
        .or_else(|| ooo.strip_prefix("ooo-exit-"))
        .or_else(|| ooo.strip_prefix("ooo-emphasis-"))
        .unwrap_or(ooo)
}

pub fn preset_vocab(class: Class, ooo: &str, sub: &str) -> Vocab {
    let name = preset_name(ooo);
    match class {
        Class::Emph => emphasis_vocab(name),
        Class::Path => effect("path", params()),
        Class::Media => match name {
            "ooo-media-start" => effect("play", params().with("from", 0u32)),
            "ooo-media-stop" => effect("stop", params()),
            _ => effect("toggle", params()),
        },
        _ => build_vocab(class, ooo, sub),
    }
}

/// The step expectation of a clip preset: the effect the preset's own clip plays (`clip_vocab`), or the fallback
/// when the preset plays no clip the vocabulary knows.
fn clip_effect(ooo: &str, sub: &str, name: &'static str, approximate: bool) -> Vocab {
    let clip = lo_clip(ooo, sub).and_then(|c| clip_vocab(c.ty, c.subtype, c.reverse, c.out));
    match clip {
        Some((clip_name, params)) if clip_name == name => Vocab::Effect {
            name,
            params,
            approximate,
            delivery: None,
        },
        // peek moves like a slide clip: it takes the slide clip's direction
        Some(("flyIn", params)) if name == "peek" => effect(
            "peek",
            Json::obj().with_opt("dir", params.get("dir").cloned()),
        ),
        _ => Vocab::Fallback,
    }
}

fn build_vocab(class: Class, ooo: &str, sub: &str) -> Vocab {
    let name = preset_name(ooo);
    let dir = sub_dir(class, name, sub);
    let exit = class == Class::Exit;
    match name {
        "appear" | "disappear" => effect("appear", params()),
        "fly-in" | "fly-out" => effect(
            "flyIn",
            params().with("dir", dir).with("distance", "offSlide"),
        ),
        "venetian-blinds" => clip_effect(ooo, sub, "blinds", false),
        "box" | "circle" | "diamond" | "plus" => clip_effect(ooo, sub, "iris", false),
        "oval" => clip_effect(ooo, sub, "iris", true),
        "checkerboard" => clip_effect(ooo, sub, "checker", false),
        "fly-in-slow" | "crawl-out" => effect("peek", params().with("dir", dir).with("slow", true)),
        "dissolve-in" | "dissolve" => clip_effect(ooo, sub, "dissolve", false),
        // `effects.xml`'s entrance is a visibility switch that does not hold: an appear, hidden after the step.
        "flash-once" if !exit => effect("appear", params()),
        "flash-once" => effect("blink", params().with("repeat", 1u32)),
        "peek-in" | "peek-out" => clip_effect(ooo, sub, "peek", false),
        "random-bars" => clip_effect(ooo, sub, "randomBars", false),
        "random" => effect("random", params()),
        "split" => clip_effect(ooo, sub, "split", false),
        "diagonal-squares" => clip_effect(ooo, sub, "strips", false),
        "wedge" => clip_effect(ooo, sub, "wedge", false),
        "clock-wipe" => clip_effect(ooo, sub, "clock", false),
        "wheel" => clip_effect(ooo, sub, "wheel", false),
        "snake-wipe" | "spiral-wipe" => Vocab::Fallback,
        "wipe" => clip_effect(ooo, sub, "wipe", false),
        "expand" | "contract" => effect("expand", params()),
        "fade-in" | "fade-out" => effect("fade", params()),
        "fade-in-and-swivel" | "fade-out-and-swivel" => approx("swivel", params()),
        "fade-in-and-zoom" | "fade-out-and-zoom" => effect("zoom", params().with("fade", true)),
        "ascend" | "descend" | "rise-up" | "sink-down" | "float" => {
            let (dir, approximate) = float_dir(name, exit);
            let params = params().with("dir", dir);
            if approximate {
                approx("floatIn", params)
            } else {
                effect("floatIn", params)
            }
        }
        "center-revolve" | "pinwheel" | "spiral-in" | "spiral-out" => approx("spinIn", params()),
        "colored-lettering" => Vocab::Effect {
            name: "appear",
            params: params(),
            approximate: true,
            delivery: Some("byChar"),
        },
        // LibreOffice's compress narrows the width onto the element: horizontal (an axis is what the source
        // animates, never a sub-type's word).
        "compress" => effect("compress", params().with("axis", "horizontal")),
        // LibreOffice's ease-in slides and wipes in, glide stretches and slides in: fadeAndMove (translate 10%,
        // scale and fade) approximates them.
        "ease-in" | "ease-out" | "glide" => approx(
            "fadeAndMove",
            params().with("dir", if exit { "l" } else { "r" }),
        ),
        "turn-and-grow" => effect("growTurn", params()),
        "spin-in" | "spin-out" => effect("spinIn", params()),
        "stretchy" | "collapse" => {
            let axis = match sub {
                "" => None,
                "across" | "from-left" | "from-right" => Some("horizontal"),
                "downward" | "from-top" | "from-bottom" => Some("vertical"),
                _ => Some("both"),
            };
            // stretch has an axis only: a sub-type that names the side it grows from is approximate
            let params = params().with_opt("axis", axis);
            if sub.starts_with("from-") {
                approx("stretch", params)
            } else {
                effect("stretch", params)
            }
        }
        // LibreOffice's unfold slides each letter in and fades it: stretch approximates it.
        "unfold" => approx("stretch", params()),
        "zoom" => {
            // An exit names the entrance it reverses: exit `out` shrinks to nothing, the zoom-in
            // entrance reversed. LibreOffice's raw exit numbers: 544 shrinks to the slide centre (in from the screen
            // centre, reversed), 20 grows away below the slide (out from the screen centre, reversed). Out from the
            // screen centre starts large at the slide's bottom centre: no zoom origin plays that path.
            let named = |in_out: &'static str| -> &'static str {
                match (exit, in_out) {
                    (true, "in") => "out",
                    (true, _) => "in",
                    (false, _) => in_out,
                }
            };
            let (in_out, origin, slight, approximate) = match sub {
                "in" => (named("in"), "object", false, false),
                "out" => (named("out"), "object", false, false),
                "in-slightly" => (named("in"), "object", true, false),
                "out-slightly" => (named("out"), "object", true, false),
                "in-from-screen-center" => (named("in"), "slideCenter", false, false),
                "out-from-screen-center" => (named("out"), "slideCenter", false, true),
                "544" => ("in", "slideCenter", false, false),
                "20" => ("out", "slideCenter", false, true),
                _ => return Vocab::Fallback,
            };
            let params = params()
                .with("inOut", in_out)
                .with("origin", origin)
                .with("slight", slight);
            if approximate {
                approx("zoom", params)
            } else {
                effect("zoom", params)
            }
        }
        "boomerang" => effect("boomerang", params()),
        "bounce" => effect("bounce", params()),
        "movie-credits" => effect("credits", params()),
        "flip" => effect("flip", params()),
        "magnify" => approx("zoom", params().with("inOut", "in").with("overshoot", true)),
        "sling" => effect("sling", params()),
        "falling-in" => effect("drop", params()),
        "swish" => effect("swish", params()),
        // LibreOffice's swivel `vertical` animates the width (a turn about the vertical axis), so its axis is
        // horizontal; `horizontal` animates the height.
        "swivel" => effect(
            "swivel",
            params().with(
                "axis",
                if sub == "horizontal" {
                    "vertical"
                } else {
                    "horizontal"
                },
            ),
        ),
        "whip" => effect("whip", params()),
        "breaks" | "thread" | "curve-up" | "curve-down" | "fold" => Vocab::Tier2 { clip: None },
        _ if name.starts_with("libo-physics") => Vocab::Tier2 { clip: None },
        _ => Vocab::Fallback,
    }
}

/// The content-moves direction of the vertical floats, and whether `floatIn` only approximates them: LibreOffice's
/// Float descends from the top right while it turns (rises to the top right when it leaves), Rise Up overshoots and
/// Sink Down lifts before it falls.
fn float_dir(name: &str, exit: bool) -> (&'static str, bool) {
    match (name, exit) {
        ("ascend" | "rise-up", false) | ("ascend", true) => ("u", name == "rise-up"),
        ("float", false) => ("d", true),
        ("float", true) => ("u", true),
        _ => ("d", name == "sink-down"),
    }
}

fn emphasis_vocab(name: &str) -> Vocab {
    let color = format!("#{PROBE_COLOR}");
    match name {
        "fill-color" => effect(
            "colorChange",
            params().with("property", "fill").with("to", color),
        ),
        "font-color" => effect(
            "colorChange",
            params().with("property", "text").with("to", color),
        ),
        "line-color" => effect(
            "colorChange",
            params().with("property", "line").with("to", color),
        ),
        "font-size" => approx(
            "growShrink",
            params().with("scale", vec![num("1.5"), num("1.5")]),
        ),
        "grow-and-shrink" => effect(
            "growShrink",
            params().with("scale", vec![num("1.5"), num("1.5")]),
        ),
        "spin" => effect("spin", params().with("deg", 360u32).with("cw", true)),
        "transparency" => effect("transparency", params().with("alpha", num("0.5"))),
        "bold-flash" => effect("boldFlash", params()),
        // LibreOffice's blast squashes, grows and recolours; flash bulb fades while it pulses; flicker flashes the
        // colours to white: pulse and blink approximate them.
        "blast" | "flash-bulb" => approx("pulse", params()),
        "flicker" => approx("blink", params()),
        "blink" => effect("blink", params()),
        "teeter" => effect("teeter", params()),
        "wave" => effect("wave", params()),
        _ => Vocab::Tier2 { clip: None },
    }
}

/// What a preset's step plays: an approximate entry yields to the behaviours the preset is written with (both
/// formats carry them), unless they compile to nothing in tier 2 (a media command alone), when the approximation
/// plays and is reported. A font family or size change moves the layout: the step plays its fallback.
pub fn plays(vocab: Vocab, bhv: &[Bhv]) -> Vocab {
    if bhv
        .iter()
        .any(|b| matches!(b, Bhv::FontFamily(_) | Bhv::FontSize))
    {
        return Vocab::Fallback;
    }
    let compiles = bhv.iter().any(|b| !matches!(b, Bhv::Command(..)));
    match vocab {
        Vocab::Effect {
            approximate: true, ..
        } if compiles => with_tier2_clip(Vocab::Tier2 { clip: None }, bhv),
        other => other,
    }
}

/// A tier-2 step's clip: what the first clip among its behaviours plays.
fn with_tier2_clip(vocab: Vocab, bhv: &[Bhv]) -> Vocab {
    match vocab {
        Vocab::Tier2 { clip: None } => Vocab::Tier2 {
            clip: bhv.iter().find_map(|b| match b {
                Bhv::Filter(c) | Bhv::FilterOver(c, _) => {
                    clip_vocab(c.ty, c.subtype, c.reverse, c.out)
                }
                _ => None,
            }),
        },
        other => other,
    }
}

/// The behaviours a preset is written with: LibreOffice's clip where `effects.xml` gives the preset one, and our own
/// motion (the frontends key on the preset; tier 2 sees these).
fn preset_bhv(class: Class, ooo: &str, sub: &str) -> Vec<Bhv> {
    let name = preset_name(ooo);
    let exit = class == Class::Exit;
    let mut out = match class {
        Class::Emph => match name {
            // The colour and font emphases animate what LibreOffice's `effects.xml` animates, in our colour.
            "fill-color" => vec![Bhv::Color("fill", PROBE_COLOR)],
            "font-color" => vec![Bhv::Color("text", PROBE_COLOR)],
            "line-color" => vec![Bhv::Color("line", PROBE_COLOR)],
            "color-blend" => vec![
                Bhv::Color("text", PROBE_COLOR),
                Bhv::Color("fill", PROBE_COLOR),
            ],
            "color-over-by-word" | "color-over-by-letter" => {
                let reverse = name == "color-over-by-letter";
                vec![
                    Bhv::SetColor("text", PROBE_COLOR, reverse),
                    Bhv::SetColor("fill", PROBE_COLOR, reverse),
                ]
            }
            "complementary-color" => shift_all(120, 0, 0),
            "complementary-color-2" => shift_all(-120, 0, 0),
            "contrasting-color" => shift_all(180, 0, 0),
            "darken" => shift_all(0, -12, -25),
            "desaturate" => shift_all(0, -70, 0),
            "lighten" => shift_all(0, 12, 25),
            "grow-with-color" => vec![
                Bhv::Color("text", PROBE_COLOR),
                Bhv::Color("fill", PROBE_COLOR),
                Bhv::FontSize,
            ],
            "font" => vec![Bhv::FontFamily("Times New Roman")],
            "font-style" => vec![
                Bhv::Font("italic", false),
                Bhv::Font("bold", true),
                Bhv::Font("underline", false),
            ],
            "bold-reveal" => vec![Bhv::Font("bold", true)],
            "reveal-underline" => vec![Bhv::Font("underline", true)],
            "style-emphasis" => vec![
                Bhv::SetColor("text", PROBE_COLOR, false),
                Bhv::Font("italic", true),
                Bhv::Font("bold", true),
                Bhv::Font("underline", true),
            ],
            "bold-flash" => vec![Bhv::BoldPulse],
            "grow-and-shrink" => vec![Bhv::Scale(150_000, 150_000)],
            "font-size" => vec![Bhv::FontSize],
            "spin" => vec![Bhv::Rotate(360)],
            "transparency" => vec![Bhv::Opacity("0.5")],
            // LibreOffice's flicker flashes the text and fill to white and back.
            "flicker" => vec![
                Bhv::ColorFlash("text", "FFFFFF"),
                Bhv::ColorFlash("fill", "FFFFFF"),
            ],
            "blink" => vec![Bhv::Opacity("0")],
            "teeter" | "wave" => vec![Bhv::Rotate(5)],
            // LibreOffice's flash bulb pulses while it fades out to half and back.
            "flash-bulb" => {
                let fade = Clip::smil("fade", "crossfade", false, true);
                vec![
                    Bhv::Pulse(105_000, 105_000),
                    Bhv::FilterOver(fade, LO_FLASH_BULB_FILTER),
                ]
            }
            // Our own held scale: a tier-2 transform emphasis that keeps its end.
            _ => vec![Bhv::Scale(110_000, 110_000)],
        },
        Class::Path => return vec![Bhv::Motion(motion_path(ooo))],
        Class::Media => {
            return vec![match name {
                "ooo-media-start" => Bhv::Command("playFrom(0.0)", "play"),
                "ooo-media-stop" => Bhv::Command("stop", "stop"),
                _ => Bhv::Command("togglePause", "toggle-pause"),
            }];
        }
        _ => entrance_motion(class, name, sub),
    };
    // LibreOffice shows the fill each fill colour animation paints, and the line Change Line Color paints.
    let paints = |property: &str| {
        out.iter().any(|b| {
            matches!(b, Bhv::Color(p, _) | Bhv::ColorBy(p, ..) | Bhv::SetColor(p, ..) | Bhv::ColorFlash(p, _) if *p == property)
        })
    };
    if paints("fill") {
        out.push(Bhv::PaintOn("fill"));
    }
    if name == "line-color" {
        out.push(Bhv::PaintOn("line"));
    }
    match lo_clip(ooo, sub) {
        // The flash bulb's fade is its time-filtered clip, above.
        Some(_) if out.iter().any(|b| matches!(b, Bhv::FilterOver(..))) => {}
        Some(clip) => out.push(Bhv::Filter(clip)),
        None if out.is_empty() && !matches!(name, "appear" | "disappear") => {
            out.push(Bhv::Filter(Clip::smil("fade", "crossfade", false, exit)))
        }
        None => {}
    }
    out
}

/// One HSL shift on the text, the fill and the line, as LibreOffice's darken, lighten and hue presets write it.
fn shift_all(h: i32, s: i32, l: i32) -> Vec<Bhv> {
    ["text", "fill", "line"]
        .into_iter()
        .map(|property| Bhv::ColorBy(property, h, s, l))
        .collect()
}

/// Our own motion for an entrance or exit preset (its clip comes from `lo_clip`).
fn entrance_motion(class: Class, name: &str, sub: &str) -> Vec<Bhv> {
    let exit = class == Class::Exit;
    if name == "flash-once" && !exit {
        return vec![Bhv::ShowWhile];
    }
    let side = |s: &str| -> Option<&'static str> {
        let bare = s.strip_prefix("from-").or_else(|| s.strip_prefix("to-"))?;
        Some(side_token(bare))
    };
    match name {
        "fly-in" | "fly-out" | "fly-in-slow" | "crawl-out" => {
            vec![fly(side(sub).unwrap_or("d"), exit)]
        }
        "ascend" | "descend" | "rise-up" | "sink-down" | "float" => {
            vec![drift(float_dir(name, exit).0, exit)]
        }
        "ease-in" | "ease-out" | "glide" => vec![drift(if exit { "l" } else { "r" }, exit)],
        "movie-credits" | "curve-up" | "curve-down" | "bounce" | "falling-in" | "boomerang"
        | "sling" | "whip" | "swish" => vec![fly("d", exit)],
        "zoom"
        | "magnify"
        | "expand"
        | "contract"
        | "compress"
        | "stretchy"
        | "collapse"
        | "unfold"
        | "swivel"
        | "flip"
        | "fold"
        | "fade-in-and-zoom"
        | "fade-out-and-zoom"
        | "fade-in-and-swivel"
        | "fade-out-and-swivel" => vec![Bhv::GrowFromZero],
        "spin-in" | "spin-out" | "pinwheel" | "turn-and-grow" | "center-revolve" | "spiral-in"
        | "spiral-out" => vec![
            Bhv::GrowFromZero,
            Bhv::Rotate(if exit { 360 } else { -360 }),
        ],
        _ => vec![],
    }
}

/// A fly from (entrance) or to (exit) a slide edge.
fn fly(side: &str, exit: bool) -> Bhv {
    let (px, py, ox, oy) = match side {
        "u" => ("#ppt_x", "0-#ppt_h/2", "x", "0-height/2"),
        "d" => ("#ppt_x", "1+#ppt_h/2", "x", "1+height/2"),
        "l" => ("0-#ppt_w/2", "#ppt_y", "0-width/2", "y"),
        "r" => ("1+#ppt_w/2", "#ppt_y", "1+width/2", "y"),
        "lu" => ("0-#ppt_w/2", "0-#ppt_h/2", "0-width/2", "0-height/2"),
        "ru" => ("1+#ppt_w/2", "0-#ppt_h/2", "1+width/2", "0-height/2"),
        "ld" => ("0-#ppt_w/2", "1+#ppt_h/2", "0-width/2", "1+height/2"),
        _ => ("1+#ppt_w/2", "1+#ppt_h/2", "1+width/2", "1+height/2"),
    };
    move_between([px, py], ["#ppt_x", "#ppt_y"], [ox, oy], ["x", "y"], exit)
}

/// A short travel of 10% of the slide in the content-moves direction `dir` (float, ascend, ease, glide): an entrance
/// starts that far behind its place, an exit ends that far ahead.
fn drift(dir: &str, exit: bool) -> Bhv {
    let behind = if exit { dir } else { opposite(dir) };
    let (px, py, ox, oy) = match behind {
        "u" => ("#ppt_x", "#ppt_y-0.1", "x", "y-0.1"),
        "d" => ("#ppt_x", "#ppt_y+0.1", "x", "y+0.1"),
        "l" => ("#ppt_x-0.1", "#ppt_y", "x-0.1", "y"),
        _ => ("#ppt_x+0.1", "#ppt_y", "x+0.1", "y"),
    };
    move_between([px, py], ["#ppt_x", "#ppt_y"], [ox, oy], ["x", "y"], exit)
}

fn move_between(
    pptx_away: [&str; 2],
    pptx_home: [&str; 2],
    odp_away: [&str; 2],
    odp_home: [&str; 2],
    exit: bool,
) -> Bhv {
    let (pf, pt, of, ot) = if exit {
        (pptx_home, pptx_away, odp_home, odp_away)
    } else {
        (pptx_away, pptx_home, odp_away, odp_home)
    };
    Bhv::Move {
        pptx: [pf[0].into(), pf[1].into(), pt[0].into(), pt[1].into()],
        odp: [of[0].into(), of[1].into(), ot[0].into(), ot[1].into()],
    }
}

/// Our own path for a motion-path preset, in slide fractions on a 0.0125 grid (16 px across, 9 px down at
/// 1280×720), so the expected pixel path is exact.
pub fn motion_path(ooo: &str) -> String {
    let name = ooo.strip_prefix("ooo-motionpath-").unwrap_or(ooo);
    let k = |n: i32| -> String {
        let value = f64::from(n) * 0.0125;
        let text = format!("{value:.4}");
        let text = text.trim_end_matches('0').trim_end_matches('.').to_owned();
        if text == "-0" { "0".to_owned() } else { text }
    };
    let line = |x: i32, y: i32| format!("M 0 0 L {} {}", k(x), k(y));
    match name {
        "up" => line(0, -20),
        "down" => line(0, 20),
        "left" => line(-20, 0),
        "right" => line(20, 0),
        "diagonal-down-right" => line(20, 20),
        "diagonal-up-right" => line(20, -20),
        "custom" => format!("M 0 0 C {} 0 {} {} {} {}", k(8), k(16), k(8), k(20), k(20)),
        _ => {
            // a closed polygon whose vertex count is fixed by the name, so paths differ per preset
            let sides = 3 + (name.bytes().map(u32::from).sum::<u32>() % 5) as i32;
            let points = [(8, 0), (8, 8), (0, 8), (-8, 8), (-8, 0), (-8, -8), (0, -8)];
            let mut path = "M 0 0".to_owned();
            for &(x, y) in points.iter().take(sides as usize) {
                path.push_str(&format!(" L {} {}", k(x), k(y)));
            }
            path.push_str(" Z");
            path
        }
    }
}

/// The expected pixel path (`path{d}`): slide fractions × 1280 / 720, relative to the element.
pub fn path_px(path: &str) -> String {
    let mut out = Vec::new();
    let mut axis = 0;
    for token in path.split_whitespace() {
        if token.chars().all(|c| c.is_ascii_alphabetic()) {
            if token != "E" {
                out.push(token.to_owned());
            }
            axis = 0;
            continue;
        }
        let value: f64 = token.parse().unwrap_or(0.0);
        let px = if axis % 2 == 0 {
            value * 1280.0
        } else {
            value * 720.0
        };
        out.push(format!("{}", px.round() as i64));
        axis += 1;
    }
    out.join(" ")
}

// ---------------------------------------------------------------------------------------------------------------
// The deck.

const COLORS: [&str; 6] = ["2F6FDF", "1E9E6A", "C2410C", "7C3AED", "B91C1C", "0E7490"];

fn shape(text: impl Into<String>, color_index: usize, formats: Formats) -> Shape {
    Shape {
        paras: vec![Para {
            text: text.into(),
            level: 0,
        }],
        media: false,
        line: false,
        color: COLORS[color_index % COLORS.len()],
        formats,
    }
}

fn slide(key: impl Into<String>, title: impl Into<String>, formats: Formats) -> Slide {
    Slide {
        key: key.into(),
        title: title.into(),
        formats,
        transition: None,
        shapes: Vec::new(),
        steps: Vec::new(),
        trigger_shape: None,
        rows: Vec::new(),
    }
}

pub fn deck() -> Vec<Slide> {
    let mut slides = Vec::new();
    let mut title = slide(
        "title",
        "slidock motion probe: transitions, builds and timing (our own content)",
        BOTH,
    );
    title.shapes.push(shape(
        "Every slide names the effect it carries; motion-probe.expect.json lists what each frontend must read.",
        0,
        BOTH,
    ));
    slides.push(title);
    transitions(&mut slides);
    timing_slides(&mut slides);
    preset_slides(&mut slides);
    feature_slides(&mut slides);
    slides
}

// ---- transitions -----------------------------------------------------------------------------------------------

fn pt(elem: &str, attrs: &[(&str, &str)]) -> PTrans {
    PTrans {
        ext: Ext::Ecma,
        elem: elem.to_owned(),
        attrs: attrs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        spd: Some("med"),
        p14_dur: None,
        alternate: false,
        adv_click: None,
        adv_tm: None,
        adv_tm_raw: None,
    }
}

fn pt_ext(ext: Ext, elem: &str, attrs: &[(&str, &str)]) -> PTrans {
    PTrans {
        ext,
        p14_dur: Some(750),
        ..pt(elem, attrs)
    }
}

fn ot(ty: &str, subtype: &str, reverse: bool) -> OTrans {
    OTrans {
        smil: Some((ty.to_owned(), subtype.to_owned(), reverse, None)),
        legacy: None,
        speed: "medium",
        filter_dur: Some(750),
        advance: OAdv::Manual,
        duration_raw: None,
    }
}

/// ODF `slideWipe`/`pushWipe` sub-type for a content-moves direction (SMIL: "fromLeft" enters from the left, so it
/// moves right).
fn odp_from(dir: &str) -> &'static str {
    match dir {
        "l" => "fromRight",
        "r" => "fromLeft",
        "u" => "fromBottom",
        "d" => "fromTop",
        "lu" => "fromBottomRight",
        "ru" => "fromBottomLeft",
        "ld" => "fromTopRight",
        _ => "fromTopLeft",
    }
}

fn dir_word(dir: &str) -> &'static str {
    match dir {
        "l" => "left",
        "r" => "right",
        "u" => "up",
        "d" => "down",
        "lu" => "left-up",
        "ru" => "right-up",
        "ld" => "left-down",
        _ => "right-down",
    }
}

struct TSpec {
    key: String,
    title: String,
    trans: Trans,
    rows: Vec<&'static str>,
}

fn tspec(
    key: impl Into<String>,
    title: impl Into<String>,
    pptx: Option<PTrans>,
    odp: Option<OTrans>,
    vocab: Option<(&'static str, Json)>,
    fallback: &'static str,
    rows: &[&'static str],
) -> TSpec {
    TSpec {
        key: key.into(),
        title: title.into(),
        trans: Trans {
            pptx,
            odp,
            vocab: vocab.map(|(name, params)| (name, params, false)),
            odp_params: None,
            fallback,
        },
        rows: rows.to_vec(),
    }
}

fn push_transition(slides: &mut Vec<Slide>, spec: TSpec) {
    let formats = Formats {
        pptx: spec.trans.pptx.is_some(),
        odp: spec.trans.odp.is_some(),
    };
    let index = slides.len();
    let mut s = slide(spec.key, format!("Transition: {}", spec.title), formats);
    s.shapes
        .push(shape(format!("transition {}", spec.title), index, formats));
    s.transition = Some(spec.trans);
    s.rows = spec.rows.iter().map(|r| (*r).to_owned()).collect();
    slides.push(s);
}

#[allow(clippy::vec_init_then_push)] // one push per catalog row reads as the table it is
fn transitions(slides: &mut Vec<Slide>) {
    let p = params;
    let mut list: Vec<TSpec> = Vec::new();
    // ECMA-376 p:transition children, with their LibreOffice transitions.xml equivalents.
    list.push(tspec(
        "tr-cut",
        "cut",
        Some(pt("cut", &[])),
        None,
        Some(("cut", p().with("throughBlack", false))),
        "none",
        &["12.1/cut"],
    ));
    list.push(tspec(
        "tr-cut-black",
        "cut through black",
        Some(pt("cut", &[("thruBlk", "1")])),
        Some(OTrans {
            smil: Some((
                "barWipe".into(),
                "fadeOverColor".into(),
                false,
                Some("#000000"),
            )),
            ..ot("", "", false)
        }),
        Some(("cut", p().with("throughBlack", true))),
        "none",
        &["12.1/cut", "12.3/barWipe", "12.3/fadeColor"],
    ));
    list.push(tspec(
        "tr-fade",
        "fade",
        Some(pt("fade", &[])),
        Some(ot("fade", "crossfade", false)),
        Some(("fade", p().with("through", "none"))),
        "dissolve",
        &["12.1/fade", "12.3/fade"],
    ));
    list.push(tspec(
        "tr-fade-black",
        "fade through black",
        Some(pt("fade", &[("thruBlk", "1")])),
        Some(OTrans {
            smil: Some((
                "fade".into(),
                "fadeOverColor".into(),
                false,
                Some("#000000"),
            )),
            ..ot("", "", false)
        }),
        Some(("fade", p().with("through", "black"))),
        "dissolve",
        &["12.1/fade", "12.3/fade", "12.3/fadeColor"],
    ));
    list.push(tspec(
        "tr-fade-color",
        "fade through colour #FF8000",
        None,
        Some(OTrans {
            smil: Some((
                "fade".into(),
                "fadeOverColor".into(),
                false,
                Some("#FF8000"),
            )),
            ..ot("", "", false)
        }),
        Some((
            "fade",
            p().with("through", "color").with("color", "#FF8000"),
        )),
        "dissolve",
        &["12.3/fade", "12.3/fadeColor"],
    ));
    list.push(tspec(
        "tr-dissolve",
        "dissolve",
        Some(pt("dissolve", &[])),
        Some(ot("dissolve", "default", false)),
        Some(("dissolve", p())),
        "dissolve",
        &["12.1/dissolve", "12.3/dissolve"],
    ));
    for dir in ["l", "u", "r", "d"] {
        list.push(tspec(
            format!("tr-push-{dir}"),
            format!("push dir={dir} (content moves {})", dir_word(dir)),
            Some(pt("push", &[("dir", dir)])),
            Some(ot("pushWipe", odp_from(dir), false)),
            Some(("push", p().with("dir", dir))),
            "push",
            &["12.1/push", "12.3/pushWipe"],
        ));
    }
    for dir in ["l", "u", "r", "d", "lu", "ru", "ld", "rd"] {
        list.push(tspec(
            format!("tr-cover-{dir}"),
            format!("cover dir={dir} (content moves {})", dir_word(dir)),
            Some(pt("cover", &[("dir", dir)])),
            Some(ot("slideWipe", odp_from(dir), false)),
            Some(("cover", p().with("dir", dir))),
            "push",
            &["12.1/cover", "12.3/slideWipe"],
        ));
    }
    for dir in ["l", "u", "r", "d", "lu", "ru", "ld", "rd"] {
        list.push(tspec(
            format!("tr-pull-{dir}"),
            format!("pull (uncover) dir={dir} (content moves {})", dir_word(dir)),
            Some(pt("pull", &[("dir", dir)])),
            Some(ot("slideWipe", odp_from(dir), true)),
            Some(("uncover", p().with("dir", dir))),
            "push",
            &["12.1/pull", "12.3/slideWipe-reverse"],
        ));
    }
    for dir in ["l", "u", "r", "d"] {
        let (subtype, reverse) = match dir {
            "l" => ("leftToRight", true),
            "r" => ("leftToRight", false),
            "u" => ("topToBottom", true),
            _ => ("topToBottom", false),
        };
        let mut spec = tspec(
            format!("tr-wipe-{dir}"),
            format!("wipe dir={dir} (edge moves {})", dir_word(dir)),
            Some(pt("wipe", &[("dir", dir)])),
            Some(ot("barWipe", subtype, reverse)),
            Some(("wipe", p().with("dir", dir).with("soft", true))),
            "wipe",
            &["12.1/wipe", "12.3/barWipe"],
        );
        spec.trans.odp_params = Some(p().with("dir", dir));
        list.push(spec);
    }
    for (o, o_word) in [("horz", "horizontal"), ("vert", "vertical")] {
        for io in ["in", "out"] {
            list.push(tspec(
                format!("tr-split-{o}-{io}"),
                format!("split orient={o_word} {io}"),
                Some(pt("split", &[("orient", o), ("dir", io)])),
                Some(ot("barnDoorWipe", o_word, io == "in")),
                Some(("split", p().with("orient", o_word).with("inOut", io))),
                "wipe",
                &["12.1/split", "12.3/barnDoorWipe"],
            ));
        }
    }
    list.push(tspec(
        "tr-circle",
        "circle",
        Some(pt("circle", &[])),
        Some(ot("ellipseWipe", "circle", false)),
        Some(("iris", p().with("shape", "circle").with("inOut", "out"))),
        "wipe",
        &["12.1/circle-diamond-plus", "12.3/ellipseWipe"],
    ));
    list.push(tspec(
        "tr-diamond",
        "diamond",
        Some(pt("diamond", &[])),
        Some(ot("irisWipe", "diamond", false)),
        Some(("iris", p().with("shape", "diamond").with("inOut", "out"))),
        "wipe",
        &["12.1/circle-diamond-plus", "12.3/irisWipe"],
    ));
    list.push(tspec(
        "tr-plus",
        "plus",
        Some(pt("plus", &[])),
        Some(ot("fourBoxWipe", "cornersOut", false)),
        Some(("iris", p().with("shape", "plus").with("inOut", "out"))),
        "wipe",
        &["12.1/circle-diamond-plus", "12.3/fourBoxWipe"],
    ));
    // LibreOffice reads both PPTX zoom directions as SMIL `zoom default`; the ODP deck writes `out` reversed.
    for io in ["in", "out"] {
        list.push(tspec(
            format!("tr-zoom-{io}"),
            format!("zoom {io}"),
            Some(pt("zoom", &[("dir", io)])),
            Some(ot("zoom", "default", io == "out")),
            Some(("zoom", p().with("inOut", io))),
            "dissolve",
            &["12.1/zoom", "12.3/zoom"],
        ));
    }
    list.push(tspec(
        "tr-newsflash",
        "newsflash (zoom rotate in)",
        Some(pt("newsflash", &[])),
        Some(ot("zoom", "rotateIn", false)),
        Some(("zoom", p().with("inOut", "in").with("rotate", true))),
        "dissolve",
        &["12.1/newsflash", "12.3/zoom"],
    ));
    for (spokes, blade) in [
        (1, "oneBlade"),
        (2, "twoBladeVertical"),
        (3, "threeBlade"),
        (4, "fourBlade"),
        (8, "eightBlade"),
    ] {
        list.push(tspec(
            format!("tr-wheel-{spokes}"),
            format!("wheel spokes={spokes}"),
            Some(pt("wheel", &[("spokes", &spokes.to_string())])),
            Some(ot("pinWheelWipe", blade, false)),
            Some(("wheel", p().with("spokes", spokes as u32).with("cw", true))),
            "wipe",
            &["12.1/wheel", "12.3/pinWheelWipe"],
        ));
    }
    list.push(tspec(
        "tr-wedge",
        "wedge",
        Some(pt("wedge", &[])),
        Some(ot("fanWipe", "centerTop", false)),
        Some(("wedge", p())),
        "wipe",
        &["12.1/wedge", "12.3/fanWipe"],
    ));
    for (o, o_word) in [("horz", "horizontal"), ("vert", "vertical")] {
        list.push(tspec(
            format!("tr-blinds-{o}"),
            format!("blinds orient={o_word}"),
            Some(pt("blinds", &[("dir", o)])),
            Some(ot("blindsWipe", o_word, false)),
            Some(("blinds", p().with("orient", o_word))),
            "wipe",
            &["12.1/blinds-checker-randomBar-comb", "12.3/blindsWipe"],
        ));
    }
    for (o, smil, d) in [("horz", "across", "r"), ("vert", "down", "d")] {
        list.push(tspec(
            format!("tr-checker-{o}"),
            format!("checker dir={o} (moves {})", dir_word(d)),
            Some(pt("checker", &[("dir", o)])),
            Some(ot("checkerBoardWipe", smil, false)),
            Some(("checker", p().with("dir", d))),
            "wipe",
            &[
                "12.1/blinds-checker-randomBar-comb",
                "12.3/checkerBoardWipe",
            ],
        ));
    }
    for (o, o_word) in [("horz", "horizontal"), ("vert", "vertical")] {
        list.push(tspec(
            format!("tr-randombar-{o}"),
            format!("random bars orient={o_word}"),
            Some(pt("randomBar", &[("dir", o)])),
            Some(ot("randomBarWipe", o_word, false)),
            Some(("randomBars", p().with("orient", o_word))),
            "dissolve",
            &["12.1/blinds-checker-randomBar-comb", "12.3/randomBarWipe"],
        ));
    }
    for (o, o_word, smil) in [
        ("horz", "horizontal", "combHorizontal"),
        ("vert", "vertical", "combVertical"),
    ] {
        list.push(tspec(
            format!("tr-comb-{o}"),
            format!("comb orient={o_word}"),
            Some(pt("comb", &[("dir", o)])),
            Some(ot("pushWipe", smil, false)),
            Some(("comb", p().with("orient", o_word))),
            "push",
            &["12.1/blinds-checker-randomBar-comb", "12.3/pushWipe"],
        ));
    }
    for (corner, subtype, reverse) in [
        ("lu", "horizontalLeft", true),
        ("ru", "horizontalRight", true),
        ("ld", "horizontalRight", false),
        ("rd", "horizontalLeft", false),
    ] {
        list.push(tspec(
            format!("tr-strips-{corner}"),
            format!("strips dir={corner} (moves {})", dir_word(corner)),
            Some(pt("strips", &[("dir", corner)])),
            Some(ot("waterfallWipe", subtype, reverse)),
            Some(("strips", p().with("corner", corner))),
            "wipe",
            &["12.1/strips", "12.3/waterfallWipe"],
        ));
    }
    list.push(tspec(
        "tr-random",
        "random",
        Some(pt("random", &[])),
        Some(ot("random", "", false)),
        Some(("random", p())),
        "dissolve",
        &["12.1/random", "12.3/random"],
    ));
    // p14 (PowerPoint 2010) inside mc:AlternateContent with a p:fade fallback choice.
    list.push(tspec(
        "tr-p14-flash",
        "p14 flash (fade through white)",
        Some(pt_ext(Ext::P14, "flash", &[])),
        Some(OTrans {
            smil: Some((
                "fade".into(),
                "fadeOverColor".into(),
                false,
                Some("#FFFFFF"),
            )),
            ..ot("", "", false)
        }),
        Some(("fade", p().with("through", "white"))),
        "dissolve",
        &["12.1/p14-flash", "12.3/fade", "12.3/fadeColor"],
    ));
    for black in [false, true] {
        for dir in ["l", "r"] {
            let mut attrs = vec![("dir", dir)];
            if black {
                attrs.push(("thruBlk", "1"));
            }
            list.push(tspec(
                format!("tr-p14-reveal-{dir}{}", if black { "-black" } else { "" }),
                format!(
                    "p14 reveal dir={dir}{}",
                    if black { " through black" } else { "" }
                ),
                Some(pt_ext(Ext::P14, "reveal", &attrs)),
                None,
                Some(("reveal", p().with("dir", dir).with("throughBlack", black))),
                "dissolve",
                &["12.1/p14-reveal"],
            ));
        }
    }
    for elem in ["flip", "switch", "gallery", "conveyor", "ferris"] {
        for dir in ["l", "r"] {
            list.push(tspec(
                format!("tr-p14-{elem}-{dir}"),
                format!("p14 {elem} dir={dir}"),
                Some(pt_ext(Ext::P14, elem, &[("dir", dir)])),
                None,
                Some((leak(elem), p().with("dir", dir))),
                "push",
                &["12.1/p14-3d"],
            ));
        }
    }
    for dir in ["l", "u", "r", "d"] {
        list.push(tspec(
            format!("tr-p14-pan-{dir}"),
            format!("p14 pan dir={dir}"),
            Some(pt_ext(Ext::P14, "pan", &[("dir", dir)])),
            None,
            Some(("pan", p().with("dir", dir))),
            "push",
            &["12.1/p14-3d"],
        ));
    }
    for elem in ["doors", "window"] {
        for (o, o_word) in [("horz", "horizontal"), ("vert", "vertical")] {
            list.push(tspec(
                format!("tr-p14-{elem}-{o}"),
                format!("p14 {elem} orient={o_word}"),
                Some(pt_ext(Ext::P14, elem, &[("dir", o)])),
                None,
                Some((leak(elem), p().with("orient", o_word))),
                "wipe",
                &["12.1/p14-doors-window"],
            ));
        }
    }
    for io in ["in", "out"] {
        for bounce in [false, true] {
            let mut attrs = vec![("dir", io)];
            if bounce {
                attrs.push(("hasBounce", "1"));
            }
            list.push(tspec(
                format!(
                    "tr-p14-flythrough-{io}{}",
                    if bounce { "-bounce" } else { "" }
                ),
                format!(
                    "p14 flythrough {io}{}",
                    if bounce { " with bounce" } else { "" }
                ),
                Some(pt_ext(Ext::P14, "flythrough", &attrs)),
                None,
                Some(("flythrough", p().with("inOut", io).with("bounce", bounce))),
                "dissolve",
                &["12.1/p14-flythrough"],
            ));
        }
    }
    for io in ["in", "out"] {
        let mut spec = tspec(
            format!("tr-p14-warp-{io}"),
            format!("p14 warp {io}"),
            Some(pt_ext(Ext::P14, "warp", &[("dir", io)])),
            None,
            Some(("zoom", p().with("inOut", io))),
            "dissolve",
            &["12.1/p14-warp"],
        );
        spec.trans.vocab = spec.trans.vocab.map(|(n, pa, _)| (n, pa, true));
        list.push(spec);
    }
    // p14 kinds with no vocabulary entry: the fallback plays (ODP's OpenGL equivalents where LibreOffice has one).
    list.push(tspec(
        "tr-p14-prism",
        "p14 prism (fallback)",
        Some(pt_ext(Ext::P14, "prism", &[("dir", "l")])),
        Some(ot("miscShapeWipe", "cornersOut", false)),
        None,
        "dissolve",
        &["12.1/p14-fallback", "12.3/miscShapeWipe"],
    ));
    list.push(tspec(
        "tr-p14-prism-inverted",
        "p14 prism inverted (fallback)",
        Some(pt_ext(
            Ext::P14,
            "prism",
            &[("dir", "l"), ("isInverted", "1")],
        )),
        Some(ot("miscShapeWipe", "cornersIn", false)),
        None,
        "dissolve",
        &["12.1/p14-fallback", "12.3/miscShapeWipe"],
    ));
    list.push(tspec(
        "tr-p14-prism-content",
        "p14 prism content (fallback)",
        Some(pt_ext(
            Ext::P14,
            "prism",
            &[("dir", "r"), ("isContent", "1")],
        )),
        None,
        None,
        "dissolve",
        &["12.1/p14-fallback"],
    ));
    for dir in ["l", "u", "r", "d"] {
        let odp = (dir == "l").then(|| ot("miscShapeWipe", "vertical", false));
        list.push(tspec(
            format!("tr-p14-vortex-{dir}"),
            format!("p14 vortex dir={dir} (fallback)"),
            Some(pt_ext(Ext::P14, "vortex", &[("dir", dir)])),
            odp,
            None,
            "dissolve",
            &["12.1/p14-fallback", "12.3/miscShapeWipe"],
        ));
    }
    for dir in ["center", "lu", "ru", "ld", "rd"] {
        let odp = (dir == "center").then(|| ot("miscShapeWipe", "horizontal", false));
        list.push(tspec(
            format!("tr-p14-ripple-{dir}"),
            format!("p14 ripple dir={dir} (fallback)"),
            Some(pt_ext(Ext::P14, "ripple", &[("dir", dir)])),
            odp,
            None,
            "dissolve",
            &["12.1/p14-fallback", "12.3/miscShapeWipe"],
        ));
    }
    for (dir, pattern) in [
        ("l", "diamond"),
        ("u", "hexagon"),
        ("r", "diamond"),
        ("d", "hexagon"),
    ] {
        let odp = (dir == "l").then(|| ot("miscShapeWipe", "diamond", false));
        list.push(tspec(
            format!("tr-p14-glitter-{dir}-{pattern}"),
            format!("p14 glitter dir={dir} pattern={pattern} (fallback)"),
            Some(pt_ext(
                Ext::P14,
                "glitter",
                &[("dir", dir), ("pattern", pattern)],
            )),
            odp,
            None,
            "dissolve",
            &["12.1/p14-fallback", "12.3/miscShapeWipe"],
        ));
    }
    list.push(tspec(
        "tr-p14-honeycomb",
        "p14 honeycomb (fallback)",
        Some(pt_ext(Ext::P14, "honeycomb", &[])),
        Some(ot("miscShapeWipe", "heart", false)),
        None,
        "dissolve",
        &["12.1/p14-fallback", "12.3/heart", "12.3/miscShapeWipe"],
    ));
    for pattern in ["strip", "rectangle"] {
        for io in ["in", "out"] {
            list.push(tspec(
                format!("tr-p14-shred-{pattern}-{io}"),
                format!("p14 shred pattern={pattern} {io} (fallback)"),
                Some(pt_ext(
                    Ext::P14,
                    "shred",
                    &[("pattern", pattern), ("dir", io)],
                )),
                None,
                None,
                "dissolve",
                &["12.1/p14-fallback"],
            ));
        }
    }
    for spokes in [1u32, 2, 3, 4, 8] {
        list.push(tspec(
            format!("tr-p14-wheelReverse-{spokes}"),
            format!("p14 wheelReverse spokes={spokes}"),
            Some(pt_ext(
                Ext::P14,
                "wheelReverse",
                &[("spokes", &spokes.to_string())],
            )),
            None,
            Some(("wheel", p().with("spokes", spokes).with("cw", false))),
            "wipe",
            &["12.1/p14-fallback"],
        ));
    }
    // p15 presets: no vocabulary entry in v1.
    for prst in [
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
    ] {
        let odp = (prst == "fallOver").then(|| ot("miscShapeWipe", "leftToRight", false));
        let mut rows = vec!["12.1/p15-prstTrans"];
        if odp.is_some() {
            rows.push("12.3/miscShapeWipe");
        }
        list.push(tspec(
            format!("tr-p15-{prst}"),
            format!("p15 prstTrans prst={prst} (fallback)"),
            Some(pt_ext(Ext::P15, "prstTrans", &[("prst", prst)])),
            odp,
            None,
            "dissolve",
            &rows,
        ));
    }
    for (prst, inv) in [
        ("fallOver", "invX"),
        ("curtains", "invX"),
        ("pageCurlDouble", "invY"),
        ("airplane", "invX"),
        ("origami", "invY"),
    ] {
        list.push(tspec(
            format!("tr-p15-{prst}-{inv}"),
            format!("p15 prstTrans prst={prst} {inv} (fallback)"),
            Some(pt_ext(Ext::P15, "prstTrans", &[("prst", prst), (inv, "1")])),
            None,
            None,
            "dissolve",
            &["12.1/p15-prstTrans"],
        ));
    }
    for (option, by) in [
        ("byObject", "object"),
        ("byWord", "word"),
        ("byChar", "char"),
    ] {
        list.push(tspec(
            format!("tr-p159-morph-{by}"),
            format!("p159 morph option={option}"),
            Some(pt_ext(Ext::P159, "morph", &[("option", option)])),
            None,
            Some(("morph", p().with("by", by))),
            "dissolve",
            &["12.1/p159-morph"],
        ));
    }
    // ODF-only transitions from transitions.xml.
    list.push(tspec(
        "tr-odp-box-out",
        "box out (irisWipe rectangle)",
        None,
        Some(ot("irisWipe", "rectangle", false)),
        Some(("iris", p().with("shape", "box").with("inOut", "out"))),
        "wipe",
        &["12.3/irisWipe"],
    ));
    list.push(tspec(
        "tr-odp-box-in",
        "box in (irisWipe rectangle reverse)",
        None,
        Some(ot("irisWipe", "rectangle", true)),
        Some(("iris", p().with("shape", "box").with("inOut", "in"))),
        "wipe",
        &["12.3/irisWipe"],
    ));
    for o in ["horizontal", "vertical"] {
        let mut spec = tspec(
            format!("tr-odp-oval-{o}"),
            format!("oval {o} (ellipseWipe {o})"),
            None,
            Some(ot("ellipseWipe", o, false)),
            Some(("iris", p().with("shape", "circle").with("inOut", "out"))),
            "wipe",
            &["12.3/ellipseWipe"],
        );
        spec.trans.vocab = spec.trans.vocab.map(|(n, pa, _)| (n, pa, true));
        list.push(spec);
    }
    list.push(tspec(
        "tr-odp-clock-cw",
        "clock wipe from 12 clockwise",
        None,
        Some(ot("clockWipe", "clockwiseTwelve", false)),
        Some(("clock", p().with("cw", true).with("from", 12u32))),
        "wipe",
        &["12.3/clockWipe"],
    ));
    list.push(tspec(
        "tr-odp-clock-ccw",
        "clock wipe from 12 counter-clockwise",
        None,
        Some(ot("clockWipe", "clockwiseTwelve", true)),
        Some(("clock", p().with("cw", false).with("from", 12u32))),
        "wipe",
        &["12.3/clockWipe"],
    ));
    for &(id, ty, subtype, reverse, _) in ODF_TRANSITIONS {
        let unlisted = matches!(ty, "spiralWipe" | "snakeWipe")
            || (ty == "miscShapeWipe"
                && !matches!(
                    subtype,
                    "cornersOut"
                        | "cornersIn"
                        | "vertical"
                        | "horizontal"
                        | "diamond"
                        | "heart"
                        | "leftToRight"
                ));
        if unlisted {
            list.push(tspec(
                format!("tr-odp-{id}"),
                format!(
                    "{id} ({ty} {subtype}{}) (fallback)",
                    if reverse { " reverse" } else { "" }
                ),
                None,
                Some(ot(ty, subtype, reverse)),
                None,
                "dissolve",
                &[if ty == "miscShapeWipe" {
                    "12.3/miscShapeWipe"
                } else if ty == "spiralWipe" {
                    "12.3/spiralWipe"
                } else {
                    "12.3/snakeWipe"
                }],
            ));
        }
    }
    for (ty, subtype, row) in [
        ("zigZagWipe", "leftToRight", "12.3/zigZagWipe"),
        ("bowTieWipe", "vertical", "12.3/bowTieWipe"),
        ("veeWipe", "down", "12.3/veeWipe"),
        ("starWipe", "fivePoint", "12.3/starWipe"),
    ] {
        list.push(tspec(
            format!("tr-odp-{ty}"),
            format!("{ty} {subtype} (fallback)"),
            None,
            Some(ot(ty, subtype, false)),
            None,
            "dissolve",
            &[row],
        ));
    }
    for (style, effect_name, params, row) in [
        (
            "move-from-top",
            "cover",
            p().with("dir", "d"),
            "odp/legacy-style",
        ),
        (
            "uncover-to-right",
            "uncover",
            p().with("dir", "r"),
            "odp/legacy-style",
        ),
        ("dissolve", "dissolve", p(), "odp/legacy-style"),
    ] {
        list.push(tspec(
            format!("tr-odp-legacy-{style}"),
            format!("legacy transition-style {style}"),
            None,
            Some(OTrans {
                smil: None,
                legacy: Some(style),
                speed: "medium",
                filter_dur: None,
                advance: OAdv::Manual,
                duration_raw: None,
            }),
            Some((effect_name, params)),
            if effect_name == "dissolve" {
                "dissolve"
            } else {
                "push"
            },
            &[row],
        ));
    }
    for spec in list {
        push_transition(slides, spec);
    }
}

fn leak(name: &str) -> &'static str {
    match name {
        "flip" => "flip",
        "switch" => "switch",
        "gallery" => "gallery",
        "conveyor" => "conveyor",
        "ferris" => "ferris",
        "doors" => "doors",
        "window" => "window",
        _ => "unknown",
    }
}

/// Durations and advance: PPTX `spd`, `p14:dur`, `advClick`/`advTm`; ODF speed, the timing root's exact duration,
/// and the three advance modes.
fn timing_slides(slides: &mut Vec<Slide>) {
    let fade = || Some(("fade", params().with("through", "none")));
    let mut list = Vec::new();
    for (spd, speed, ms_odp) in [
        ("slow", "slow", 2000),
        ("med", "medium", 1000),
        ("fast", "fast", 500),
    ] {
        let mut pptx = pt("fade", &[]);
        pptx.spd = Some(spd);
        let odp = OTrans {
            filter_dur: None,
            speed,
            ..ot("fade", "crossfade", false)
        };
        let _ = ms_odp;
        list.push(tspec(
            format!("tm-speed-{spd}"),
            format!("fade at speed {speed} (no exact duration)"),
            Some(pptx),
            Some(odp),
            fade(),
            "dissolve",
            &["12.1/durations", "12.3/duration-speed"],
        ));
    }
    let mut no_spd = pt("fade", &[]);
    no_spd.spd = None;
    list.push(tspec(
        "tm-speed-default",
        "fade with no speed (schema default fast)",
        Some(no_spd),
        None,
        fade(),
        "dissolve",
        &["12.1/durations"],
    ));
    let mut exact = pt("push", &[("dir", "u")]);
    exact.alternate = true;
    exact.p14_dur = Some(1200);
    exact.spd = Some("slow");
    let mut exact_odp = ot("pushWipe", "fromBottom", false);
    exact_odp.filter_dur = Some(1200);
    exact_odp.speed = "slow";
    list.push(tspec(
        "tm-exact-1200",
        "push dir=u with an exact 1200 ms duration",
        Some(exact),
        Some(exact_odp),
        Some(("push", params().with("dir", "u"))),
        "push",
        &["12.1/durations", "12.3/duration-filter"],
    ));
    let mut kiosk = pt("fade", &[]);
    kiosk.adv_click = Some(false);
    kiosk.adv_tm = Some(3000);
    let kiosk_odp = OTrans {
        advance: OAdv::Automatic(Some(3000)),
        ..ot("fade", "crossfade", false)
    };
    list.push(tspec(
        "tm-advance-auto-3000",
        "fade, advances after 3000 ms without a click",
        Some(kiosk),
        Some(kiosk_odp),
        fade(),
        "dissolve",
        &["12.1/advance", "12.3/advance"],
    ));
    let mut both = pt("fade", &[]);
    both.adv_tm = Some(2000);
    list.push(tspec(
        "tm-advance-click-or-2000",
        "fade, advances on click or after 2000 ms",
        Some(both),
        None,
        fade(),
        "dissolve",
        &["12.1/advance"],
    ));
    let semi = OTrans {
        advance: OAdv::SemiAutomatic,
        ..ot("fade", "crossfade", false)
    };
    list.push(tspec(
        "tm-advance-semi",
        "fade, semi-automatic (effects run on their own, advance on click)",
        None,
        Some(semi),
        fade(),
        "dissolve",
        &["12.3/advance"],
    ));
    let auto0 = OTrans {
        advance: OAdv::Automatic(None),
        ..ot("fade", "crossfade", false)
    };
    list.push(tspec(
        "tm-advance-auto-0",
        "fade, automatic with no duration (advances at once)",
        None,
        Some(auto0),
        fade(),
        "dissolve",
        &["12.3/advance"],
    ));
    for spec in list {
        push_transition(slides, spec);
    }
}

// ---- presets ---------------------------------------------------------------------------------------------------

struct Row {
    class: Class,
    ooo: String,
    sub: String,
}

fn class_of(odf: &str) -> Class {
    match odf {
        "entrance" => Class::Entr,
        "exit" => Class::Exit,
        "emphasis" => Class::Emph,
        "motion-path" => Class::Path,
        _ => Class::Media,
    }
}

/// One step per ODF preset, six to a slide (eight for motion paths), in `effects.xml` order; media presets go to the
/// media slide.
fn preset_slides(slides: &mut Vec<Slide>) {
    let rows: Vec<Row> = ODF_PRESETS
        .iter()
        .filter(|(class, ..)| *class != "media-call")
        .map(|&(class, ooo, sub)| Row {
            class: class_of(class),
            ooo: ooo.to_owned(),
            sub: sub.to_owned(),
        })
        .collect();
    let mut index = 0;
    let mut page = 0;
    while index < rows.len() {
        let class = rows[index].class;
        let per_slide = if class == Class::Path { 8 } else { 6 };
        let chunk: Vec<&Row> = rows[index..]
            .iter()
            .take_while(|r| r.class == class)
            .take(per_slide)
            .collect();
        index += chunk.len();
        page += 1;
        let class_word = match class {
            Class::Entr => "entrance",
            Class::Exit => "exit",
            Class::Emph => "emphasis",
            _ => "motion path",
        };
        let mut s = slide(
            format!("pr-{:02}-{}", page, class.pptx()),
            format!("Build presets ({class_word}) {page}"),
            BOTH,
        );
        let mut any_pptx = false;
        let mut any_odp = false;
        for (n, row) in chunk.into_iter().enumerate() {
            let pptx_id = pptx_preset_id(&row.ooo);
            let pptx_sub = pptx_id.and_then(|id| pptx_subtype(id, &row.sub));
            let formats = Formats {
                pptx: pptx_id.is_some() && pptx_sub.is_some(),
                odp: true,
            };
            any_pptx |= formats.pptx;
            any_odp |= formats.odp;
            let mut bhv = preset_bhv(row.class, &row.ooo, &row.sub);
            let vocab = with_tier2_clip(preset_vocab(row.class, &row.ooo, &row.sub), &bhv);
            if matches!(
                vocab,
                Vocab::Effect {
                    delivery: Some(_),
                    ..
                }
            ) {
                bhv = vec![Bhv::Color("text", PROBE_COLOR)];
            }
            let label = preset_label(row, pptx_id, pptx_sub, &plays(vocab.clone(), &bhv));
            s.shapes.push(shape(label, n, formats));
            let effect = Effect {
                class: row.class,
                ooo: Some(row.ooo.clone()),
                sub: (!row.sub.is_empty()).then(|| row.sub.clone()),
                pptx_id,
                pptx_sub: pptx_sub.unwrap_or(0),
                bhv,
            };
            let mut step = Step::new(n, effect, vocab, formats);
            if matches!(
                step.vocab,
                Vocab::Effect {
                    delivery: Some("byChar"),
                    ..
                }
            ) {
                step.iterate = Some(Iterate {
                    unit: Unit::Letter,
                    interval: Interval::Ms(50),
                    backwards: false,
                });
            }
            step.vocab = plays(step.vocab, &step.effect.bhv);
            if step.is_switch() {
                step.dur_ms = 1;
            }
            s.rows.push(preset_row(row.class, pptx_id, &row.ooo));
            if let Some(sub) =
                pptx_sub.filter(|&sub| sub != 0 && row.class != Class::Emph && pptx_id != Some(21))
            {
                s.rows.push(format!("12.2/subtype.{sub}"));
            }
            s.steps.push(step);
        }
        s.formats = Formats {
            pptx: any_pptx,
            odp: any_odp,
        };
        s.rows.push("12.3/preset-class".into());
        s.rows.push("12.3/preset-id".into());
        s.rows.push("12.3/preset-sub-type".into());
        s.rows.push("12.3/node-type".into());
        slides.push(s);
    }
}

fn preset_row(class: Class, pptx_id: Option<u32>, ooo: &str) -> String {
    match (class, pptx_id) {
        (Class::Path, Some(id)) => format!("12.2/path.{id}"),
        (_, Some(id)) => format!("12.2/{}.{id}", class.pptx()),
        _ => format!("odp/{ooo}"),
    }
}

fn preset_label(row: &Row, pptx_id: Option<u32>, pptx_sub: Option<u32>, vocab: &Vocab) -> String {
    let short = row.ooo.trim_start_matches("ooo-");
    let sub = if row.sub.is_empty() {
        String::new()
    } else {
        format!(" {}", row.sub)
    };
    let pptx = match (pptx_id, pptx_sub) {
        (Some(id), Some(s)) if s != 0 => format!(" [pptx {}.{id}/{s}]", row.class.pptx()),
        (Some(id), Some(_)) => format!(" [pptx {}.{id}]", row.class.pptx()),
        _ => " [odp only]".to_owned(),
    };
    format!("{short}{sub}{pptx}: {}", vocab_text(vocab))
}

/// A short human description of the expectation, printed on the shape.
pub fn vocab_text(vocab: &Vocab) -> String {
    match vocab {
        Vocab::Effect { name, params, .. } => {
            let mut text = (*name).to_owned();
            if let Json::Obj(fields) = params {
                for (key, value) in fields {
                    let value = match value {
                        Json::Str(s) => s.clone(),
                        Json::Int(i) => i.to_string(),
                        Json::Num(n) => n.clone(),
                        Json::Bool(b) => b.to_string(),
                        Json::Arr(items) => items
                            .iter()
                            .map(|i| match i {
                                Json::Num(n) => n.clone(),
                                _ => String::new(),
                            })
                            .collect::<Vec<_>>()
                            .join(","),
                        _ => String::new(),
                    };
                    text.push_str(&format!(" {key}={value}"));
                }
            }
            text
        }
        Vocab::Tier2 { .. } => "behaviors (tier 2)".to_owned(),
        Vocab::Fallback => "no vocabulary entry: plays the fallback".to_owned(),
        Vocab::Dropped => "dropped (not in the IR)".to_owned(),
        Vocab::Bounded => "hostile timing: dropped".to_owned(),
    }
}

// ---- feature slides --------------------------------------------------------------------------------------------

fn preset(class: Class, ooo: &str, sub: &str) -> (Effect, Vocab) {
    let pptx_id = pptx_preset_id(ooo);
    let pptx_sub = pptx_id.and_then(|id| pptx_subtype(id, sub)).unwrap_or(0);
    let bhv = preset_bhv(class, ooo, sub);
    let vocab = plays(with_tier2_clip(preset_vocab(class, ooo, sub), &bhv), &bhv);
    let effect = Effect {
        class,
        ooo: Some(ooo.to_owned()),
        sub: (!sub.is_empty()).then(|| sub.to_owned()),
        pptx_id,
        pptx_sub,
        bhv,
    };
    (effect, vocab)
}

fn fade_in() -> (Effect, Vocab) {
    preset(Class::Entr, "ooo-entrance-fade-in", "")
}

fn feature(
    slides: &mut Vec<Slide>,
    key: &str,
    title: &str,
    formats: Formats,
    rows: &[&str],
) -> usize {
    let mut s = slide(key, title, formats);
    s.rows = rows.iter().map(|r| (*r).to_owned()).collect();
    slides.push(s);
    slides.len() - 1
}

fn add_step(
    s: &mut Slide,
    label: &str,
    (effect, vocab): (Effect, Vocab),
    formats: Formats,
    edit: impl FnOnce(&mut Step),
) {
    let n = s.shapes.len();
    s.shapes.push(shape(label, n, formats));
    let mut step = Step::new(n, effect, vocab, formats);
    edit(&mut step);
    s.steps.push(step);
}

fn feature_slides(slides: &mut Vec<Slide>) {
    // Triggers: click, with previous (+250 ms), after previous (+500 ms), a second click group.
    let i = feature(
        slides,
        "ft-triggers",
        "Triggers: click, with previous, after previous, with delays",
        BOTH,
        &["12.3/node-type", "triggers"],
    );
    let s = &mut slides[i];
    add_step(s, "A: fade on click (offset 0)", fade_in(), BOTH, |_| {});
    add_step(
        s,
        "B: fly-in dir=r with previous, delay 250 ms (offset 250)",
        preset(Class::Entr, "ooo-entrance-fly-in", "from-left"),
        BOTH,
        |st| {
            st.trigger = Trigger::With;
            st.delay_ms = 250;
        },
    );
    add_step(
        s,
        "C: wipe dir=u after previous, delay 500 ms (offset 1250)",
        preset(Class::Entr, "ooo-entrance-wipe", "from-bottom"),
        BOTH,
        |st| {
            st.trigger = Trigger::After;
            st.delay_ms = 500;
        },
    );
    add_step(
        s,
        "D: zoom in on a second click (offset 0)",
        preset(Class::Entr, "ooo-entrance-zoom", "in"),
        BOTH,
        |_| {},
    );
    add_step(
        s,
        "E: fade after previous, no delay (offset 500)",
        fade_in(),
        BOTH,
        |st| st.trigger = Trigger::After,
    );

    // An automatic group 0: the first effect starts after the transition, without a click.
    let i = feature(
        slides,
        "ft-auto-start",
        "Automatic start: the first effect plays after the previous (no click)",
        BOTH,
        &["12.3/node-type", "triggers"],
    );
    let s = &mut slides[i];
    add_step(
        s,
        "fade after previous, starts on its own (group 0)",
        fade_in(),
        BOTH,
        |st| st.trigger = Trigger::After,
    );
    add_step(
        s,
        "fly-in dir=u with previous, delay 200 ms (group 0)",
        preset(Class::Entr, "ooo-entrance-fly-in", "from-bottom"),
        BOTH,
        |st| {
            st.trigger = Trigger::With;
            st.delay_ms = 200;
        },
    );
    add_step(s, "fade on click (group 1)", fade_in(), BOTH, |_| {});

    // By paragraph: an empty paragraph (no step of its own) and a level-2 child that builds with its parent.
    let i = feature(
        slides,
        "ft-by-paragraph",
        "By paragraph: an empty paragraph and a level-2 child",
        BOTH,
        &["12.3/paragraph-target", "bldP"],
    );
    let s = &mut slides[i];
    s.shapes.push(Shape {
        paras: vec![
            Para {
                text: "fade by paragraph: paragraph 0 on click".into(),
                level: 0,
            },
            Para {
                text: String::new(),
                level: 0,
            },
            Para {
                text: "paragraph 2 on the next click".into(),
                level: 0,
            },
            Para {
                text: "paragraph 3, a level-2 child, with its parent".into(),
                level: 1,
            },
        ],
        media: false,
        line: false,
        color: COLORS[1],
        formats: BOTH,
    });
    for (para, trigger) in [(0, Trigger::Click), (2, Trigger::Click), (3, Trigger::With)] {
        let (fx, vocab) = fade_in();
        let mut step = Step::new(0, fx, vocab, BOTH);
        step.paragraphs = Some((para, para));
        step.trigger = trigger;
        s.steps.push(step);
    }

    // Iterate by word and letter, absolute and relative intervals.
    let i = feature(
        slides,
        "ft-iterate-word-50ms",
        "Iterate by word, 50 ms per word",
        BOTH,
        &["12.3/iterate", "iterate"],
    );
    add_step(
        &mut slides[i],
        "fade by word, 50 ms between words",
        fade_in(),
        BOTH,
        |st| {
            st.iterate = Some(Iterate {
                unit: Unit::Word,
                interval: Interval::Ms(50),
                backwards: false,
            });
        },
    );
    let i = feature(
        slides,
        "ft-iterate-letter-25ms",
        "Iterate by letter, 25 ms per letter",
        BOTH,
        &["12.3/iterate", "iterate"],
    );
    add_step(
        &mut slides[i],
        "appear by letter, 25 ms between letters",
        preset(Class::Entr, "ooo-entrance-appear", ""),
        BOTH,
        |st| {
            st.iterate = Some(Iterate {
                unit: Unit::Letter,
                interval: Interval::Ms(25),
                backwards: false,
            });
            st.dur_ms = 1;
        },
    );
    let i = feature(
        slides,
        "ft-iterate-word-pct-back",
        "Iterate by word, 10% of the duration, backwards",
        PPTX,
        &["iterate"],
    );
    add_step(
        &mut slides[i],
        "fade by word, tmPct 10%, backwards",
        fade_in(),
        PPTX,
        |st| {
            st.iterate = Some(Iterate {
                unit: Unit::Word,
                interval: Interval::Pct(10_000),
                backwards: true,
            });
        },
    );
    let i = feature(
        slides,
        "ft-iterate-letter-pct",
        "Iterate by letter, 5% of the duration",
        PPTX,
        &["iterate"],
    );
    add_step(
        &mut slides[i],
        "fly-in dir=u by letter, tmPct 5%",
        preset(Class::Entr, "ooo-entrance-fly-in", "from-bottom"),
        PPTX,
        |st| {
            st.iterate = Some(Iterate {
                unit: Unit::Letter,
                interval: Interval::Pct(5_000),
                backwards: false,
            });
        },
    );
    let i = feature(
        slides,
        "ft-iterate-paragraph",
        "Iterate by paragraph (anim:iterate by-paragraph)",
        ODP,
        &["12.3/iterate", "iterate"],
    );
    let s = &mut slides[i];
    s.shapes.push(Shape {
        paras: vec![
            Para {
                text: "fade iterating by paragraph: first".into(),
                level: 0,
            },
            Para {
                text: "second, 100 ms later".into(),
                level: 0,
            },
            Para {
                text: "third, 100 ms after that".into(),
                level: 0,
            },
        ],
        media: false,
        line: false,
        color: COLORS[3],
        formats: ODP,
    });
    let (fx, vocab) = fade_in();
    let mut step = Step::new(0, fx, vocab, ODP);
    step.iterate = Some(Iterate {
        unit: Unit::Paragraph,
        interval: Interval::Ms(100),
        backwards: false,
    });
    s.steps.push(step);

    // The shape's background first (PPTX animBg / ODF sub-item background), then its text by paragraph.
    let i = feature(
        slides,
        "ft-anim-bg",
        "Background then text: animBg / sub-item background",
        BOTH,
        &["animBg"],
    );
    let s = &mut slides[i];
    s.shapes.push(Shape {
        paras: vec![
            Para {
                text: "the box fades in first (background target)".into(),
                level: 0,
            },
            Para {
                text: "then this paragraph wipes in (dir=r)".into(),
                level: 0,
            },
        ],
        media: false,
        line: false,
        color: COLORS[2],
        formats: BOTH,
    });
    let (fx, vocab) = fade_in();
    let mut step = Step::new(0, fx, vocab, BOTH);
    step.background = true;
    s.steps.push(step);
    for para in [0, 1] {
        let (fx, vocab) = preset(Class::Entr, "ooo-entrance-wipe", "from-left");
        let mut step = Step::new(0, fx, vocab, BOTH);
        step.paragraphs = Some((para, para));
        step.trigger = if para == 0 {
            Trigger::Click
        } else {
            Trigger::After
        };
        s.steps.push(step);
    }

    // Repeat and auto-reverse.
    let i = feature(
        slides,
        "ft-repeat",
        "Repeat: 3 times, until the end of the slide, until the next click",
        BOTH,
        &["repeat"],
    );
    let s = &mut slides[i];
    add_step(
        s,
        "spin 360 repeated 3 times",
        preset(Class::Emph, "ooo-emphasis-spin", ""),
        BOTH,
        |st| st.repeat = Some(Repeat::Count(3000)),
    );
    add_step(
        s,
        "pulse (blast) repeated until the end of the slide",
        preset(Class::Emph, "ooo-emphasis-blast", ""),
        BOTH,
        |st| st.repeat = Some(Repeat::UntilEndOfSlide),
    );
    add_step(
        s,
        "teeter repeated until the next click",
        preset(Class::Emph, "ooo-emphasis-teeter", ""),
        BOTH,
        |st| st.repeat = Some(Repeat::UntilNextClick),
    );
    let i = feature(
        slides,
        "ft-auto-reverse",
        "Auto-reverse",
        BOTH,
        &["autoRev"],
    );
    let s = &mut slides[i];
    add_step(
        s,
        "grow/shrink 150% with auto-reverse",
        preset(Class::Emph, "ooo-emphasis-grow-and-shrink", ""),
        BOTH,
        |st| st.auto_rev = true,
    );
    add_step(
        s,
        "fade after previous (offset 1000: after the reversed grow)",
        fade_in(),
        BOTH,
        |st| st.trigger = Trigger::After,
    );
    let i = feature(
        slides,
        "ft-easing",
        "Easing: accelerate 50%, decelerate 50%",
        BOTH,
        &["easing"],
    );
    add_step(
        &mut slides[i],
        "fly-in dir=l, accel 50% decel 50%",
        preset(Class::Entr, "ooo-entrance-fly-in", "from-right"),
        BOTH,
        |st| {
            st.accel = Some(50_000);
            st.decel = Some(50_000);
        },
    );

    // After effects.
    let i = feature(
        slides,
        "ft-after-effects",
        "After animation: hide, dim to grey",
        BOTH,
        &["after"],
    );
    let s = &mut slides[i];
    add_step(
        s,
        "fade, then hide after the animation",
        fade_in(),
        BOTH,
        |st| st.after = Some(After::Hide),
    );
    add_step(s, "fade, then dim to #808080", fade_in(), BOTH, |st| {
        st.after = Some(After::Dim("808080"))
    });

    // Tier 2: an unknown preset whose behaviours are all known primitives, including a formula.
    let i = feature(
        slides,
        "ft-tier2-unknown-preset",
        "Unknown preset with known behaviours (tier 2)",
        BOTH,
        &["12.2/tier2", "12.3/custom-tier2", "fmla"],
    );
    let s = &mut slides[i];
    let tier2 = Effect {
        class: Class::Entr,
        ooo: Some("ooo-entrance-probe-unknown".into()),
        sub: None,
        pptx_id: Some(99),
        pptx_sub: 0,
        bhv: vec![
            Bhv::Formula {
                pptx: "#ppt_x+sin(2*pi*$)*0.05".into(),
                odp: "x+sin(2*pi*$)*0.05".into(),
            },
            Bhv::Filter(pptx_clip("wipe(down)")),
            Bhv::Motion("M 0 0 L 0.125 0".into()),
            Bhv::Rotate(90),
            Bhv::Scale(150_000, 150_000),
            Bhv::Color("text", PROBE_COLOR),
        ],
    };
    let vocab = with_tier2_clip(Vocab::Tier2 { clip: None }, &tier2.bhv);
    add_step(
        s,
        "unknown entrance preset (pptx entr.99 / odp ooo-entrance-probe-unknown): set, anim with fmla, animEffect wipe(down) (edge moves up), animMotion, animRot 90, animScale 150%, animClr",
        (tier2, vocab),
        BOTH,
        |_| {},
    );
    let i = feature(
        slides,
        "ft-unknown-preset-fallback",
        "Unknown preset with no known behaviour: fallback",
        BOTH,
        &["fallback"],
    );
    let unknown = Effect {
        class: Class::Entr,
        ooo: Some("ooo-entrance-probe-opaque".into()),
        sub: None,
        pptx_id: Some(98),
        pptx_sub: 0,
        bhv: vec![Bhv::Unknown],
    };
    add_step(
        &mut slides[i],
        "unknown entrance preset (pptx entr.98) animating an unknown property: plays dissolve",
        (unknown, Vocab::Fallback),
        BOTH,
        |_| {},
    );

    // Tier 2 clip table: custom effects whose only clip is an animEffect filter. LibreOffice's
    // reading of the filter gives the ODF clip, and `clip_vocab` the expectation.
    let filters = [
        "wipe(up)",
        "wheel(4)",
        "strips(downLeft)",
        "barn(inVertical)",
        "randombar(horizontal)",
        "checkerboard(across)",
        "plus(in)",
        "diamond(out)",
        "circle(in)",
        "box(out)",
        "wedge",
        "blinds(horizontal)",
        "fade",
        "dissolve",
        "slide(fromTop)",
    ];
    for (page, chunk) in filters.chunks(5).enumerate() {
        let i = feature(
            slides,
            &format!("ft-tier2-filters-{}", page + 1),
            &format!(
                "Custom effects: animEffect filter decides the clip ({})",
                page + 1
            ),
            BOTH,
            &["12.2/tier2"],
        );
        let s = &mut slides[i];
        for filter in chunk {
            let clip = pptx_clip(filter);
            let custom = Effect {
                class: Class::Custom,
                ooo: Some("probe-custom-filter".into()),
                sub: None,
                pptx_id: Some(0),
                pptx_sub: 0,
                bhv: vec![Bhv::Filter(clip)],
            };
            let vocab = with_tier2_clip(Vocab::Tier2 { clip: None }, &custom.bhv);
            let clip_text = match &vocab {
                Vocab::Tier2 {
                    clip: Some((name, params)),
                } => vocab_text(&effect(name, params.clone())),
                _ => "none".to_owned(),
            };
            s.rows.push(format!(
                "12.2/tier2-filter.{}",
                filter.split('(').next().unwrap_or(filter)
            ));
            add_step(
                s,
                &format!("custom entrance (presetID 0), filter={filter}: clip {clip_text}"),
                (custom, vocab),
                BOTH,
                |_| {},
            );
        }
    }

    // A custom motion path.
    let i = feature(
        slides,
        "ft-custom-path",
        "Custom motion path (presetID 0)",
        BOTH,
        &["12.2/path", "path"],
    );
    let custom_path = Effect {
        class: Class::Path,
        ooo: Some("ooo-motionpath-custom".into()),
        sub: None,
        pptx_id: Some(0),
        pptx_sub: 0,
        bhv: vec![Bhv::Motion(motion_path("custom"))],
    };
    add_step(
        &mut slides[i],
        "custom curve path, ends 320 px right and 180 px down",
        (custom_path, effect("path", params())),
        BOTH,
        |_| {},
    );

    // Media: play, pause (toggle) and stop on our own clip.
    let i = feature(
        slides,
        "ft-media",
        "Media: play, pause (toggle), stop on a 1 s clip",
        BOTH,
        &["media"],
    );
    let s = &mut slides[i];
    s.shapes.push(Shape {
        paras: vec![Para {
            text: "probe clip (64x36, 1 s)".into(),
            level: 0,
        }],
        media: true,
        line: false,
        color: COLORS[0],
        formats: BOTH,
    });
    for (n, (ooo, label)) in [
        ("ooo-media-start", "play"),
        ("ooo-media-toggle-pause", "pause (toggle)"),
        ("ooo-media-stop", "stop"),
    ]
    .into_iter()
    .enumerate()
    {
        let (fx, vocab) = preset(Class::Media, ooo, "");
        let mut step = Step::new(0, fx, vocab, BOTH);
        // Play lasts the one-second clip; pause and stop are instants (PowerPoint's 1 ms, LibreOffice's 0.001 s).
        step.dur_ms = if ooo == "ooo-media-start" { 1000 } else { 1 };
        let _ = (n, label);
        s.steps.push(step);
    }
    s.rows.push("12.3/media-call".into());

    // An interactive sequence (an effect triggered by clicking a shape): not in the IR.
    let i = feature(
        slides,
        "ft-interactive",
        "Interactive trigger: click the button to fade the box (dropped)",
        BOTH,
        &["interactive"],
    );
    let s = &mut slides[i];
    s.shapes
        .push(shape("button: clicking it triggers the box", 4, BOTH));
    s.shapes.push(shape(
        "box: fades in when the button is clicked (interactive sequence)",
        5,
        BOTH,
    ));
    s.trigger_shape = Some(0);
    let (fx, _) = fade_in();
    let mut step = Step::new(1, fx, Vocab::Dropped, BOTH);
    step.interactive = true;
    s.steps.push(step);

    // Hostile timing (the IR's limits: bounded node counts, depths, repeats and durations; cycles end): a frontend
    // drops what the step model can't express, clamps what is too large and plays the fallback for what can't
    // compile, each with a diagnostic.
    let i = feature(
        slides,
        "ft-hostile",
        "Hostile timing: cycles, huge values, deep formula and nesting (bounded)",
        BOTH,
        &["hostile"],
    );
    let s = &mut slides[i];
    add_step(
        s,
        "A: fade whose start condition names its own end (self-reference): dropped",
        fade_in(),
        BOTH,
        |st| {
            st.begin_on = Some((0, true));
            st.vocab = Vocab::Bounded;
        },
    );
    add_step(
        s,
        "B: fade that starts when C begins (a two-node cycle): dropped",
        fade_in(),
        BOTH,
        |st| {
            st.begin_on = Some((2, false));
            st.vocab = Vocab::Bounded;
        },
    );
    add_step(
        s,
        "C: fade that starts when B ends (a two-node cycle): dropped",
        fade_in(),
        BOTH,
        |st| {
            st.begin_on = Some((1, true));
            st.vocab = Vocab::Bounded;
        },
    );
    add_step(
        s,
        "spin repeated 4294967 times: plays, the count clamps",
        preset(Class::Emph, "ooo-emphasis-spin", ""),
        BOTH,
        |st| st.repeat = Some(Repeat::Count(4_294_967_000)),
    );
    add_step(
        s,
        "fade lasting 4294967295 ms: plays, the duration clamps",
        fade_in(),
        BOTH,
        |st| st.dur_ms = u32::MAX,
    );
    add_step(
        s,
        "fade after a 4294967295 ms delay: plays, the delay clamps",
        fade_in(),
        BOTH,
        |st| st.delay_ms = u32::MAX,
    );
    let deep = Effect {
        class: Class::Entr,
        ooo: Some("ooo-entrance-probe-deep".into()),
        sub: None,
        pptx_id: Some(97),
        pptx_sub: 0,
        bhv: vec![Bhv::DeepFormula(400)],
    };
    add_step(
        s,
        "unknown preset with a formula nested 400 deep: can't compile, plays dissolve",
        (deep, Vocab::Fallback),
        BOTH,
        |_| {},
    );
    add_step(
        s,
        "fade nested in 64 container levels: dropped",
        fade_in(),
        BOTH,
        |st| {
            st.nest = HOSTILE_NEST;
            st.vocab = Vocab::Bounded;
        },
    );

    // Malformed timing values: an unknown value fails closed to the declared default, an overflowing one clamps.
    let i = feature(
        slides,
        "ft-hostile-malformed",
        "Malformed timing values: NaN, negative, overflowing (bounded)",
        BOTH,
        &["hostile"],
    );
    let mut malformed = pt("fade", &[]);
    malformed.adv_tm_raw = Some("-1");
    let mut malformed_odp = OTrans {
        advance: OAdv::Automatic(None),
        ..ot("fade", "crossfade", false)
    };
    malformed_odp.duration_raw = Some("-PT5S");
    slides[i].transition = Some(Trans {
        pptx: Some(malformed),
        odp: Some(malformed_odp),
        vocab: Some(("fade", params().with("through", "none"), false)),
        odp_params: None,
        fallback: "dissolve",
    });
    let s = &mut slides[i];
    s.rows.push("12.3/advance".into());
    add_step(
        s,
        "fade with an unparsable duration (NaN): plays with the default duration",
        fade_in(),
        BOTH,
        |st| {
            st.raw.pptx_dur = Some("NaN");
            st.raw.odp_dur = Some("NaN");
        },
    );
    add_step(
        s,
        "fade with a negative duration (-0.5 s): plays with the default duration",
        fade_in(),
        BOTH,
        |st| {
            st.raw.pptx_dur = Some("-500");
            st.raw.odp_dur = Some("-0.5s");
        },
    );
    add_step(
        s,
        "fade after an overflowing delay (99999999999 ms / s): plays, the delay clamps",
        fade_in(),
        BOTH,
        |st| {
            st.raw.pptx_delay = Some("99999999999");
            st.raw.odp_delay = Some("99999999999s");
        },
    );

    // A line fading in: its box has no height, and its arrowhead and stroke overflow it (a fade fades them too).
    let i = feature(
        slides,
        "ft-line-fade",
        "A line with an arrowhead fades in: its stroke and arrowhead overflow a box of zero height",
        BOTH,
        &["line"],
    );
    let s = &mut slides[i];
    add_step(s, "line: fade in on click", fade_in(), BOTH, |_| {});
    if let Some(line) = s.shapes.last_mut() {
        line.line = true;
    }
}

/// The container levels the nesting probe adds: past any sane timing depth (the IR proposes 32), within the XML
/// parser's 256 element levels.
pub const HOSTILE_NEST: u32 = 64;

/// A PowerPoint filter from `PPTX_FILTERS`, entering.
fn pptx_clip(name: &str) -> Clip {
    Clip::pptx(name, false).unwrap_or(Clip {
        pptx: None,
        ty: "fade",
        subtype: "crossfade",
        reverse: false,
        out: false,
    })
}

// ---------------------------------------------------------------------------------------------------------------
// Timeline layout shared by both writers: click groups, time slots and offsets.

pub struct Group {
    /// Group 0 that starts on its own (its first step is not a click).
    pub auto: bool,
    pub slots: Vec<Slot>,
}

pub struct Slot {
    pub begin_ms: u32,
    pub steps: Vec<usize>,
}

/// Groups the main-sequence steps of a slide that a format carries: a click opens a group, a with-previous step
/// joins the current slot, an after-previous step opens a slot at the group's latest end.
pub fn layout(steps: &[Step], fmt: Fmt) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    let mut ends: Vec<u32> = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        if !step.formats.has(fmt) || step.interactive {
            continue;
        }
        match step.trigger {
            Trigger::Click => {
                groups.push(Group {
                    auto: false,
                    slots: vec![Slot {
                        begin_ms: 0,
                        steps: vec![index],
                    }],
                });
                ends.push(0);
            }
            Trigger::With | Trigger::After if groups.is_empty() => {
                groups.push(Group {
                    auto: true,
                    slots: vec![Slot {
                        begin_ms: 0,
                        steps: vec![index],
                    }],
                });
                ends.push(0);
            }
            Trigger::With => {
                if let Some(slot) = groups.last_mut().and_then(|g| g.slots.last_mut()) {
                    slot.steps.push(index);
                }
            }
            Trigger::After => {
                let begin = ends.last().copied().unwrap_or(0);
                if let Some(group) = groups.last_mut() {
                    group.slots.push(Slot {
                        begin_ms: begin,
                        steps: vec![index],
                    });
                }
            }
        }
        let begin = groups
            .last()
            .and_then(|g| g.slots.last())
            .map(|s| s.begin_ms)
            .unwrap_or(0);
        if let Some(end) = ends.last_mut() {
            *end = (*end).max(
                begin
                    .saturating_add(step.delay_ms)
                    .saturating_add(step.span_ms()),
            );
        }
    }
    groups
}

/// The absolute start of each laid-out step inside its click group (`offset`).
pub fn offsets(groups: &[Group], count: usize) -> Vec<Option<u32>> {
    let mut out = vec![None; count];
    for group in groups {
        for slot in &group.slots {
            for &index in &slot.steps {
                out[index] = Some(slot.begin_ms);
            }
        }
    }
    out
}
