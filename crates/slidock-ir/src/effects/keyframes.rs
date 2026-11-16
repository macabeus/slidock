//! Tier-2 effects and motion paths. A preset the vocabulary does not name, whose primitives are known, plays as
//! keyframe tracks the compiler computed from the source's behaviours (PPTX `p:anim*`, ODP `anim:*`). Tracks animate
//! only what the player may animate: opacity, translate, rotate, scale, a blur filter, a clip reveal, and the
//! colours and font switches the deck path exposes as registered custom properties on the targeted element (its
//! text, fill and line colours; bold, italic and underline). A source's `set` (PPTX `p:set`, ODF `anim:set`) is a
//! switch at its begin ([`SetTrack`]), never a track of equal frames.

use std::fmt;

use serde::de::{self, SeqAccess, Visitor};
use serde::ser::SerializeTuple;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ts_rs::TS;

use super::build::{
    ClockParams, Dir4Params, IrisParams, OrientParams, SplitParams, StripsParams, WheelParams,
    WipeParams,
};
use super::limits;
use super::params::{HexColor, InOut};
use super::timing::Easing;
use crate::diagnostics::codes;
use crate::units::{Deg, Ms, Px, Ratio, UnitRatio};
use crate::validate::{Validate, Validator};

/// One keyframe: a value at an offset (0 to 1) of its track.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct Frame<V> {
    pub offset: UnitRatio,
    pub value: V,
}

/// A translation from the element's static position, in slide pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct Offset {
    pub x: Px,
    pub y: Px,
}

/// A scale factor per axis (1 = the element's size).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct Scale {
    pub x: Ratio,
    pub y: Ratio,
}

/// A shift of a colour in HSL, each part added to the colour's own: `h` turns the hue by degrees (positive turns
/// it forward), `s` and `l` add a fraction (−1 to 1) of full saturation and lightness, clamped to the range. All zero
/// is the colour itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct HslShift {
    pub h: Deg,
    pub s: Ratio,
    pub l: Ratio,
}

impl HslShift {
    /// No shift: the element's own colour.
    pub const NONE: HslShift = HslShift {
        h: Deg::ZERO,
        s: Ratio::ZERO,
        l: Ratio::ZERO,
    };

    /// `color` shifted: its hue turned, its saturation and lightness moved and clamped to [0, 1].
    pub fn apply(self, color: HexColor) -> HexColor {
        let value = color.value();
        let channel = |shift: u32| f64::from((value >> shift) & 0xff) / 255.0;
        let (h, s, l) = to_hsl(channel(16), channel(8), channel(0));
        let h = (h + self.h.to_f64()).rem_euclid(360.0);
        let s = (s + self.s.to_f64()).clamp(0.0, 1.0);
        let l = (l + self.l.to_f64()).clamp(0.0, 1.0);
        let (r, g, b) = from_hsl(h, s, l);
        let byte = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        HexColor::rgb(byte(r), byte(g), byte(b))
    }
}

/// Hue in degrees [0, 360), saturation and lightness of sRGB components.
fn to_hsl(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn hue_channel(p: f64, q: f64, t: f64) -> f64 {
    let t = t.rem_euclid(1.0);
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 1.0 / 2.0 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

fn from_hsl(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    if s == 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let t = h / 360.0;
    (
        hue_channel(p, q, t + 1.0 / 3.0),
        hue_channel(p, q, t),
        hue_channel(p, q, t - 1.0 / 3.0),
    )
}

/// Two colours mixed channel by channel in sRGB: `a` toward `b` by `f` (0 to 1), rounded.
pub(crate) fn mix_hex(a: HexColor, b: HexColor, f: f64) -> HexColor {
    let channel = |c: HexColor, shift: u32| f64::from((c.value() >> shift) & 0xff);
    let mix = |shift: u32| {
        let (x, y) = (channel(a, shift), channel(b, shift));
        (x + (y - x) * f).round().clamp(0.0, 255.0) as u8
    };
    HexColor::rgb(mix(16), mix(8), mix(0))
}

/// The element's own colour shifted, then mixed toward a colour: where a track between the two passes when a fold
/// samples it part-way (`by` 0 is the shifted colour, 1 is `toward`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct ColorMix {
    pub shift: HslShift,
    pub toward: HexColor,
    pub by: UnitRatio,
}

impl ColorMix {
    /// The colour it names on an element whose own colour is `own`.
    pub fn apply(self, own: HexColor) -> HexColor {
        mix_hex(self.shift.apply(own), self.toward, self.by.to_f64())
    }
}

/// A colour keyframe: a colour, the element's own colour shifted (`{"h":0,"s":0,"l":0}` is the colour itself), or
/// a shifted own colour mixed toward a colour. The deck builder computes each from the element's paint, so a deck's
/// tracks hold only `#rrggbb`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export_to = "ir/")]
pub enum ColorValue {
    Hex(HexColor),
    Shift(HslShift),
    Mix(ColorMix),
}

impl ColorValue {
    /// The colour it names on an element whose own colour is `own`.
    pub fn on(self, own: HexColor) -> HexColor {
        match self {
            ColorValue::Hex(hex) => hex,
            ColorValue::Shift(shift) => shift.apply(own),
            ColorValue::Mix(mix) => mix.apply(own),
        }
    }
}

/// Keyframes of one property over a span of the step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub struct Track<V> {
    /// From the step's start.
    pub begin_ms: Ms,
    pub duration_ms: Ms,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    #[ts(as = "Option<Easing>", optional)]
    pub easing: Option<Easing>,
    /// At least two, offsets rising from 0 to 1.
    pub frames: Vec<Frame<V>>,
}

/// The clip a reveal track draws, with the build recipes' parameters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "effect",
    content = "params",
    rename_all = "camelCase",
    deny_unknown_fields
)]
#[ts(export_to = "ir/")]
pub enum Reveal {
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
    Dissolve,
}

impl Validate for Reveal {
    fn validate(&self, v: &mut Validator) {
        if let Reveal::Wheel(wheel) = self {
            v.field("params", wheel);
        }
    }
}

/// A clip that opens (`in`, revealing the element) or closes (`out`) over a span of the step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub struct ClipTrack {
    pub begin_ms: Ms,
    pub duration_ms: Ms,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    #[ts(as = "Option<Easing>", optional)]
    pub easing: Option<Easing>,
    pub reveal: Reveal,
    pub transition: InOut,
}

/// The value a switch gives one property.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "property",
    content = "value",
    rename_all = "camelCase",
    deny_unknown_fields
)]
#[ts(export_to = "ir/")]
pub enum SetValue {
    Opacity(UnitRatio),
    /// The text colour (`--sd-color`).
    Text(ColorValue),
    /// The fill colour (`--sd-fill`).
    Fill(ColorValue),
    /// The line colour (`--sd-line`).
    Line(ColorValue),
    Bold(bool),
    Italic(bool),
    Underline(bool),
}

impl SetValue {
    /// The element's own value of the property, where a value names it: its own colour (no shift), full opacity.
    /// `None` for a font switch, whose own value the IR does not hold.
    pub fn own(&self) -> Option<SetValue> {
        let own = ColorValue::Shift(HslShift::NONE);
        Some(match self {
            SetValue::Opacity(_) => SetValue::Opacity(UnitRatio::from_steps(100_000)),
            SetValue::Text(_) => SetValue::Text(own),
            SetValue::Fill(_) => SetValue::Fill(own),
            SetValue::Line(_) => SetValue::Line(own),
            SetValue::Bold(_) | SetValue::Italic(_) | SetValue::Underline(_) => return None,
        })
    }
}

/// A switch (PPTX `p:set`, ODF `anim:set`): before `beginMs` the property shows what it showed (its static value, or
/// what an earlier step left), and from `beginMs` on it shows `to`, with no frame in between. It lasts `durationMs`
/// (a behaviour's own auto-reverse doubles it, the value unchanged) and holds after it, as every track holds its end.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub struct SetTrack {
    /// From the step's start.
    pub begin_ms: Ms,
    pub duration_ms: Ms,
    pub to: SetValue,
}

/// One tier-2 track, by the property it animates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "property", rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub enum Keyframes {
    Opacity(Track<UnitRatio>),
    Translate(Track<Offset>),
    /// Degrees clockwise from the element's static rotation.
    Rotate(Track<Deg>),
    Scale(Track<Scale>),
    /// `filter: blur()`, in slide pixels.
    Blur(Track<Px>),
    ClipPath(ClipTrack),
    /// The text colour (`--sd-color` on the deck path).
    Text(Track<ColorValue>),
    /// The fill colour (`--sd-fill`).
    Fill(Track<ColorValue>),
    /// The line colour (`--sd-line`).
    Line(Track<ColorValue>),
    /// Bold text (`--sd-bold`); each value holds until the next frame.
    Bold(Track<bool>),
    /// Italic text (`--sd-italic`); each value holds until the next frame.
    Italic(Track<bool>),
    /// Underlined text (`--sd-underline`); each value holds until the next frame.
    Underline(Track<bool>),
    /// A switch of one property at its begin.
    Set(SetTrack),
}

impl Keyframes {
    /// The frames of a track, or `None` for a clip, which has none.
    pub fn frame_count(&self) -> Option<usize> {
        Some(match self {
            Keyframes::Opacity(t) => t.frames.len(),
            Keyframes::Translate(t) => t.frames.len(),
            Keyframes::Rotate(t) => t.frames.len(),
            Keyframes::Scale(t) => t.frames.len(),
            Keyframes::Blur(t) => t.frames.len(),
            Keyframes::Text(t) | Keyframes::Fill(t) | Keyframes::Line(t) => t.frames.len(),
            Keyframes::Bold(t) | Keyframes::Italic(t) | Keyframes::Underline(t) => t.frames.len(),
            Keyframes::ClipPath(_) | Keyframes::Set(_) => return None,
        })
    }

    /// The track with `easing` as its own curve (a clip too); a switch has none.
    pub fn eased(self, easing: Option<Easing>) -> Keyframes {
        let Some(easing) = easing else {
            return self;
        };
        let easing = Some(easing);
        match self {
            Keyframes::Opacity(t) => Keyframes::Opacity(Track { easing, ..t }),
            Keyframes::Translate(t) => Keyframes::Translate(Track { easing, ..t }),
            Keyframes::Rotate(t) => Keyframes::Rotate(Track { easing, ..t }),
            Keyframes::Scale(t) => Keyframes::Scale(Track { easing, ..t }),
            Keyframes::Blur(t) => Keyframes::Blur(Track { easing, ..t }),
            Keyframes::Text(t) => Keyframes::Text(Track { easing, ..t }),
            Keyframes::Fill(t) => Keyframes::Fill(Track { easing, ..t }),
            Keyframes::Line(t) => Keyframes::Line(Track { easing, ..t }),
            Keyframes::Bold(t) => Keyframes::Bold(Track { easing, ..t }),
            Keyframes::Italic(t) => Keyframes::Italic(Track { easing, ..t }),
            Keyframes::Underline(t) => Keyframes::Underline(Track { easing, ..t }),
            Keyframes::ClipPath(clip) => Keyframes::ClipPath(ClipTrack { easing, ..clip }),
            set @ Keyframes::Set(_) => set,
        }
    }

    /// Where the track plays in the step: its begin and duration.
    pub fn span(&self) -> (Ms, Ms) {
        match self {
            Keyframes::Opacity(t) => (t.begin_ms, t.duration_ms),
            Keyframes::Translate(t) => (t.begin_ms, t.duration_ms),
            Keyframes::Rotate(t) => (t.begin_ms, t.duration_ms),
            Keyframes::Scale(t) => (t.begin_ms, t.duration_ms),
            Keyframes::Blur(t) => (t.begin_ms, t.duration_ms),
            Keyframes::Text(t) | Keyframes::Fill(t) | Keyframes::Line(t) => {
                (t.begin_ms, t.duration_ms)
            }
            Keyframes::Bold(t) | Keyframes::Italic(t) | Keyframes::Underline(t) => {
                (t.begin_ms, t.duration_ms)
            }
            Keyframes::ClipPath(clip) => (clip.begin_ms, clip.duration_ms),
            Keyframes::Set(set) => (set.begin_ms, set.duration_ms),
        }
    }

    /// The colour or font switch the track animates through the deck's hooks, if any (a switch's own property).
    pub fn paint(&self) -> Option<Paint> {
        Some(match self {
            Keyframes::Text(_)
            | Keyframes::Set(SetTrack {
                to: SetValue::Text(_),
                ..
            }) => Paint::Text,
            Keyframes::Fill(_)
            | Keyframes::Set(SetTrack {
                to: SetValue::Fill(_),
                ..
            }) => Paint::Fill,
            Keyframes::Line(_)
            | Keyframes::Set(SetTrack {
                to: SetValue::Line(_),
                ..
            }) => Paint::Line,
            Keyframes::Bold(_)
            | Keyframes::Set(SetTrack {
                to: SetValue::Bold(_),
                ..
            }) => Paint::Bold,
            Keyframes::Italic(_)
            | Keyframes::Set(SetTrack {
                to: SetValue::Italic(_),
                ..
            }) => Paint::Italic,
            Keyframes::Underline(_)
            | Keyframes::Set(SetTrack {
                to: SetValue::Underline(_),
                ..
            }) => Paint::Underline,
            _ => return None,
        })
    }
}

/// A paint a colour or font track animates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Paint {
    Text,
    Fill,
    Line,
    Bold,
    Italic,
    Underline,
}

fn span(v: &mut Validator, begin: Ms, duration: Ms) {
    v.field("beginMs", &begin);
    v.field("durationMs", &duration);
    if u64::from(begin.0) + u64::from(duration.0) > u64::from(Ms::MAX) {
        v.at("durationMs", |v| {
            v.invalid(codes::IR0008, "the track ends after an hour")
        });
    }
}

fn track<V>(v: &mut Validator, track: &Track<V>, value: impl Fn(&mut Validator, &V)) {
    span(v, track.begin_ms, track.duration_ms);
    v.field("easing", &track.easing);
    v.at("frames", |v| {
        v.array(track.frames.len(), 2, limits::KEYFRAMES);
        let mut last = None;
        for (i, frame) in track.frames.iter().enumerate() {
            v.at(i, |v| {
                v.field("offset", &frame.offset);
                if last.is_some_and(|last| frame.offset < last) {
                    v.at("offset", |v| {
                        v.invalid(codes::IR0001, "keyframe offsets must not decrease")
                    });
                }
                v.at("value", |v| value(v, &frame.value));
            });
            last = Some(frame.offset);
        }
        let ends = track.frames.first().map(|frame| frame.offset.steps()) == Some(0)
            && track.frames.last().map(|frame| frame.offset.steps()) == Some(100_000);
        if !track.frames.is_empty() && !ends {
            v.invalid(
                codes::IR0001,
                "keyframes must start at offset 0 and end at 1",
            );
        }
    });
}

fn color_value(v: &mut Validator, value: &ColorValue) {
    match value {
        ColorValue::Hex(_) => {}
        ColorValue::Shift(shift) => hsl_shift(v, shift),
        ColorValue::Mix(mix) => {
            v.at("shift", |v| hsl_shift(v, &mix.shift));
            v.field("by", &mix.by);
        }
    }
}

fn hsl_shift(v: &mut Validator, shift: &HslShift) {
    v.at("h", |v| {
        v.min(shift.h.to_f64(), -limits::HUE_SHIFT_DEGREES, false);
        v.max(shift.h.to_f64(), limits::HUE_SHIFT_DEGREES);
    });
    for (name, part) in [("s", shift.s), ("l", shift.l)] {
        v.at(name, |v| {
            v.min(part.to_f64(), -1.0, false);
            v.max(part.to_f64(), 1.0);
        });
    }
}

fn scale_value(v: &mut Validator, x: Ratio) {
    v.min(x.to_f64(), 0.0, false);
    v.max(x.to_f64(), limits::SCALE);
}

impl Validate for Keyframes {
    fn validate(&self, v: &mut Validator) {
        match self {
            Keyframes::Opacity(t) => track(v, t, |v, value| value.validate(v)),
            Keyframes::Translate(t) => track(v, t, |v, value| {
                v.field("x", &value.x);
                v.field("y", &value.y);
            }),
            Keyframes::Rotate(t) => track(v, t, |v, value| {
                v.min(value.to_f64(), -limits::SPIN_DEGREES, false);
                v.max(value.to_f64(), limits::SPIN_DEGREES);
            }),
            Keyframes::Scale(t) => track(v, t, |v, value| {
                v.at("x", |v| scale_value(v, value.x));
                v.at("y", |v| scale_value(v, value.y));
            }),
            Keyframes::Blur(t) => track(v, t, |v, value| {
                v.min(value.to_f64(), 0.0, false);
                v.max(value.to_f64(), limits::BLUR_PX);
            }),
            Keyframes::ClipPath(clip) => {
                span(v, clip.begin_ms, clip.duration_ms);
                v.field("easing", &clip.easing);
                v.field("reveal", &clip.reveal);
            }
            Keyframes::Text(t) | Keyframes::Fill(t) | Keyframes::Line(t) => {
                track(v, t, color_value);
            }
            Keyframes::Bold(t) | Keyframes::Italic(t) | Keyframes::Underline(t) => {
                track(v, t, |_, _| {});
            }
            Keyframes::Set(set) => {
                span(v, set.begin_ms, set.duration_ms);
                v.at("to", |v| match &set.to {
                    SetValue::Opacity(value) => v.field("value", value),
                    SetValue::Text(value) | SetValue::Fill(value) | SetValue::Line(value) => {
                        v.at("value", |v| color_value(v, value));
                    }
                    SetValue::Bold(_) | SetValue::Italic(_) | SetValue::Underline(_) => {}
                });
            }
        }
    }
}

/// One motion-path segment, in slide pixels from the element's static top-left corner. Prints as a JSON array:
/// `["M", x, y]`, `["L", x, y]`, `["C", x1, y1, x2, y2, x, y]`, `["Z"]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, TS)]
#[ts(
    export_to = "ir/",
    type = "[\"M\", number, number] | [\"L\", number, number] | [\"C\", number, number, number, number, number, number] | [\"Z\"]"
)]
pub enum PathSeg {
    MoveTo(Px, Px),
    LineTo(Px, Px),
    CurveTo(Px, Px, Px, Px, Px, Px),
    Close,
}

impl PathSeg {
    fn points(&self) -> Vec<Px> {
        match *self {
            PathSeg::MoveTo(x, y) | PathSeg::LineTo(x, y) => vec![x, y],
            PathSeg::CurveTo(x1, y1, x2, y2, x, y) => vec![x1, y1, x2, y2, x, y],
            PathSeg::Close => Vec::new(),
        }
    }

    fn command(&self) -> &'static str {
        match self {
            PathSeg::MoveTo(..) => "M",
            PathSeg::LineTo(..) => "L",
            PathSeg::CurveTo(..) => "C",
            PathSeg::Close => "Z",
        }
    }
}

impl Serialize for PathSeg {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let points = self.points();
        let mut tuple = serializer.serialize_tuple(1 + points.len())?;
        tuple.serialize_element(self.command())?;
        for point in &points {
            tuple.serialize_element(point)?;
        }
        tuple.end()
    }
}

struct PathSegVisitor;

impl<'de> Visitor<'de> for PathSegVisitor {
    type Value = PathSeg;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a path segment: [\"M\", x, y], [\"L\", x, y], [\"C\", x1, y1, x2, y2, x, y] or [\"Z\"]")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<PathSeg, A::Error> {
        let command: String = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;
        let count = match command.as_str() {
            "M" | "L" => 2,
            "C" => 6,
            "Z" => 0,
            _ => return Err(de::Error::unknown_variant(&command, &["M", "L", "C", "Z"])),
        };
        let mut points = Vec::with_capacity(count);
        for i in 0..count {
            let point: Px = seq
                .next_element()?
                .ok_or_else(|| de::Error::invalid_length(i + 1, &self))?;
            points.push(point);
        }
        if seq.next_element::<de::IgnoredAny>()?.is_some() {
            return Err(de::Error::invalid_length(count + 2, &self));
        }
        Ok(match (command.as_str(), points.as_slice()) {
            ("M", &[x, y]) => PathSeg::MoveTo(x, y),
            ("L", &[x, y]) => PathSeg::LineTo(x, y),
            ("C", &[x1, y1, x2, y2, x, y]) => PathSeg::CurveTo(x1, y1, x2, y2, x, y),
            _ => PathSeg::Close,
        })
    }
}

impl<'de> Deserialize<'de> for PathSeg {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_seq(PathSegVisitor)
    }
}

/// A motion path's bounds: 1 to [`limits::PATH_SEGMENTS`] segments, starting with a move.
pub(crate) fn validate_path(v: &mut Validator, path: &[PathSeg]) {
    v.array(path.len(), 1, limits::PATH_SEGMENTS);
    if path
        .first()
        .is_some_and(|first| !matches!(first, PathSeg::MoveTo(..)))
    {
        v.at(0, |v| {
            v.invalid(codes::IR0001, "a motion path starts with a move (M)")
        });
    }
    for (i, segment) in path.iter().enumerate() {
        v.at(i, |v| {
            for (j, point) in segment.points().iter().enumerate() {
                v.at(j + 1, |v| point.validate(v));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shift(h: f64, s: f64, l: f64) -> HslShift {
        HslShift {
            h: Deg::quantise(h).unwrap(),
            s: Ratio::quantise(s).unwrap(),
            l: Ratio::quantise(l).unwrap(),
        }
    }

    #[test]
    fn a_shift_turns_the_hue_and_moves_saturation_and_lightness() {
        let red = HexColor::rgb(0xff, 0, 0);
        assert_eq!(HslShift::NONE.apply(red), red);
        assert_eq!(shift(120.0, 0.0, 0.0).apply(red), HexColor::rgb(0, 0xff, 0));
        assert_eq!(
            shift(-120.0, 0.0, 0.0).apply(red),
            HexColor::rgb(0, 0, 0xff)
        );
        assert_eq!(
            shift(180.0, 0.0, 0.0).apply(red),
            HexColor::rgb(0, 0xff, 0xff)
        );
        // Lightness 0.5 − 0.25 = 0.25, saturation stays 1: #800000.
        assert_eq!(shift(0.0, 0.0, -0.25).apply(red), HexColor::rgb(0x80, 0, 0));
        // Saturation clamps at 0: a grey of the same lightness.
        assert_eq!(
            shift(0.0, -2.0, 0.0).apply(red),
            HexColor::rgb(0x80, 0x80, 0x80)
        );
        assert_eq!(shift(0.0, 0.0, 1.0).apply(red), HexColor::WHITE);
        let grey = HexColor::rgb(0x80, 0x80, 0x80);
        assert_eq!(shift(120.0, 0.0, 0.0).apply(grey), grey);
    }

    /// The middle frame of a registered `<color>` custom property animated between two `#rrggbb` keyframes, as
    /// Chromium 148, Firefox 150 (Playwright 1.60's image) and WebKit 26.4 (macOS and the image) compute it: each
    /// mixes legacy sRGB colours in sRGB (CSS Color 4 §12.1), so a folded mix is the colour the player interpolates.
    /// An Oklab mix differs on every pair (`#000000`→`#ffffff` is `#636363` there).
    #[test]
    fn a_mix_is_the_colour_browsers_interpolate_halfway() {
        let hex = |text: &str| HexColor::parse(text).unwrap();
        for (a, b, measured) in [
            ("#ff0000", "#0000ff", HexColor::rgb(128, 0, 128)),
            ("#2f6fdf", "#ffcc00", HexColor::rgb(151, 158, 112)),
            ("#000000", "#ffffff", HexColor::rgb(128, 128, 128)),
            ("#00ff00", "#ff00ff", HexColor::rgb(128, 128, 128)),
        ] {
            assert_eq!(mix_hex(hex(a), hex(b), 0.5), measured, "{a} → {b}");
        }
    }
}
