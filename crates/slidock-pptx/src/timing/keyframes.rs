//! Behaviours as keyframes (tier 2): a preset the vocabulary does not name plays the tracks its own
//! behaviours compile to. Positions and sizes (`ppt_x`, `ppt_y`, `ppt_w`, `ppt_h`) become translations and scales of
//! the shape's box, rotations and opacities pass through, `animEffect` filters become clips, and
//! motion paths are sampled into translations. Colours (`animClr`, and `set` of `style.color`, `fillcolor`,
//! `stroke.color`) become text, fill and line colour tracks, a change by an HSL amount sampled as shifts of the
//! element's own colour; `style.fontWeight`, `style.fontStyle` and `style.textDecorationUnderline` become bold,
//! italic and underline switches. A `set` is a switch at its begin ([`SetTrack`]). A behaviour's own time
//! manipulations fold into its frames: `autoRev` runs them forward then back over twice its span, `accel`/`decel`
//! and a `tmFilter` reshape their timing ([`Retime`]). Formulas are evaluated here, sampled where they bend. A
//! behaviour the player cannot animate (a scheme colour, an RGB amount, an attribute it does not know) is left out
//! and named ([`Compiled::lost`]); a step none of whose behaviours compile plays its fallback. The switches that show
//! a fill or line a colour paints (`fill.on`, `fill.type`, `stroke.on`) are the deck builder's: it turns the paint
//! on, or plays the fallback where the element lacks it.

use slidock_ir::effects::limits::HUE_SHIFT_DEGREES;
use slidock_ir::effects::limits::{KEYFRAMES, PATH_SEGMENTS, SPIN_DEGREES, SPOKES, TRACKS};
use slidock_ir::effects::{
    ClipTrack, ColorValue, Diagonal, Dir, Dir4, Dir4Params, Easing, Frame, HslShift, InOut,
    IrisParams, IrisShape, Keyframes, Offset, Orient, OrientParams, PathSeg, Retime, Reveal, Scale,
    SetTrack, SetValue, SplitParams, StripsParams, Track, WheelParams, WipeParams,
};
use slidock_ir::units::{Deg, Ms, Px, Ratio, UnitRatio};

use super::behavior::{Amounts, Behavior, ColorBy, Common, Tav, Value};
use super::fmla::{self, Env, Expr, Rng};
use super::values::Time;

/// Where a step plays: the slide in pixels and, when known, the target's box as fractions of the slide (its
/// centre and size, what `#ppt_x`… read) and its rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stage {
    pub w_px: f64,
    pub h_px: f64,
    pub shape: Option<ShapeBox>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ShapeBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub rotation: f64,
}

/// What an `animEffect` filter plays (ECMA-376 Part 1 §19.5.3 `filter`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Clip {
    Reveal(Reveal),
    Fade,
    /// The content slides into (or out of) place from just outside its box, moving along the direction.
    Slide(Dir),
}

fn side(name: &str) -> Option<Dir> {
    Some(match name {
        "up" | "fromTop" => Dir::Up,
        "down" | "fromBottom" => Dir::Down,
        "left" | "fromLeft" => Dir::Left,
        "right" | "fromRight" => Dir::Right,
        _ => return None,
    })
}

fn orient(name: &str) -> Option<Orient> {
    match name {
        "horizontal" => Some(Orient::Horizontal),
        "vertical" => Some(Orient::Vertical),
        _ => None,
    }
}

fn in_out(name: &str) -> Option<InOut> {
    match name {
        "in" => Some(InOut::In),
        "out" => Some(InOut::Out),
        _ => None,
    }
}

/// The clip a filter plays, its names read as LibreOffice reads them (`pptfilterhelpers.cxx`): `wipe(X)` starts at
/// side X (its edge moves away from it), `strips(XY)` travels toward corner XY, `barn`/`plus`/`diamond`/`circle`/`box`
/// open outward (`out`) or close inward (`in`), `slide(fromX)` enters from side X and leaves toward it (`exiting`).
/// `image` dissolves. An in/out clip (iris, split) names the entrance it plays, whichever way it runs: leaving
/// (`transition="out"`, which ECMA-376 defines only as shown at the start and hidden at the end) it plays the filter's
/// progress reversed, SMIL's `mode="out"` as LibreOffice's engine plays it, so `box(in)` leaving opens a hole from the
/// centre, the `in` entrance backwards. `None` for a filter it does not know.
pub(crate) fn filter(text: &str, exiting: bool) -> Option<Clip> {
    let text = text.trim();
    let (name, arg) = match text.split_once('(') {
        Some((name, rest)) => (name, rest.strip_suffix(')')?),
        None => (text, ""),
    };
    let reveal = |reveal| Some(Clip::Reveal(reveal));
    match (name, arg) {
        ("fade", "") => Some(Clip::Fade),
        ("dissolve" | "image", "") => reveal(Reveal::Dissolve),
        ("wedge", "") => reveal(Reveal::Wedge),
        ("wipe", side_name) => {
            let dir = Dir::from_side(side(side_name).filter(|_| !side_name.starts_with("from"))?);
            reveal(Reveal::Wipe(WipeParams { dir, soft: true }))
        }
        ("wheel", spokes) => {
            let spokes = spokes
                .parse::<u8>()
                .ok()
                .filter(|n| (1..=SPOKES).contains(n))?;
            reveal(Reveal::Wheel(WheelParams { spokes, cw: true }))
        }
        ("strips", corner) => {
            let corner = match corner {
                "downLeft" => Diagonal::LeftDown,
                "upLeft" => Diagonal::LeftUp,
                "downRight" => Diagonal::RightDown,
                "upRight" => Diagonal::RightUp,
                _ => return None,
            };
            reveal(Reveal::Strips(StripsParams { corner }))
        }
        ("barn", split) => {
            let (motion, line) = if let Some(line) = split.strip_prefix("in") {
                (InOut::In, line)
            } else {
                (InOut::Out, split.strip_prefix("out")?)
            };
            let orient = orient(&line.to_ascii_lowercase())?;
            reveal(Reveal::Split(SplitParams {
                orient,
                in_out: motion,
            }))
        }
        ("blinds", line) => reveal(Reveal::Blinds(OrientParams {
            orient: orient(line)?,
        })),
        ("randombar", line) => reveal(Reveal::RandomBars(OrientParams {
            orient: orient(line)?,
        })),
        ("checkerboard", way) => {
            let dir = match way {
                "across" => Dir4::Right,
                "down" => Dir4::Down,
                _ => return None,
            };
            reveal(Reveal::Checker(Dir4Params { dir }))
        }
        ("plus" | "diamond" | "circle" | "box", motion) => {
            let shape = match name {
                "plus" => IrisShape::Plus,
                "diamond" => IrisShape::Diamond,
                "circle" => IrisShape::Circle,
                _ => IrisShape::Box,
            };
            reveal(Reveal::Iris(IrisParams {
                shape,
                in_out: in_out(motion)?,
            }))
        }
        ("slide", from) => {
            let from = side(from).filter(|_| from.starts_with("from"))?;
            Some(Clip::Slide(if exiting {
                from
            } else {
                Dir::from_side(from)
            }))
        }
        _ => None,
    }
}

/// A motion path (`animMotion@path`): `M`, `L`, `C` and `Z` commands (lowercase relative), in fractions of the
/// slide from the shape's position, ending at `E` or the end of the text. Coordinates become slide pixels. `None`
/// for a path that is not one, does not start with a move, or has more than the bound of segments.
pub(crate) fn path(text: &str, stage: &Stage) -> Option<Vec<PathSeg>> {
    let mut words = text.split_ascii_whitespace().peekable();
    let mut out = Vec::new();
    let mut at = (0.0f64, 0.0f64);
    let mut start = (0.0f64, 0.0f64);
    let mut command: Option<char> = None;
    let px = |(x, y): (f64, f64)| -> Option<(Px, Px)> {
        Some((
            Px::quantise(x * stage.w_px).ok()?,
            Px::quantise(y * stage.h_px).ok()?,
        ))
    };
    while let Some(&word) = words.peek() {
        let mut chars = word.chars();
        let letter = chars.next().filter(|c| c.is_ascii_alphabetic());
        if let Some(letter) = letter {
            words.next();
            if letter == 'E' || letter == 'e' {
                break;
            }
            command = Some(letter);
            let rest = chars.as_str();
            if !rest.is_empty() {
                return None;
            }
            if letter == 'Z' || letter == 'z' {
                out.push(PathSeg::Close);
                at = start;
                command = None;
                if out.len() > PATH_SEGMENTS {
                    return None;
                }
                continue;
            }
        }
        let command = command?;
        let relative = command.is_ascii_lowercase();
        let count = match command.to_ascii_uppercase() {
            'M' | 'L' => 2,
            'C' => 6,
            _ => return None,
        };
        let mut numbers = Vec::with_capacity(count);
        for _ in 0..count {
            let value: f64 = words.next()?.parse().ok()?;
            if !value.is_finite() {
                return None;
            }
            numbers.push(value);
        }
        let point = |i: usize| -> (f64, f64) {
            let x = numbers.get(i).copied().unwrap_or(0.0);
            let y = numbers.get(i + 1).copied().unwrap_or(0.0);
            if relative {
                (at.0 + x, at.1 + y)
            } else {
                (x, y)
            }
        };
        let segment = match command.to_ascii_uppercase() {
            'M' => {
                let p = point(0);
                start = p;
                at = p;
                let (x, y) = px(p)?;
                PathSeg::MoveTo(x, y)
            }
            'L' => {
                let p = point(0);
                at = p;
                let (x, y) = px(p)?;
                PathSeg::LineTo(x, y)
            }
            _ => {
                let (c1, c2, p) = (point(0), point(2), point(4));
                at = p;
                let ((x1, y1), (x2, y2), (x, y)) = (px(c1)?, px(c2)?, px(p)?);
                PathSeg::CurveTo(x1, y1, x2, y2, x, y)
            }
        };
        if out.is_empty() && !matches!(segment, PathSeg::MoveTo(..)) {
            return None;
        }
        out.push(segment);
        if out.len() > PATH_SEGMENTS {
            return None;
        }
    }
    (!out.is_empty()).then_some(out)
}

fn easing(common: &Common) -> Option<Easing> {
    let (accel, decel) = (common.accel.unwrap_or(0), common.decel.unwrap_or(0));
    if accel == 0 && decel == 0 || accel + decel > 100_000 {
        return None;
    }
    Some(Easing::AccelDecel {
        accel: UnitRatio::from_steps(i64::from(accel)),
        decel: UnitRatio::from_steps(i64::from(decel)),
    })
}

/// A behaviour's span inside the step: its start and duration; a duration it does not give is the step's.
fn span(common: &Common, step: Ms) -> Option<(Ms, Ms)> {
    let begin = match common.begin {
        Some(Time::Ms(ms)) => ms,
        None => Ms(0),
        Some(Time::Indefinite) => return None,
    };
    let duration = match common.dur {
        Some(Time::Ms(ms)) => ms,
        None | Some(Time::Indefinite) => step,
    };
    (u64::from(begin.0) + u64::from(duration.0) <= u64::from(Ms::MAX)).then_some((begin, duration))
}

fn offset(i: usize, count: usize) -> UnitRatio {
    let steps = if count <= 1 {
        0
    } else {
        (i as i64 * 100_000) / (count as i64 - 1)
    };
    UnitRatio::from_steps(steps)
}

/// The value of an `anim` keyframe value: a number, or a formula over the shape's box.
fn scalar(value: &Value, env: &Env, rng: &mut Rng) -> Option<f64> {
    match value {
        Value::Num(x) => Some(*x),
        Value::Str(text) => fmla::parse(text).ok()?.eval(env, rng).ok(),
        Value::Bool(_) | Value::Color(_) => None,
    }
}

/// Samples an `anim`'s value at `samples` evenly spaced points: keyframe values interpolated (or held, for a
/// discrete one), each segment shaped by its keyframe's formula with `$` the interpolated value.
fn sample_anim(
    tavs: &[Tav],
    from: Option<&str>,
    to: Option<&str>,
    by: Option<&str>,
    discrete: bool,
    env: &Env,
    rng: &mut Rng,
) -> Option<Vec<(UnitRatio, f64)>> {
    let mut keys: Vec<(f64, f64, Option<Expr>)> = Vec::new();
    if tavs.is_empty() {
        let read = |text: Option<&str>, rng: &mut Rng| -> Option<Option<f64>> {
            match text {
                None => Some(None),
                Some(text) => Some(Some(fmla::parse(text).ok()?.eval(env, rng).ok()?)),
            }
        };
        let (from, to, by) = (read(from, rng)?, read(to, rng)?, read(by, rng)?);
        let start = from?;
        let end = to.or_else(|| by.map(|by| start + by))?;
        keys.push((0.0, start, None));
        keys.push((1.0, end, None));
    } else {
        if tavs.len() > KEYFRAMES {
            return None;
        }
        let count = tavs.len();
        for (i, tav) in tavs.iter().enumerate() {
            let tm = match tav.tm {
                Some(tm) => f64::from(tm) / 100_000.0,
                None if count == 1 => 0.0,
                None => i as f64 / (count - 1) as f64,
            };
            let value = scalar(tav.value.as_ref()?, env, rng)?;
            let formula = match &tav.fmla {
                Some(text) => Some(fmla::parse(text).ok()?),
                None => None,
            };
            keys.push((tm, value, formula));
        }
    }
    if keys.windows(2).any(|pair| match pair {
        [a, b] => b.0 < a.0,
        _ => false,
    }) {
        return None;
    }
    let shaped = keys.iter().any(|(_, _, formula)| formula.is_some());
    let times: Vec<f64> = if shaped {
        (0..KEYFRAMES)
            .map(|i| i as f64 / (KEYFRAMES - 1) as f64)
            .collect()
    } else {
        let mut times: Vec<f64> = keys.iter().map(|(tm, _, _)| *tm).collect();
        if times.first().is_some_and(|first| *first > 0.0) {
            times.insert(0, 0.0);
        }
        if times.last().is_some_and(|last| *last < 1.0) {
            times.push(1.0);
        }
        times
    };
    if times.len() > KEYFRAMES {
        return None;
    }
    let mut out = Vec::with_capacity(times.len());
    for t in times {
        let segment = keys.iter().rposition(|(tm, _, _)| *tm <= t).unwrap_or(0);
        let (t0, v0, formula) = keys.get(segment)?;
        let value = match keys.get(segment + 1) {
            Some((t1, v1, _)) if !discrete && *t1 > *t0 => v0 + (v1 - v0) * ((t - t0) / (t1 - t0)),
            _ => *v0,
        };
        let value = match formula {
            Some(formula) => formula.eval(&Env { value, ..*env }, rng).ok()?,
            None => value,
        };
        let offset = UnitRatio::quantise(t).ok()?;
        out.push((offset, value));
    }
    Some(out)
}

fn track<V>(common: &Common, step: Ms, frames: Vec<Frame<V>>) -> Option<Track<V>> {
    let (begin_ms, duration_ms) = span(common, step)?;
    if frames.len() < 2 || frames.len() > KEYFRAMES {
        return None;
    }
    Some(Track {
        begin_ms,
        duration_ms,
        easing: easing(common),
        frames,
    })
}

fn frames<V>(values: Vec<V>) -> Vec<Frame<V>> {
    let count = values.len();
    values
        .into_iter()
        .enumerate()
        .map(|(i, value)| Frame {
            offset: offset(i, count),
            value,
        })
        .collect()
}

fn unit(x: f64) -> Option<UnitRatio> {
    (0.0..=1.0)
        .contains(&x)
        .then(|| UnitRatio::quantise(x).ok())
        .flatten()
}

fn scale_value(x: f64) -> Option<Ratio> {
    (0.0..=slidock_ir::effects::limits::SCALE)
        .contains(&x)
        .then(|| Ratio::quantise(x).ok())
        .flatten()
}

fn degrees(x: f64) -> Option<Deg> {
    (x.abs() <= SPIN_DEGREES)
        .then(|| Deg::quantise(x).ok())
        .flatten()
}

/// One axis of a translation or a scale, before the axes are paired.
struct Axis {
    begin: Ms,
    duration: Ms,
    easing: Option<Easing>,
    retime: Retime,
    frames: Vec<(UnitRatio, f64)>,
}

/// The value of an axis at `t`, linearly between its frames.
fn at(axis: &Axis, t: UnitRatio) -> f64 {
    let index = axis.frames.iter().rposition(|(offset, _)| *offset <= t);
    let Some(index) = index else {
        return axis.frames.first().map_or(0.0, |(_, v)| *v);
    };
    match (axis.frames.get(index), axis.frames.get(index + 1)) {
        (Some((t0, v0)), Some((t1, v1))) if t1 > t0 => {
            let f = (t.to_f64() - t0.to_f64()) / (t1.to_f64() - t0.to_f64());
            v0 + (v1 - v0) * f
        }
        (Some((_, v0)), _) => *v0,
        _ => 0.0,
    }
}

/// A paired track's span, easing and frames (offset, x, y).
type Paired = (Ms, Ms, Option<Easing>, Vec<(UnitRatio, f64, f64)>);

/// Pairs an x axis with a y axis of the same span into one track's frames; an axis alone pairs with `rest`.
fn pair(x: Option<&Axis>, y: Option<&Axis>, rest: f64) -> Option<Paired> {
    let lead = x.or(y)?;
    let mut offsets: Vec<UnitRatio> = x
        .iter()
        .chain(y.iter())
        .flat_map(|axis| axis.frames.iter().map(|(offset, _)| *offset))
        .collect();
    offsets.sort();
    offsets.dedup();
    if offsets.len() > KEYFRAMES {
        return None;
    }
    let frames = offsets
        .into_iter()
        .map(|t| {
            (
                t,
                x.map_or(rest, |axis| at(axis, t)),
                y.map_or(rest, |axis| at(axis, t)),
            )
        })
        .collect();
    Some((lead.begin, lead.duration, lead.easing, frames))
}

/// Frames a colour change that turns the hue is sampled at, so the hue travels its own way round.
const HUE_FRAMES: usize = 9;

/// The colour track an attribute names: `style.color` the text, `fillcolor`/`fill.color` the fill, `stroke.color`
/// the line.
fn color_track(attr: &str) -> Option<fn(Track<ColorValue>) -> Keyframes> {
    Some(match attr {
        "style.color" => Keyframes::Text,
        "fillcolor" | "fill.color" => Keyframes::Fill,
        "stroke.color" => Keyframes::Line,
        _ => return None,
    })
}

/// A font switch's track, and how its values read.
type Switch = (fn(Track<bool>) -> Keyframes, fn(&str) -> Option<bool>);

/// The font switch an attribute names, and how its values read: `None` for a value it does not know.
fn font_switch(attr: &str) -> Option<Switch> {
    Some(match attr {
        "style.fontWeight" => (Keyframes::Bold, |v| match v.trim() {
            "bold" => Some(true),
            "normal" => Some(false),
            _ => None,
        }),
        "style.fontStyle" => (Keyframes::Italic, |v| match v.trim() {
            "italic" | "oblique" => Some(true),
            "normal" => Some(false),
            _ => None,
        }),
        "style.textDecorationUnderline" => (Keyframes::Underline, |v| match v.trim() {
            "true" | "single" | "solid" => Some(true),
            "false" | "none" => Some(false),
            _ => None,
        }),
        _ => return None,
    })
}

/// The switch a colour `set` gives: `style.color` the text, `fillcolor`/`fill.color` the fill, `stroke.color` the
/// line.
fn color_set(attr: &str) -> Option<fn(ColorValue) -> SetValue> {
    Some(match attr {
        "style.color" => SetValue::Text,
        "fillcolor" | "fill.color" => SetValue::Fill,
        "stroke.color" => SetValue::Line,
        _ => return None,
    })
}

/// A font switch's value, and how its source value reads.
type SwitchSet = (fn(bool) -> SetValue, fn(&str) -> Option<bool>);

/// The switch a font `set` gives, and how its value reads.
fn switch_set(attr: &str) -> Option<SwitchSet> {
    let (_, read) = font_switch(attr)?;
    let make: fn(bool) -> SetValue = match attr {
        "style.fontWeight" => SetValue::Bold,
        "style.fontStyle" => SetValue::Italic,
        _ => SetValue::Underline,
    };
    Some((make, read))
}

/// Whether an attribute changes the text's font family or size, which moves the layout: no track plays it.
pub(crate) fn changes_layout(attr: &str) -> bool {
    matches!(attr, "style.fontFamily" | "style.fontSize")
}

fn switch_value(value: &Value, read: fn(&str) -> Option<bool>) -> Option<bool> {
    match value {
        Value::Str(text) => read(text),
        Value::Bool(flag) => Some(*flag),
        Value::Num(_) | Value::Color(_) => None,
    }
}

/// An HSL amount `t` of the way: `None` past the shift bounds.
fn hsl_shift(h: f64, s: f64, l: f64, t: f64) -> Option<HslShift> {
    let in_bounds = h.abs() <= HUE_SHIFT_DEGREES && s.abs() <= 1.0 && l.abs() <= 1.0;
    in_bounds
        .then(|| {
            Some(HslShift {
                h: Deg::quantise(h * t).ok()?,
                s: Ratio::quantise(s * t).ok()?,
                l: Ratio::quantise(l * t).ok()?,
            })
        })
        .flatten()
}

/// A colour change's frames: from `from` (the element's own colour when absent) to `to`, or by an HSL amount,
/// sampled where it turns the hue. A colour from `from` shifts at compile time; the element's own colour shifts
/// when the deck is built.
fn color_frames(
    from: Option<&Value>,
    to: Option<&Value>,
    by: Option<&ColorBy>,
) -> Option<Vec<Frame<ColorValue>>> {
    let start = match from {
        None => None,
        Some(Value::Color(Some(hex))) => Some(*hex),
        Some(_) => return None,
    };
    let at = |shift: HslShift| match start {
        Some(hex) => ColorValue::Hex(shift.apply(hex)),
        None => ColorValue::Shift(shift),
    };
    match (to, by) {
        (Some(Value::Color(Some(hex))), _) => Some(frames(vec![
            start.map_or(ColorValue::Shift(HslShift::NONE), ColorValue::Hex),
            ColorValue::Hex(*hex),
        ])),
        (Some(_), _) => None,
        (None, Some(ColorBy::Hsl { h, s, l })) => {
            let count = if *h == 0.0 { 2 } else { HUE_FRAMES };
            let values = (0..count)
                .map(|i| {
                    let t = i as f64 / (count - 1) as f64;
                    hsl_shift(*h, *s, *l, t).map(at)
                })
                .collect::<Option<Vec<_>>>()?;
            Some(frames(values))
        }
        (None, Some(ColorBy::Rgb) | None) => None,
    }
}

/// The frames of an `anim` of a font switch: its keyframes' values, each held until the next; the last holds to
/// the end. `None` when a value does not read or the first keyframe is not at the start.
fn switch_frames(tavs: &[Tav], read: fn(&str) -> Option<bool>) -> Option<Vec<Frame<bool>>> {
    let count = tavs.len();
    let mut out: Vec<Frame<bool>> = Vec::with_capacity(count + 1);
    for (i, tav) in tavs.iter().enumerate() {
        let offset = tav.tm.map_or_else(
            || offset(i, count),
            |tm| UnitRatio::from_steps(i64::from(tm)),
        );
        if out.last().is_some_and(|last| offset < last.offset) || (i == 0 && offset.steps() != 0) {
            return None;
        }
        out.push(Frame {
            offset,
            value: switch_value(tav.value.as_ref()?, read)?,
        });
    }
    let last = *out.last()?;
    if last.offset.steps() != 100_000 {
        out.push(Frame {
            offset: UnitRatio::from_steps(100_000),
            value: last.value,
        });
    }
    Some(out)
}

/// What a behaviour adds: a finished track, or one axis of a translation or a scale.
enum Part {
    Track(Keyframes, Retime),
    /// Tracks already retimed (a switch that does not hold, and its way back).
    Done(Vec<Keyframes>),
    MoveX(Axis),
    MoveY(Axis),
    SizeX(Axis),
    SizeY(Axis),
}

/// The time manipulations a behaviour applies to itself, in a step of `step` milliseconds; a repeat that does not
/// fold is left out (and named by [`compile`]).
fn retime_of(common: &Common, step: Ms) -> Retime {
    let duration = span(common, step).map_or(step, |(_, duration)| duration);
    common.retime(duration).0
}

/// `track` with its behaviour's own time folded into it; a repeat that does not fit the frames plays once, named in
/// `lost`. `None` when nothing fits.
fn fold(track: &Keyframes, retime: &Retime, lost: &mut Vec<String>) -> Option<Keyframes> {
    let (folded, once) = track.folded(retime, frames_ok)?;
    if once {
        lost.push(format!("the repeat of its {} animation", track_name(track)));
    }
    Some(folded)
}

/// Whether a behaviour shows the fill or line a colour animation paints (`fill.on`, `fill.type` solid,
/// `stroke.on`): the deck builder's to play.
fn turns_paint_on(behavior: &Behavior) -> bool {
    let Behavior::Set { to: Some(to), .. } = behavior else {
        return false;
    };
    let on = match to {
        Value::Str(text) => matches!(text.trim(), "true" | "solid"),
        Value::Bool(flag) => *flag,
        Value::Num(_) | Value::Color(_) => false,
    };
    on && matches!(behavior.attr(), Some("fill.on" | "fill.type" | "stroke.on"))
}

/// What tier 2 compiled: the tracks, and the behaviours they leave out, named.
#[derive(Debug, Default)]
pub(crate) struct Compiled {
    pub tracks: Vec<Keyframes>,
    pub lost: Vec<String>,
}

/// The tracks `behaviors` compile to, for a step of `step` milliseconds that `exiting` or not; `seed` serves the
/// formulas' `rand`. No track when nothing compiles, or when more than [`TRACKS`] would.
pub(crate) fn compile(
    behaviors: &[Behavior],
    exiting: bool,
    step: Ms,
    stage: &Stage,
    seed: u64,
) -> Compiled {
    let mut rng = Rng::new(seed);
    let mut parts = Vec::new();
    let mut lost: Vec<String> = Vec::new();
    for behavior in behaviors {
        if behavior.visibility().is_some()
            || turns_paint_on(behavior)
            || behavior.attr().is_some_and(changes_layout)
        {
            continue;
        }
        let holds = behavior.common().is_none_or(|common| common.holds);
        if let Some(common) = behavior.common() {
            let duration = span(common, step).map_or(step, |(_, duration)| duration);
            if common.retime(duration).1 {
                lost.push(format!("the repeat of its {} animation", behavior.label()));
            }
        }
        match compile_one(behavior, exiting, step, stage, &mut rng) {
            // A switch that does not hold gives the element its own value back when it ends.
            Some(Part::Track(Keyframes::Set(set), retime)) if !holds => {
                let Some(switch) = fold(&Keyframes::Set(set.clone()), &retime, &mut lost) else {
                    lost.push(format!("its {} animation", behavior.label()));
                    continue;
                };
                let (begin, length) = switch.span();
                let mut switches = vec![switch];
                match (set.to.own(), begin.0.checked_add(length.0)) {
                    (Some(own), Some(end)) => switches.push(Keyframes::Set(SetTrack {
                        begin_ms: Ms(end),
                        duration_ms: Ms(0),
                        to: own,
                    })),
                    _ => lost.push(format!("the end of its {} switch", behavior.label())),
                }
                parts.push(Part::Done(switches));
            }
            Some(part) => parts.push(part),
            None => lost.push(format!("its {} animation", behavior.label())),
        }
    }
    let mut tracks = Vec::new();
    let mut moves: Vec<(bool, Axis)> = Vec::new();
    let mut sizes: Vec<(bool, Axis)> = Vec::new();
    for part in parts {
        match part {
            Part::Track(track, retime) => {
                let reverses = retime.auto_reverse;
                match fold(&track, &retime, &mut lost) {
                    Some(track) => tracks.push(track),
                    None => lost.push(format!(
                        "its {} animation{}",
                        track_name(&track),
                        if reverses { " and its reversal" } else { "" }
                    )),
                }
            }
            Part::Done(done) => tracks.extend(done),
            Part::MoveX(axis) => moves.push((true, axis)),
            Part::MoveY(axis) => moves.push((false, axis)),
            Part::SizeX(axis) => sizes.push((true, axis)),
            Part::SizeY(axis) => sizes.push((false, axis)),
        }
    }
    for (axes, scale) in [(moves, false), (sizes, true)] {
        let what = if scale { "size" } else { "position" };
        let mut axes: Vec<Option<(bool, Axis)>> = axes.into_iter().map(Some).collect();
        for i in 0..axes.len() {
            let Some((is_x, first)) = axes.get_mut(i).and_then(Option::take) else {
                continue;
            };
            let partner = axes.iter_mut().skip(i + 1).find(|other| {
                other.as_ref().is_some_and(|(other_x, axis)| {
                    *other_x != is_x
                        && axis.begin == first.begin
                        && axis.duration == first.duration
                        && axis.retime == first.retime
                })
            });
            let second = partner.and_then(Option::take).map(|(_, axis)| axis);
            let (x, y) = if is_x {
                (Some(&first), second.as_ref())
            } else {
                (second.as_ref(), Some(&first))
            };
            let rest = if scale { 1.0 } else { 0.0 };
            let Some((begin_ms, duration_ms, easing, points)) = pair(x, y, rest) else {
                lost.push(format!("its {what} animation"));
                continue;
            };
            let built = if scale {
                points
                    .into_iter()
                    .map(|(offset, x, y)| {
                        Some(Frame {
                            offset,
                            value: Scale {
                                x: scale_value(x)?,
                                y: scale_value(y)?,
                            },
                        })
                    })
                    .collect::<Option<Vec<_>>>()
                    .map(|frames| {
                        Keyframes::Scale(Track {
                            begin_ms,
                            duration_ms,
                            easing,
                            frames,
                        })
                    })
            } else {
                points
                    .into_iter()
                    .map(|(offset, x, y)| {
                        Some(Frame {
                            offset,
                            value: Offset {
                                x: Px::quantise(x).ok()?,
                                y: Px::quantise(y).ok()?,
                            },
                        })
                    })
                    .collect::<Option<Vec<_>>>()
                    .map(|frames| {
                        Keyframes::Translate(Track {
                            begin_ms,
                            duration_ms,
                            easing,
                            frames,
                        })
                    })
            };
            match built
                .filter(frames_ok)
                .and_then(|track| fold(&track, &first.retime, &mut lost))
            {
                Some(track) => tracks.push(track),
                None => lost.push(format!("its {what} animation")),
            }
        }
    }
    if tracks.len() > TRACKS {
        lost.push(format!("its tracks past the first {TRACKS}"));
        tracks.clear();
    }
    Compiled { tracks, lost }
}

/// What a track animates, as a diagnostic names it.
fn track_name(track: &Keyframes) -> &'static str {
    match track {
        Keyframes::Opacity(_) => "opacity",
        Keyframes::Translate(_) => "position",
        Keyframes::Rotate(_) => "rotation",
        Keyframes::Scale(_) => "size",
        Keyframes::Blur(_) => "blur",
        Keyframes::ClipPath(_) => "clip",
        Keyframes::Text(_) => "text colour",
        Keyframes::Fill(_) => "fill colour",
        Keyframes::Line(_) => "line colour",
        Keyframes::Bold(_) => "font weight",
        Keyframes::Italic(_) => "font style",
        Keyframes::Underline(_) => "underline",
        Keyframes::Set(_) => "switch",
    }
}

fn frames_ok(track: &Keyframes) -> bool {
    (2..=KEYFRAMES).contains(&track.frame_count().unwrap_or(2))
}

fn compile_one(
    behavior: &Behavior,
    exiting: bool,
    step: Ms,
    stage: &Stage,
    rng: &mut Rng,
) -> Option<Part> {
    let env = stage.shape.map(|shape| Env {
        value: 0.0,
        x: shape.x,
        y: shape.y,
        w: shape.w,
        h: shape.h,
    });
    match behavior {
        Behavior::Set { common, to } => {
            let attr = behavior.attr()?;
            let to = to.as_ref()?;
            let value = if let Some(make) = color_set(attr) {
                let Value::Color(Some(hex)) = to else {
                    return None;
                };
                make(ColorValue::Hex(*hex))
            } else if let Some((make, read)) = switch_set(attr) {
                make(switch_value(to, read)?)
            } else if attr == "style.opacity" {
                SetValue::Opacity(unit(scalar(to, &env.unwrap_or_default(), rng)?)?)
            } else {
                return None;
            };
            let (begin_ms, duration_ms) = span(common, step)?;
            Some(Part::Track(
                Keyframes::Set(SetTrack {
                    begin_ms,
                    duration_ms,
                    to: value,
                }),
                retime_of(common, step),
            ))
        }
        Behavior::Color {
            common,
            from,
            to,
            by,
        } => {
            let make = color_track(behavior.attr()?)?;
            let frames = color_frames(from.as_ref(), to.as_ref(), by.as_ref())?;
            track(common, step, frames).map(|t| Part::Track(make(t), retime_of(common, step)))
        }
        Behavior::Anim {
            common,
            discrete,
            tavs,
            from,
            to,
            by,
        } => {
            let attr = behavior.attr()?;
            if let Some((make, read)) = font_switch(attr) {
                let frames = switch_frames(tavs, read)?;
                return track(common, step, frames)
                    .map(|t| Part::Track(make(t), retime_of(common, step)));
            }
            let discrete = *discrete;
            let sampled = |env: &Env, rng: &mut Rng| {
                sample_anim(
                    tavs,
                    from.as_deref(),
                    to.as_deref(),
                    by.as_deref(),
                    discrete,
                    env,
                    rng,
                )
            };
            let (begin, duration) = span(common, step)?;
            let axis = |frames: Vec<(UnitRatio, f64)>| Axis {
                begin,
                duration,
                easing: easing(common),
                retime: retime_of(common, step),
                frames,
            };
            match attr {
                "ppt_x" | "ppt_y" | "ppt_w" | "ppt_h" => {
                    let env = env?;
                    let shape = stage.shape?;
                    let points = sampled(&env, rng)?;
                    let map = |f: &dyn Fn(f64) -> Option<f64>| {
                        points
                            .iter()
                            .map(|(t, v)| f(*v).map(|v| (*t, v)))
                            .collect::<Option<Vec<_>>>()
                    };
                    Some(match attr {
                        "ppt_x" => Part::MoveX(axis(map(&|v| Some((v - shape.x) * stage.w_px))?)),
                        "ppt_y" => Part::MoveY(axis(map(&|v| Some((v - shape.y) * stage.h_px))?)),
                        "ppt_w" => {
                            Part::SizeX(axis(map(&|v| (shape.w > 0.0).then(|| v / shape.w))?))
                        }
                        _ => Part::SizeY(axis(map(&|v| (shape.h > 0.0).then(|| v / shape.h))?)),
                    })
                }
                "r" | "style.rotation" => {
                    let rotation = stage.shape.map_or(0.0, |shape| shape.rotation);
                    let points = sampled(&env.unwrap_or_default(), rng)?;
                    let frames = points
                        .into_iter()
                        .map(|(offset, v)| {
                            Some(Frame {
                                offset,
                                value: degrees(v - rotation)?,
                            })
                        })
                        .collect::<Option<Vec<_>>>()?;
                    track(common, step, frames)
                        .map(|t| Part::Track(Keyframes::Rotate(t), retime_of(common, step)))
                }
                "style.opacity" => {
                    let points = sampled(&env.unwrap_or_default(), rng)?;
                    let frames = points
                        .into_iter()
                        .map(|(offset, v)| {
                            Some(Frame {
                                offset,
                                value: unit(v)?,
                            })
                        })
                        .collect::<Option<Vec<_>>>()?;
                    track(common, step, frames)
                        .map(|t| Part::Track(Keyframes::Opacity(t), retime_of(common, step)))
                }
                _ => None,
            }
        }
        Behavior::Effect {
            common,
            transition,
            filter: Some(name),
        } => {
            let motion = match transition.as_str() {
                "in" => InOut::In,
                "out" => InOut::Out,
                _ => return None,
            };
            let leaving = motion == InOut::Out;
            match filter(name, exiting || leaving)? {
                Clip::Fade => {
                    let (from, to) = if leaving { (1, 0) } else { (0, 1) };
                    let values = vec![
                        UnitRatio::from_steps(from * 100_000),
                        UnitRatio::from_steps(to * 100_000),
                    ];
                    track(common, step, frames(values))
                        .map(|t| Part::Track(Keyframes::Opacity(t), retime_of(common, step)))
                }
                Clip::Reveal(reveal) => {
                    let (begin_ms, duration_ms) = span(common, step)?;
                    Some(Part::Track(
                        Keyframes::ClipPath(ClipTrack {
                            begin_ms,
                            duration_ms,
                            easing: easing(common),
                            reveal,
                            transition: motion,
                        }),
                        retime_of(common, step),
                    ))
                }
                Clip::Slide(dir) => {
                    let shape = stage.shape?;
                    // The player's slight distance: a quarter of the box.
                    let (dx, dy) = match dir {
                        Dir::Left => (-1.0, 0.0),
                        Dir::Right => (1.0, 0.0),
                        Dir::Up => (0.0, -1.0),
                        _ => (0.0, 1.0),
                    };
                    let far = (
                        dx * 0.25 * shape.w * stage.w_px,
                        dy * 0.25 * shape.h * stage.h_px,
                    );
                    let (start, end) = if leaving {
                        ((0.0, 0.0), far)
                    } else {
                        ((-far.0, -far.1), (0.0, 0.0))
                    };
                    let point = |(x, y): (f64, f64)| -> Option<Offset> {
                        Some(Offset {
                            x: Px::quantise(x).ok()?,
                            y: Px::quantise(y).ok()?,
                        })
                    };
                    let values = vec![point(start)?, point(end)?];
                    track(common, step, frames(values))
                        .map(|t| Part::Track(Keyframes::Translate(t), retime_of(common, step)))
                }
            }
        }
        Behavior::Scale { common, amounts } => {
            let Amounts { by, from, to } = *amounts;
            let start = from.unwrap_or((1.0, 1.0));
            let end = to.or_else(|| by.map(|(x, y)| (start.0 * x, start.1 * y)))?;
            let value = |(x, y): (f64, f64)| -> Option<Scale> {
                Some(Scale {
                    x: scale_value(x)?,
                    y: scale_value(y)?,
                })
            };
            let values = vec![value(start)?, value(end)?];
            track(common, step, frames(values))
                .map(|t| Part::Track(Keyframes::Scale(t), retime_of(common, step)))
        }
        Behavior::Rotate { common, amounts } => {
            let rotation = stage.shape.map_or(0.0, |shape| shape.rotation);
            let Amounts { by, from, to } = *amounts;
            let start = from.map_or(0.0, |from| from - rotation);
            let end = match (to, by) {
                (Some(to), _) => to - rotation,
                (None, Some(by)) => start + by,
                (None, None) => return None,
            };
            let values = vec![degrees(start)?, degrees(end)?];
            track(common, step, frames(values))
                .map(|t| Part::Track(Keyframes::Rotate(t), retime_of(common, step)))
        }
        Behavior::Motion {
            common,
            path: Some(text),
        } => {
            let segments = path(text, stage)?;
            let points = path_points(&segments);
            if points.len() < 2 {
                return None;
            }
            let (begin, duration) = span(common, step)?;
            let count = points.len();
            let xs = Axis {
                begin,
                duration,
                easing: easing(common),
                retime: retime_of(common, step),
                frames: points
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (offset(i, count), p.0))
                    .collect(),
            };
            let ys: Vec<(UnitRatio, f64)> = points
                .iter()
                .enumerate()
                .map(|(i, p)| (offset(i, count), p.1))
                .collect();
            let frames = xs
                .frames
                .iter()
                .zip(ys)
                .map(|((offset, x), (_, y))| {
                    Some(Frame {
                        offset: *offset,
                        value: Offset {
                            x: Px::quantise(*x).ok()?,
                            y: Px::quantise(y).ok()?,
                        },
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            track(common, step, frames)
                .map(|t| Part::Track(Keyframes::Translate(t), retime_of(common, step)))
        }
        _ => None,
    }
}

/// A path's points, curves sampled, at most [`KEYFRAMES`] of them (the path thinned evenly when it has more).
fn path_points(segments: &[PathSeg]) -> Vec<(f64, f64)> {
    let mut points: Vec<(f64, f64)> = Vec::new();
    let mut at = (0.0, 0.0);
    let mut start = (0.0, 0.0);
    for segment in segments {
        match *segment {
            PathSeg::MoveTo(x, y) => {
                at = (x.to_f64(), y.to_f64());
                start = at;
                points.push(at);
            }
            PathSeg::LineTo(x, y) => {
                at = (x.to_f64(), y.to_f64());
                points.push(at);
            }
            PathSeg::CurveTo(x1, y1, x2, y2, x, y) => {
                let (p0, p1, p2, p3) = (
                    at,
                    (x1.to_f64(), y1.to_f64()),
                    (x2.to_f64(), y2.to_f64()),
                    (x.to_f64(), y.to_f64()),
                );
                for i in 1..=8 {
                    let t = f64::from(i) / 8.0;
                    let u = 1.0 - t;
                    let bez = |a: f64, b: f64, c: f64, d: f64| {
                        u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
                    };
                    points.push((bez(p0.0, p1.0, p2.0, p3.0), bez(p0.1, p1.1, p2.1, p3.1)));
                }
                at = p3;
            }
            PathSeg::Close => {
                at = start;
                points.push(at);
            }
        }
    }
    if points.len() > KEYFRAMES {
        let count = points.len();
        points = (0..KEYFRAMES)
            .filter_map(|i| points.get(i * (count - 1) / (KEYFRAMES - 1)).copied())
            .collect();
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAGE: Stage = Stage {
        w_px: 1280.0,
        h_px: 720.0,
        shape: Some(ShapeBox {
            x: 0.5,
            y: 0.5,
            w: 0.25,
            h: 0.25,
            rotation: 0.0,
        }),
    };

    fn px(x: f64) -> Px {
        Px::quantise(x).unwrap()
    }

    /// The tracks alone.
    fn compile(
        behaviors: &[Behavior],
        exiting: bool,
        step: Ms,
        stage: &Stage,
        seed: u64,
    ) -> Vec<Keyframes> {
        super::compile(behaviors, exiting, step, stage, seed).tracks
    }

    #[test]
    fn filters_read_as_powerpoint_plays_them() {
        let wipe = |dir| Some(Clip::Reveal(Reveal::Wipe(WipeParams { dir, soft: true })));
        assert_eq!(filter("wipe(up)", false), wipe(Dir::Down));
        assert_eq!(filter("wipe(down)", false), wipe(Dir::Up));
        assert_eq!(filter("wipe(left)", false), wipe(Dir::Right));
        assert_eq!(filter("wipe(right)", true), wipe(Dir::Left));
        assert_eq!(
            filter("strips(downLeft)", false),
            Some(Clip::Reveal(Reveal::Strips(StripsParams {
                corner: Diagonal::LeftDown
            })))
        );
        assert_eq!(
            filter("barn(inVertical)", false),
            Some(Clip::Reveal(Reveal::Split(SplitParams {
                orient: Orient::Vertical,
                in_out: InOut::In
            })))
        );
        assert_eq!(
            filter("barn(outHorizontal)", false),
            Some(Clip::Reveal(Reveal::Split(SplitParams {
                orient: Orient::Horizontal,
                in_out: InOut::Out
            })))
        );
        assert_eq!(
            filter("checkerboard(across)", false),
            Some(Clip::Reveal(Reveal::Checker(Dir4Params {
                dir: Dir4::Right
            })))
        );
        assert_eq!(
            filter("diamond(out)", false),
            Some(Clip::Reveal(Reveal::Iris(IrisParams {
                shape: IrisShape::Diamond,
                in_out: InOut::Out
            })))
        );
        assert_eq!(
            filter("wheel(8)", false),
            Some(Clip::Reveal(Reveal::Wheel(WheelParams {
                spokes: 8,
                cw: true
            })))
        );
        assert_eq!(filter("fade", false), Some(Clip::Fade));
        assert_eq!(filter("image", false), Some(Clip::Reveal(Reveal::Dissolve)));
        assert_eq!(
            filter("slide(fromTop)", false),
            Some(Clip::Slide(Dir::Down))
        );
        assert_eq!(filter("slide(fromTop)", true), Some(Clip::Slide(Dir::Up)));
        for bad in [
            "wipe(sideways)",
            "wheel(9)",
            "wheel(0)",
            "slide(up)",
            "wipe(fromTop)",
            "barn(in)",
            "shimmer",
            "fade(1)",
            "box(in",
        ] {
            assert_eq!(filter(bad, false), None, "{bad}");
        }
    }

    /// An in/out clip names the entrance it plays, leaving too: `transition="out"` runs it backwards as SMIL's
    /// `mode="out"` does in LibreOffice's engine, so `box(in)` leaving is `iris{in}` reversed, a hole opening from the
    /// centre.
    #[test]
    fn a_leaving_clip_keeps_its_in_out() {
        let iris = |shape, in_out| Some(Clip::Reveal(Reveal::Iris(IrisParams { shape, in_out })));
        for (text, shape, in_out) in [
            ("box(in)", IrisShape::Box, InOut::In),
            ("box(out)", IrisShape::Box, InOut::Out),
            ("circle(in)", IrisShape::Circle, InOut::In),
            ("diamond(out)", IrisShape::Diamond, InOut::Out),
            ("plus(in)", IrisShape::Plus, InOut::In),
        ] {
            assert_eq!(filter(text, true), iris(shape, in_out), "{text}");
            assert_eq!(filter(text, true), filter(text, false), "{text}");
        }
        let split =
            |orient, in_out| Some(Clip::Reveal(Reveal::Split(SplitParams { orient, in_out })));
        assert_eq!(
            filter("barn(inHorizontal)", true),
            split(Orient::Horizontal, InOut::In)
        );
        assert_eq!(
            filter("barn(outVertical)", true),
            split(Orient::Vertical, InOut::Out)
        );
        // Tier 2: the closing track plays the filter's own entrance backwards.
        let exit = Behavior::Effect {
            common: common(500),
            transition: "out".into(),
            filter: Some("box(in)".into()),
        };
        let tracks = compile(&[exit], true, Ms(500), &STAGE, 1);
        assert!(
            matches!(tracks.as_slice(), [Keyframes::ClipPath(c)] if c.transition == InOut::Out
                && c.reveal == Reveal::Iris(IrisParams { shape: IrisShape::Box, in_out: InOut::In })),
            "{tracks:?}"
        );
    }

    #[test]
    fn paths_become_slide_pixels() {
        let path = path("M 0 0 L 0.1 0 C 0.1 0.1 0.2 0.1 0.25 0.25 Z E", &STAGE).unwrap();
        assert_eq!(
            path,
            vec![
                PathSeg::MoveTo(px(0.0), px(0.0)),
                PathSeg::LineTo(px(128.0), px(0.0)),
                PathSeg::CurveTo(
                    px(128.0),
                    px(72.0),
                    px(256.0),
                    px(72.0),
                    px(320.0),
                    px(180.0)
                ),
                PathSeg::Close,
            ]
        );
        let relative = super::path("m 0.1 0 l 0.1 0.1", &STAGE).unwrap();
        assert_eq!(relative[1], PathSeg::LineTo(px(256.0), px(72.0)));
        for bad in [
            "L 0 0",
            "M 0",
            "M 0 0 Q 1 1",
            "M x 0",
            "",
            "M 0 0 L NaN 0",
            "Mx 0 0",
        ] {
            assert_eq!(super::path(bad, &STAGE), None, "{bad:?}");
        }
        let long = format!("M 0 0{}", " L 0 0".repeat(PATH_SEGMENTS));
        assert_eq!(super::path(&long, &STAGE), None);
    }

    fn common(dur: u32) -> Common {
        Common {
            begin: None,
            dur: Some(Time::Ms(Ms(dur))),
            accel: None,
            decel: None,
            target: None,
            attrs: Vec::new(),
            auto_reverse: false,
            filter: None,
            holds: true,
            repeat_count: None,
            repeat_dur: None,
        }
    }

    fn anim(attr: &str, values: &[(&str, Option<&str>)]) -> Behavior {
        let count = values.len();
        Behavior::Anim {
            common: Common {
                attrs: vec![attr.to_owned()],
                ..common(500)
            },
            discrete: false,
            tavs: values
                .iter()
                .enumerate()
                .map(|(i, (value, fmla))| Tav {
                    tm: Some((i * 100_000 / (count - 1)) as u32),
                    value: Some(Value::Str((*value).to_owned())),
                    fmla: fmla.map(str::to_owned),
                })
                .collect(),
            from: None,
            to: None,
            by: None,
        }
    }

    #[test]
    fn positions_pair_into_one_translation() {
        // A fly-in from the left: the centre starts half a box left of the slide.
        let tracks = compile(
            &[
                anim("ppt_x", &[("0-#ppt_w/2", None), ("#ppt_x", None)]),
                anim("ppt_y", &[("#ppt_y", None), ("#ppt_y", None)]),
            ],
            false,
            Ms(500),
            &STAGE,
            1,
        );
        let [Keyframes::Translate(track)] = tracks.as_slice() else {
            panic!("{tracks:?}");
        };
        assert_eq!(track.frames.len(), 2);
        assert_eq!(
            track.frames[0].value,
            Offset {
                x: px(-0.625 * 1280.0),
                y: px(0.0)
            }
        );
        assert_eq!(
            track.frames[1].value,
            Offset {
                x: px(0.0),
                y: px(0.0)
            }
        );
    }

    #[test]
    fn formulas_shape_their_segments() {
        let tracks = compile(
            &[anim(
                "ppt_y",
                &[("0", Some("#ppt_y-sin(pi*$)/3")), ("1", None)],
            )],
            false,
            Ms(500),
            &STAGE,
            1,
        );
        let [Keyframes::Translate(track)] = tracks.as_slice() else {
            panic!("{tracks:?}");
        };
        assert_eq!(track.frames.len(), KEYFRAMES);
        let middle = track
            .frames
            .iter()
            .map(|f| f.value.y.to_f64())
            .fold(0.0, f64::min);
        assert!(middle < -230.0, "{middle}");
        assert_eq!(track.frames[0].value.y, px(0.0));
        // A formula too deep to read compiles to nothing.
        let deep = format!("{}$/2{}", "(".repeat(400), ")".repeat(400));
        assert!(
            compile(
                &[anim("ppt_y", &[("0", Some(&deep)), ("1", None)])],
                false,
                Ms(500),
                &STAGE,
                1
            )
            .is_empty()
        );
    }

    #[test]
    fn scales_rotations_opacities_and_clips() {
        let scale = Behavior::Scale {
            common: common(500),
            amounts: Amounts {
                by: Some((1.5, 1.5)),
                from: None,
                to: None,
            },
        };
        let rotate = Behavior::Rotate {
            common: common(500),
            amounts: Amounts {
                by: Some(360.0),
                from: None,
                to: None,
            },
        };
        let fade = Behavior::Effect {
            common: common(500),
            transition: "in".into(),
            filter: Some("fade".into()),
        };
        let wipe = Behavior::Effect {
            common: common(500),
            transition: "out".into(),
            filter: Some("wipe(down)".into()),
        };
        let color = Behavior::Color {
            common: common(500),
            from: None,
            to: Some(Value::Color(None)),
            by: None,
        };
        let tracks = compile(
            &[scale, rotate, fade, wipe, color],
            false,
            Ms(500),
            &STAGE,
            1,
        );
        assert_eq!(tracks.len(), 4);
        assert!(
            matches!(&tracks[0], Keyframes::Scale(t) if t.frames[1].value.x == Ratio::quantise(1.5).unwrap())
        );
        assert!(
            matches!(&tracks[1], Keyframes::Rotate(t) if t.frames[1].value == Deg::quantise(360.0).unwrap())
        );
        assert!(
            matches!(&tracks[2], Keyframes::Opacity(t) if t.frames[0].value == UnitRatio::ZERO)
        );
        assert!(
            matches!(&tracks[3], Keyframes::ClipPath(c) if c.transition == InOut::Out && c.reveal == Reveal::Wipe(WipeParams { dir: Dir::Up, soft: true }))
        );
        // Without a box, positions cannot compile; a scheme colour never does.
        let no_box = Stage {
            shape: None,
            ..STAGE
        };
        assert!(
            compile(
                &[anim("ppt_x", &[("0", None), ("#ppt_x", None)])],
                false,
                Ms(500),
                &no_box,
                1
            )
            .is_empty()
        );
        let color = Behavior::Color {
            common: common(500),
            from: None,
            to: Some(Value::Color(None)),
            by: None,
        };
        assert!(compile(&[color], false, Ms(500), &STAGE, 1).is_empty());
    }
}
