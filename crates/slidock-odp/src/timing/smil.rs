//! Tier 2: an effect the vocabulary does not name plays as keyframe tracks compiled from its SMIL
//! behaviours, in slide pixels. `x`/`y` become a translation, `width`/`height` a scale, `rotate` and
//! `animateTransform` a rotation or a scale, `opacity` an opacity, `animateMotion` a translation along its path, and
//! a `transitionFilter` the clip its filter plays (the one SMIL filter mapper of the slide transitions,
//! [`slidock_presets::odp::transition`]). `animateColor` and `set` of `color`, `fill-color` and `stroke-color` become
//! text, fill and line colour tracks (a change `by` an HSL amount sampled as shifts of the element's own colour), and
//! `font-weight`, `font-style` and `text-underline` bold, italic and underline switches; a `set` is a switch at its
//! begin ([`SetTrack`]), which a `set` that does not hold switches back at its end. A behaviour's own
//! `smil:accelerate`/`smil:decelerate` is its track's easing, and its own time manipulations fold into its frames
//! ([`Retime`]): `smil:autoReverse` runs them forward then back over twice its span, `smil:keySplines`, which
//! LibreOffice writes for a time filter (pairs of time and progress), reshapes their timing, and `smil:repeatCount`
//! or `smil:repeatDur` plays them again. What tracks cannot animate (an unknown attribute or value, a time filter,
//! easing or repeat that does not read or fold) is left out and named; an effect left with no track does not
//! compile.

use slidock_ir::effects::limits::{HUE_SHIFT_DEGREES, KEYFRAMES, PATH_SEGMENTS, TRACKS};
use slidock_ir::effects::{
    BuildEffect, ClipTrack, ColorValue, Dir, Distance, Easing, FlyInParams, Frame, HexColor,
    HslShift, InOut, Keyframes, Offset, PathSeg, RepeatWritten, Retime, Reveal, Scale, SetTrack,
    SetValue, Through, TimeFilter, Track, TransitionEffect, UnfoldedRepeat,
};
use slidock_ir::units::{Deg, Ms, Px, Ratio, UnitRatio};

use super::expr::{Expr, Vars};
use super::value;
use super::{Geometry, Leaf, LeafKind};
use crate::names::qn;
use crate::transition::table_subtype;

/// The share of its box a slide clip moves the element by (`flyIn{distance: slight}`).
const SLIGHT: f64 = 0.25;

/// Frames a formula is sampled at.
const FORMULA_FRAMES: usize = 9;

/// The build effect a `transitionFilter` plays on an element: a clip, a fade, or a slide clip (a short fly-in
/// from the filter's side, toward it when the filter closes, `smil:mode="out"`). A closing iris or split plays the
/// entrance its filter reads as backwards, as LibreOffice's engine does (`ClippingFunctor` inverts the sweep of the
/// filters whose `transitionfactorytab` row sets `outInvertsSweep`): `ooo-exit-box in` (`irisWipe` reversed, mode out)
/// is `iris{in}` reversed, a hole opening from the centre. `None` for a filter with no entry, or a
/// sub-type its type does not know.
pub(super) fn filter_effect(leaf: &Leaf<'_>) -> Option<BuildEffect> {
    let element = leaf.element;
    let lookup = slidock_presets::odp::transition(
        element.attr(qn!("smil:type"))?,
        table_subtype(element.attr(qn!("smil:subtype"))),
        element.attr(qn!("smil:direction")) == Some("reverse"),
        element.attr(qn!("smil:fadeColor")),
    )?;
    if !lookup.unknown.is_empty() {
        return None;
    }
    let closes = element.attr(qn!("smil:mode")) == Some("out");
    Some(match lookup.row.effect? {
        TransitionEffect::Wipe(p) => BuildEffect::Wipe(p),
        TransitionEffect::Split(p) => BuildEffect::Split(p),
        TransitionEffect::Iris(p) => BuildEffect::Iris(p),
        TransitionEffect::Wheel(p) => BuildEffect::Wheel(p),
        TransitionEffect::Wedge => BuildEffect::Wedge,
        TransitionEffect::Clock(p) => BuildEffect::Clock(p),
        TransitionEffect::Blinds(p) => BuildEffect::Blinds(p),
        TransitionEffect::Checker(p) => BuildEffect::Checker(p),
        TransitionEffect::RandomBars(p) => BuildEffect::RandomBars(p),
        TransitionEffect::Strips(p) => BuildEffect::Strips(p),
        TransitionEffect::Dissolve => BuildEffect::Dissolve,
        TransitionEffect::Fade(fade) if fade.through == Through::None => BuildEffect::Fade,
        TransitionEffect::Cover(p) | TransitionEffect::Uncover(p) => {
            BuildEffect::FlyIn(FlyInParams {
                dir: if closes { p.dir.opposite() } else { p.dir },
                distance: Distance::Slight,
            })
        }
        _ => return None,
    })
}

/// The reveal a clip effect draws.
fn reveal(effect: &BuildEffect) -> Option<Reveal> {
    Some(match effect {
        BuildEffect::Wipe(p) => Reveal::Wipe(*p),
        BuildEffect::Split(p) => Reveal::Split(*p),
        BuildEffect::Iris(p) => Reveal::Iris(*p),
        BuildEffect::Wheel(p) => Reveal::Wheel(*p),
        BuildEffect::Wedge => Reveal::Wedge,
        BuildEffect::Clock(p) => Reveal::Clock(*p),
        BuildEffect::Blinds(p) => Reveal::Blinds(*p),
        BuildEffect::Checker(p) => Reveal::Checker(*p),
        BuildEffect::RandomBars(p) => Reveal::RandomBars(*p),
        BuildEffect::Strips(p) => Reveal::Strips(*p),
        BuildEffect::Dissolve => Reveal::Dissolve,
        _ => return None,
    })
}

/// A motion path (`svg:path`, in fractions of the slide from the element's position) in slide pixels: `M`, `L`,
/// `H`, `V`, `C` and `Z`, absolute or relative. `None` for a path that does not parse, starts without a move, or
/// passes [`PATH_SEGMENTS`].
pub(super) fn motion_path(text: &str, geometry: &Geometry) -> Option<Vec<PathSeg>> {
    let mut tokens = Tokens { text, at: 0 };
    let mut out = Vec::new();
    let (mut x, mut y) = (0.0, 0.0);
    let (mut start_x, mut start_y) = (0.0, 0.0);
    let px = |fx: f64, fy: f64| -> Option<(Px, Px)> {
        Some((
            Px::quantise(fx * geometry.slide_w).ok()?,
            Px::quantise(fy * geometry.slide_h).ok()?,
        ))
    };
    let mut command = None;
    loop {
        if out.len() > PATH_SEGMENTS {
            return None;
        }
        match tokens.next_command() {
            Some(c) => command = Some(c),
            None if tokens.done() => break,
            None => {}
        }
        let c = command?;
        let relative = c.is_ascii_lowercase();
        let (dx, dy) = if relative { (x, y) } else { (0.0, 0.0) };
        match c.to_ascii_uppercase() {
            'M' => {
                (x, y) = (dx + tokens.number()?, dy + tokens.number()?);
                (start_x, start_y) = (x, y);
                let (px_x, px_y) = px(x, y)?;
                out.push(PathSeg::MoveTo(px_x, px_y));
                command = Some(if relative { 'l' } else { 'L' });
            }
            'L' => {
                (x, y) = (dx + tokens.number()?, dy + tokens.number()?);
                let (px_x, px_y) = px(x, y)?;
                out.push(PathSeg::LineTo(px_x, px_y));
            }
            'H' => {
                x = dx + tokens.number()?;
                let (px_x, px_y) = px(x, y)?;
                out.push(PathSeg::LineTo(px_x, px_y));
            }
            'V' => {
                y = (if relative { y } else { 0.0 }) + tokens.number()?;
                let (px_x, px_y) = px(x, y)?;
                out.push(PathSeg::LineTo(px_x, px_y));
            }
            'C' => {
                let (x1, y1) = (dx + tokens.number()?, dy + tokens.number()?);
                let (x2, y2) = (dx + tokens.number()?, dy + tokens.number()?);
                (x, y) = (dx + tokens.number()?, dy + tokens.number()?);
                let (a, b) = px(x1, y1)?;
                let (c2, d) = px(x2, y2)?;
                let (e, f) = px(x, y)?;
                out.push(PathSeg::CurveTo(a, b, c2, d, e, f));
            }
            'Z' => {
                out.push(PathSeg::Close);
                (x, y) = (start_x, start_y);
                command = None;
                if tokens.done() {
                    break;
                }
                continue;
            }
            _ => return None,
        }
        if tokens.done() {
            break;
        }
    }
    (!out.is_empty()
        && out.len() <= PATH_SEGMENTS
        && matches!(out.first(), Some(PathSeg::MoveTo(..))))
    .then_some(out)
}

struct Tokens<'a> {
    text: &'a str,
    at: usize,
}

impl Tokens<'_> {
    fn skip(&mut self) {
        while let Some(b) = self.text.as_bytes().get(self.at) {
            if b.is_ascii_whitespace() || *b == b',' {
                self.at += 1;
            } else {
                break;
            }
        }
    }

    fn done(&mut self) -> bool {
        self.skip();
        self.at >= self.text.len()
    }

    fn next_command(&mut self) -> Option<char> {
        self.skip();
        let b = *self.text.as_bytes().get(self.at)?;
        if b.is_ascii_alphabetic() {
            self.at += 1;
            Some(char::from(b))
        } else {
            None
        }
    }

    fn number(&mut self) -> Option<f64> {
        self.skip();
        let bytes = self.text.as_bytes();
        let start = self.at;
        if matches!(bytes.get(self.at), Some(b'-' | b'+')) {
            self.at += 1;
        }
        while bytes
            .get(self.at)
            .is_some_and(|b| b.is_ascii_digit() || *b == b'.')
        {
            self.at += 1;
        }
        value::number(self.text.get(start..self.at)?)
    }
}

/// The keyframes a value list (`smil:values` with `smil:keyTimes`, or `smil:from`/`smil:to`/`smil:by`) reads,
/// each value as `read` gives it; `None` when they do not read.
fn frames<V: Copy>(
    leaf: &Leaf<'_>,
    base: f64,
    vars: &Vars,
    read: impl Fn(f64) -> Option<V>,
) -> Option<Vec<Frame<V>>> {
    let element = leaf.element;
    let eval = |text: &str| Expr::parse(text).ok()?.eval(vars).ok();
    let values: Vec<f64> = match element.attr(qn!("smil:values")) {
        Some(list) => list.split(';').map(eval).collect::<Option<_>>()?,
        None => {
            let from = element.attr(qn!("smil:from")).map(eval);
            let to = element.attr(qn!("smil:to")).map(eval);
            let by = element.attr(qn!("smil:by")).map(eval);
            let from = from.unwrap_or(Some(base))?;
            let to = match (to, by) {
                (Some(to), _) => to?,
                (None, Some(by)) => from + by?,
                (None, None) => return None,
            };
            vec![from, to]
        }
    };
    if values.len() < 2 || values.len() > KEYFRAMES {
        return None;
    }
    let times: Vec<f64> = match element.attr(qn!("smil:keyTimes")) {
        Some(list) => list
            .split(';')
            .map(value::fraction)
            .collect::<Option<_>>()?,
        None => {
            let last = (values.len() - 1) as f64;
            (0..values.len()).map(|i| i as f64 / last).collect()
        }
    };
    let ordered = times.windows(2).all(|pair| pair.first() <= pair.last());
    if times.len() != values.len()
        || !ordered
        || times.first() != Some(&0.0)
        || times.last() != Some(&1.0)
    {
        return None;
    }
    let formula = element
        .attr(qn!("anim:formula"))
        .map(Expr::parse)
        .transpose()
        .ok()?;
    let sampled: Vec<(f64, f64)> = match formula {
        None => times.into_iter().zip(values).collect(),
        Some(formula) => (0..FORMULA_FRAMES)
            .map(|i| {
                let t = i as f64 / (FORMULA_FRAMES - 1) as f64;
                let value = interpolate(&times, &values, t);
                let out = formula
                    .eval(&Vars {
                        dollar: value,
                        ..*vars
                    })
                    .ok()?;
                Some((t, out))
            })
            .collect::<Option<_>>()?,
    };
    sampled
        .into_iter()
        .map(|(t, v)| {
            Some(Frame {
                offset: UnitRatio::quantise(t).ok()?,
                value: read(v)?,
            })
        })
        .collect()
}

/// The value of a key-timed list at `t`, linear between the keys.
fn interpolate(times: &[f64], values: &[f64], t: f64) -> f64 {
    let mut out = values.first().copied().unwrap_or(0.0);
    for (pair_t, pair_v) in times.windows(2).zip(values.windows(2)) {
        if let ([t0, t1], [v0, v1]) = (pair_t, pair_v)
            && t >= *t0
            && t <= *t1
        {
            out = if t1 > t0 {
                v0 + (v1 - v0) * (t - t0) / (t1 - t0)
            } else {
                *v1
            };
        }
    }
    out
}

/// A leaf's span: its begin and duration, `None` past an hour or without a valid duration.
fn span(leaf: &Leaf<'_>) -> Option<(Ms, Ms)> {
    let begin = leaf.begin.value()?;
    let dur = match leaf.dur {
        Some(dur) => dur.value()?,
        None => Ms(0),
    };
    (u64::from(begin.0) + u64::from(dur.0) <= u64::from(Ms::MAX)).then_some((begin, dur))
}

fn track<V>(span: (Ms, Ms), frames: Vec<Frame<V>>) -> Track<V> {
    Track {
        begin_ms: span.0,
        duration_ms: span.1,
        easing: None,
        frames,
    }
}

fn two<V: Copy>(from: V, to: V) -> Option<Vec<Frame<V>>> {
    Some(vec![
        Frame {
            offset: UnitRatio::ZERO,
            value: from,
        },
        Frame {
            offset: UnitRatio::quantise(1.0).ok()?,
            value: to,
        },
    ])
}

fn pair(text: &str) -> Option<(f64, f64)> {
    let mut parts = text.split([',', ' ']).filter(|part| !part.is_empty());
    let x = value::number(parts.next()?)?;
    let y = parts.next().map_or(Some(x), value::number)?;
    parts.next().is_none().then_some((x, y))
}

fn scale(x: f64, y: f64) -> Option<Scale> {
    let ok = |v: f64| (0.0..=slidock_ir::effects::limits::SCALE).contains(&v);
    (ok(x) && ok(y)).then_some(Scale {
        x: Ratio::quantise(x).ok()?,
        y: Ratio::quantise(y).ok()?,
    })
}

fn degrees(deg: f64) -> Option<Deg> {
    (deg.abs() <= slidock_ir::effects::limits::SPIN_DEGREES)
        .then(|| Deg::quantise(deg).ok())
        .flatten()
}

fn px(x: f64) -> Option<Px> {
    Px::quantise(x).ok()
}

/// One axis of a translation or a scale, waiting for its other axis.
struct Axis {
    span: (Ms, Ms),
    easing: Option<Easing>,
    retime: Retime,
    frames: Vec<Frame<f64>>,
    horizontal: bool,
}

/// Joins the axes that share a span and key times into one track, each other axis at `rest`.
fn join<V>(
    axes: Vec<Axis>,
    rest: f64,
    make: impl Fn(f64, f64) -> Option<V>,
) -> Option<Vec<(Track<V>, Retime)>> {
    let mut out = Vec::new();
    let mut used = vec![false; axes.len()];
    for (i, axis) in axes.iter().enumerate() {
        if used.get(i) == Some(&true) {
            continue;
        }
        let partner = axes.iter().enumerate().position(|(j, other)| {
            j > i
                && used.get(j) == Some(&false)
                && other.horizontal != axis.horizontal
                && other.span == axis.span
                && other.easing == axis.easing
                && other.retime == axis.retime
                && other.frames.len() == axis.frames.len()
                && other
                    .frames
                    .iter()
                    .zip(&axis.frames)
                    .all(|(a, b)| a.offset == b.offset)
        });
        let other = partner.and_then(|j| {
            if let Some(slot) = used.get_mut(j) {
                *slot = true;
            }
            axes.get(j)
        });
        let frames = axis
            .frames
            .iter()
            .enumerate()
            .map(|(k, frame)| {
                let theirs = other
                    .and_then(|o| o.frames.get(k))
                    .map_or(rest, |f| f.value);
                let (x, y) = if axis.horizontal {
                    (frame.value, theirs)
                } else {
                    (theirs, frame.value)
                };
                Some(Frame {
                    offset: frame.offset,
                    value: make(x, y)?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let mut joined = track(axis.span, frames);
        joined.easing = axis.easing;
        out.push((joined, axis.retime.clone()));
    }
    Some(out)
}

/// Frames a colour change that turns the hue is sampled at.
const HUE_FRAMES: usize = 9;

/// The colour track an attribute names: `color` the text, `fill-color` the fill, `stroke-color` the line.
fn color_track(attribute: &str) -> Option<fn(Track<ColorValue>) -> Keyframes> {
    Some(match attribute {
        "color" | "char-color" => Keyframes::Text,
        "fill-color" => Keyframes::Fill,
        "stroke-color" | "line-color" => Keyframes::Line,
        _ => return None,
    })
}

/// A font switch's track, and how its values read.
type Switch = (fn(Track<bool>) -> Keyframes, fn(&str) -> Option<bool>);

/// The font switch an attribute names, and how its values read: `None` for a value it does not know.
fn font_switch(attribute: &str) -> Option<Switch> {
    Some(match attribute {
        "font-weight" => (Keyframes::Bold, |v| match v.trim() {
            "bold" => Some(true),
            "normal" => Some(false),
            _ => None,
        }),
        "font-style" => (Keyframes::Italic, |v| match v.trim() {
            "italic" | "oblique" => Some(true),
            "normal" => Some(false),
            _ => None,
        }),
        "text-underline" => (Keyframes::Underline, |v| match v.trim() {
            "none" => Some(false),
            "solid" | "single" | "true" => Some(true),
            _ => None,
        }),
        _ => return None,
    })
}

/// The switch a colour `set` gives: `color` the text, `fill-color` the fill, `stroke-color` the line.
fn color_set(attribute: &str) -> Option<fn(ColorValue) -> SetValue> {
    Some(match attribute {
        "color" | "char-color" => SetValue::Text,
        "fill-color" => SetValue::Fill,
        "stroke-color" | "line-color" => SetValue::Line,
        _ => return None,
    })
}

/// A font switch's value, and how its source value reads.
type SwitchSet = (fn(bool) -> SetValue, fn(&str) -> Option<bool>);

/// The switch a font `set` gives, and how its value reads.
fn switch_set(attribute: &str) -> Option<SwitchSet> {
    let (_, read) = font_switch(attribute)?;
    let make: fn(bool) -> SetValue = match attribute {
        "font-weight" => SetValue::Bold,
        "font-style" => SetValue::Italic,
        _ => SetValue::Underline,
    };
    Some((make, read))
}

/// The switches a `set` plays: its value from its begin and, when it ends without holding, the element's own value
/// back at its end (a colour's own, full opacity), with whether that end is lost (a font switch: no value names the
/// element's own, so it holds). `None` for an attribute or value no switch plays.
fn set_tracks(leaf: &Leaf<'_>, attribute: &str, retime: &Retime) -> Option<(Vec<Keyframes>, bool)> {
    let to = leaf.element.attr(qn!("smil:to")).unwrap_or_default();
    let value = if let Some(make) = color_set(attribute) {
        make(ColorValue::Hex(HexColor::parse(to.trim()).ok()?))
    } else if let Some((make, read)) = switch_set(attribute) {
        make(read(to)?)
    } else if attribute == "opacity" {
        let v = value::number(to).filter(|v| (0.0..=1.0).contains(v))?;
        SetValue::Opacity(UnitRatio::quantise(v).ok()?)
    } else {
        return None;
    };
    let (begin_ms, duration_ms) = set_span(leaf)?;
    let switch = Keyframes::Set(SetTrack {
        begin_ms,
        duration_ms,
        to: value,
    })
    .folded(retime, |_| true)?
    .0;
    let (begin, length) = switch.span();
    let mut out = vec![switch];
    // An indefinite switch never ends, so it holds whatever its fill.
    let indefinite = leaf.element.attr(qn!("smil:dur")).map(str::trim) == Some("indefinite");
    if holds(leaf) || indefinite {
        return Some((out, false));
    }
    match (value.own(), begin.0.checked_add(length.0)) {
        (Some(own), Some(end)) => {
            out.push(Keyframes::Set(SetTrack {
                begin_ms: Ms(end),
                duration_ms: Ms(0),
                to: own,
            }));
            Some((out, false))
        }
        _ => Some((out, true)),
    }
}

/// A leaf's own time: its manipulations, its curve, and what of them it writes that does not read or fold (it
/// plays without that), each named as "the … of".
pub(super) struct OwnTime {
    pub retime: Retime,
    pub easing: Option<Easing>,
    pub unread: Vec<&'static str>,
}

/// A leaf's own time ([`OwnTime`]): `smil:autoReverse`, `smil:keySplines` (a time filter), `smil:repeatCount` and
/// `smil:repeatDur` (the smaller where both are written), `smil:accelerate` and `smil:decelerate`.
pub(super) fn own_time(leaf: &Leaf<'_>) -> OwnTime {
    let element = leaf.element;
    let mut unread = Vec::new();
    let written = element.attr(qn!("smil:keySplines"));
    let filter = written.and_then(TimeFilter::parse);
    if written.is_some() && filter.is_none() {
        unread.push("time filter");
    }
    let auto_reverse = element.bool_attr(qn!("smil:autoReverse")) == Some(true);
    let count = element
        .attr(qn!("smil:repeatCount"))
        .map(|text| match value::repeat_count(text) {
            value::RepeatCount::Indefinite => Some(RepeatWritten::Indefinite),
            value::RepeatCount::Times(read) => read.value().map(RepeatWritten::Value),
        });
    let total = element.attr(qn!("smil:repeatDur")).map(|text| {
        if text.trim() == "indefinite" {
            Some(RepeatWritten::Indefinite)
        } else {
            value::clock(text)
                .value()
                .map(|ms| RepeatWritten::Value(f64::from(ms.0)))
        }
    });
    let duration = leaf.dur.and_then(|dur| dur.value()).unwrap_or(Ms(0));
    let repeat = match (count, total) {
        (Some(None), _) | (_, Some(None)) => Err(UnfoldedRepeat),
        (count, total) => {
            Retime::repeat_of(count.flatten(), total.flatten(), duration, auto_reverse)
        }
    };
    let repeat = repeat.unwrap_or_else(|UnfoldedRepeat| {
        unread.push("repeat");
        None
    });
    let mut fraction = |name| {
        let text = element.attr(name)?;
        let read = value::fraction(text);
        if read.is_none() {
            unread.push("easing");
        }
        read
    };
    let accel = fraction(qn!("smil:accelerate")).unwrap_or(0.0);
    let decel = fraction(qn!("smil:decelerate")).unwrap_or(0.0);
    let easing = if accel == 0.0 && decel == 0.0 {
        None
    } else {
        let curve = UnitRatio::quantise(accel)
            .ok()
            .zip(UnitRatio::quantise(decel).ok())
            .filter(|(a, d)| a.steps() + d.steps() <= 100_000)
            .map(|(accel, decel)| Easing::AccelDecel { accel, decel });
        if curve.is_none() && !unread.contains(&"easing") {
            unread.push("easing");
        }
        curve
    };
    OwnTime {
        retime: Retime {
            auto_reverse,
            filter,
            repeat,
        },
        easing,
        unread,
    }
}

/// Whether an attribute changes the text's font family or size, which moves the layout: no track plays it.
pub(super) fn changes_layout(attribute: &str) -> bool {
    matches!(
        attribute,
        "font-family" | "font-size" | "char-font-name" | "char-height"
    )
}

/// A `set` that turns on a fill or line a colour track paints: nothing of its own to play.
fn turns_paint_on(leaf: &Leaf<'_>, attribute: &str) -> bool {
    matches!(attribute, "fill" | "stroke") && leaf.element.attr(qn!("smil:to")) == Some("solid")
}

/// A `set`'s span: its begin, and its duration, 0 when it writes none or an indefinite one (the value holds).
fn set_span(leaf: &Leaf<'_>) -> Option<(Ms, Ms)> {
    let begin = leaf.begin.value()?;
    let dur = leaf.dur.and_then(|dur| dur.value()).unwrap_or(Ms(0));
    (u64::from(begin.0) + u64::from(dur.0) <= u64::from(Ms::MAX)).then_some((begin, dur))
}

/// An HSL amount, `hsl(h,s%,l%)` with signed parts: `None` when it does not read or passes the shift bounds.
fn hsl_amount(text: &str) -> Option<(f64, f64, f64)> {
    let inner = text.trim().strip_prefix("hsl(")?.strip_suffix(')')?;
    let mut parts = inner.split(',').map(str::trim);
    let h = value::number(parts.next()?)?;
    let percent = |part: Option<&str>| value::number(part?.strip_suffix('%')?).map(|x| x / 100.0);
    let s = percent(parts.next())?;
    let l = percent(parts.next())?;
    let bounded = h.abs() <= HUE_SHIFT_DEGREES && s.abs() <= 1.0 && l.abs() <= 1.0;
    (parts.next().is_none() && bounded).then_some((h, s, l))
}

/// An `animateColor`'s frames: from `smil:from` (the element's own colour when absent) to `smil:to`, or by an HSL
/// amount, sampled where it turns the hue. A colour from `smil:from` shifts here; the element's own colour shifts
/// when the deck is built.
fn color_frames(leaf: &Leaf<'_>) -> Option<Vec<Frame<ColorValue>>> {
    let element = leaf.element;
    let start = match element.attr(qn!("smil:from")) {
        None => None,
        Some(text) => Some(HexColor::parse(text.trim()).ok()?),
    };
    if let Some(to) = element.attr(qn!("smil:to")) {
        let to = HexColor::parse(to.trim()).ok()?;
        return two(
            start.map_or(ColorValue::Shift(HslShift::NONE), ColorValue::Hex),
            ColorValue::Hex(to),
        );
    }
    let (h, s, l) = hsl_amount(element.attr(qn!("smil:by"))?)?;
    let count = if h == 0.0 { 2 } else { HUE_FRAMES };
    (0..count)
        .map(|i| {
            let t = i as f64 / (count - 1) as f64;
            let shift = HslShift {
                h: Deg::quantise(h * t).ok()?,
                s: Ratio::quantise(s * t).ok()?,
                l: Ratio::quantise(l * t).ok()?,
            };
            Some(Frame {
                offset: UnitRatio::quantise(t).ok()?,
                value: match start {
                    Some(hex) => ColorValue::Hex(shift.apply(hex)),
                    None => ColorValue::Shift(shift),
                },
            })
        })
        .collect()
}

/// Whether a behaviour keeps its end value: `smil:fill` `hold`, `freeze` or `transition`, or no duration of its own
/// (SMIL's `auto`). One that does not returns to the element's own value when it ends.
fn holds(leaf: &Leaf<'_>) -> bool {
    matches!(
        leaf.element.attr(qn!("smil:fill")),
        Some("hold" | "freeze" | "transition")
    ) || leaf.dur.is_none()
}

/// The key times of a value list: `smil:keyTimes`, else evenly spread; `None` when they do not read, rise or start
/// at 0, or do not match the values. A discrete list may end before 1 (its last value holds to the end).
fn key_times(element: &crate::odf::Element, count: usize, discrete: bool) -> Option<Vec<f64>> {
    let times: Vec<f64> = match element.attr(qn!("smil:keyTimes")) {
        Some(list) => list
            .split(';')
            .map(value::fraction)
            .collect::<Option<_>>()?,
        None if discrete => (0..count).map(|i| i as f64 / count as f64).collect(),
        None => {
            let last = count.checked_sub(1)?.max(1) as f64;
            (0..count).map(|i| i as f64 / last).collect()
        }
    };
    let ordered = times.windows(2).all(|pair| pair.first() <= pair.last());
    (times.len() == count
        && ordered
        && times.first() == Some(&0.0)
        && (discrete || times.last() == Some(&1.0)))
    .then_some(times)
}

/// Frames from values at key times: linear ones as they are, discrete ones as steps (each value held from its key
/// time to the next, the last to the end), at most [`KEYFRAMES`] of them.
fn keyed<V: Copy>(times: &[f64], values: &[V], discrete: bool) -> Option<Vec<Frame<V>>> {
    let mut points: Vec<(f64, V)> = Vec::with_capacity(values.len() * 2);
    for (i, (t, value)) in times.iter().zip(values).enumerate() {
        if discrete && i > 0 {
            let previous = *values.get(i - 1)?;
            points.push((*t, previous));
        }
        points.push((*t, *value));
    }
    let last = *points.last()?;
    if last.0 < 1.0 {
        points.push((1.0, last.1));
    }
    if points.len() < 2 || points.len() > KEYFRAMES {
        return None;
    }
    points
        .into_iter()
        .map(|(t, value)| {
            Some(Frame {
                offset: UnitRatio::quantise(t).ok()?,
                value,
            })
        })
        .collect()
}

/// An `animate` of a colour attribute: its `smil:values`, `#rrggbb` each, linear or discrete.
fn color_values_frames(leaf: &Leaf<'_>) -> Option<Vec<Frame<ColorValue>>> {
    let element = leaf.element;
    let values: Vec<ColorValue> = element
        .attr(qn!("smil:values"))?
        .split(';')
        .map(|text| HexColor::parse(text.trim()).ok().map(ColorValue::Hex))
        .collect::<Option<_>>()?;
    let discrete = element.attr(qn!("smil:calcMode")) == Some("discrete");
    let times = key_times(element, values.len(), discrete)?;
    keyed(&times, &values, discrete)
}

/// A colour track's frames, with a return to the element's own colour at the end when the behaviour does not
/// hold.
fn ending(leaf: &Leaf<'_>, mut frames: Vec<Frame<ColorValue>>) -> Option<Vec<Frame<ColorValue>>> {
    if !holds(leaf) {
        if frames.len() >= KEYFRAMES {
            return None;
        }
        frames.push(Frame {
            offset: UnitRatio::quantise(1.0).ok()?,
            value: ColorValue::Shift(HslShift::NONE),
        });
    }
    Some(frames)
}

/// An `animate` of a font switch: its `smil:values`, each held from its key time to the next.
fn switch_frames(leaf: &Leaf<'_>, read: fn(&str) -> Option<bool>) -> Option<Vec<Frame<bool>>> {
    let element = leaf.element;
    let values: Vec<bool> = element
        .attr(qn!("smil:values"))?
        .split(';')
        .map(read)
        .collect::<Option<_>>()?;
    let times = key_times(element, values.len(), true)?;
    keyed(&times, &values, true)
}

/// What tier 2 compiled: the tracks, and what they leave out.
pub(super) struct Compiled {
    pub tracks: Vec<Keyframes>,
    pub lost: Vec<String>,
}

/// Compiles an effect's behaviours into keyframe tracks.
pub(super) fn compile(leaves: &[Leaf<'_>], geometry: &Geometry) -> Compiled {
    let vars = Vars {
        x: geometry.x,
        y: geometry.y,
        width: geometry.width,
        height: geometry.height,
        dollar: 0.0,
    };
    let mut tracks = Vec::new();
    let mut moves = Vec::new();
    let mut sizes = Vec::new();
    let mut lost: Vec<String> = Vec::new();
    let past_limit = format!("its tracks past the first {TRACKS}");
    for leaf in leaves {
        // Two axes join into at most one track: past twice the limit, the rest could only be cut, and the join
        // stays bounded.
        if tracks.len() + moves.len() + sizes.len() >= 2 * TRACKS {
            lost.push(past_limit.clone());
            break;
        }
        let attribute = leaf.attribute();
        let name = || {
            let what = attribute.unwrap_or(leaf.element.local.as_str());
            format!("its {what} animation")
        };
        if leaf.kind == LeafKind::Set
            && attribute.is_some_and(|attr| attr == "visibility" || turns_paint_on(leaf, attr))
        {
            continue;
        }
        let OwnTime {
            retime,
            easing,
            unread,
        } = own_time(leaf);
        for what in unread {
            lost.push(format!("the {what} of {}", name()));
        }
        let before = tracks.len();
        let (moves_before, sizes_before) = (moves.len(), sizes.len());
        if leaf.kind == LeafKind::Set
            && let Some(attr) = attribute
            && let Some((switches, end_lost)) = set_tracks(leaf, attr, &retime)
        {
            tracks.extend(switches);
            if end_lost {
                lost.push(format!("the end of its {attr} switch"));
            }
            continue;
        }
        let Some(span) = span(leaf) else {
            lost.push(name());
            continue;
        };
        let compiled: Option<()> = match (leaf.kind, attribute) {
            (LeafKind::TransitionFilter, _) => {
                let closes = leaf.element.attr(qn!("smil:mode")) == Some("out");
                match filter_effect(leaf) {
                    Some(BuildEffect::Fade) => {
                        let (from, to) = if closes { (1.0, 0.0) } else { (0.0, 1.0) };
                        UnitRatio::quantise(from)
                            .ok()
                            .zip(UnitRatio::quantise(to).ok())
                            .and_then(|(from, to)| two(from, to))
                            .map(|frames| tracks.push(Keyframes::Opacity(track(span, frames))))
                    }
                    Some(BuildEffect::FlyIn(fly)) => slide_clip(fly.dir, closes, geometry)
                        .map(|frames| tracks.push(Keyframes::Translate(track(span, frames)))),
                    Some(effect) => reveal(&effect).map(|reveal| {
                        tracks.push(Keyframes::ClipPath(ClipTrack {
                            begin_ms: span.0,
                            duration_ms: span.1,
                            easing: None,
                            reveal,
                            transition: if closes { InOut::Out } else { InOut::In },
                        }));
                    }),
                    None => None,
                }
            }
            (LeafKind::Animate, Some(axis @ ("x" | "y"))) => {
                let horizontal = axis == "x";
                let (base, scale_px) = if horizontal {
                    (geometry.x, geometry.slide_w)
                } else {
                    (geometry.y, geometry.slide_h)
                };
                frames(leaf, base, &vars, |v| Some((v - base) * scale_px)).map(|frames| {
                    moves.push(Axis {
                        span,
                        easing: None,
                        retime: Retime::default(),
                        frames,
                        horizontal,
                    });
                })
            }
            (LeafKind::Animate, Some(axis @ ("width" | "height"))) => {
                let horizontal = axis == "width";
                let base = if horizontal {
                    geometry.width
                } else {
                    geometry.height
                };
                (base > 0.0)
                    .then(|| frames(leaf, base, &vars, |v| Some(v / base)))
                    .flatten()
                    .map(|frames| {
                        sizes.push(Axis {
                            span,
                            easing: None,
                            retime: Retime::default(),
                            frames,
                            horizontal,
                        });
                    })
            }
            (LeafKind::Animate, Some("opacity")) => frames(leaf, 1.0, &vars, |v| {
                UnitRatio::quantise(v)
                    .ok()
                    .filter(|_| (0.0..=1.0).contains(&v))
            })
            .map(|frames| tracks.push(Keyframes::Opacity(track(span, frames)))),
            (LeafKind::AnimateColor, Some(attr)) => color_track(attr).and_then(|make| {
                color_frames(leaf)
                    .and_then(|frames| ending(leaf, frames))
                    .map(|frames| tracks.push(make(track(span, frames))))
            }),
            (LeafKind::Animate, Some(attr)) if color_track(attr).is_some() => color_track(attr)
                .and_then(|make| {
                    color_values_frames(leaf)
                        .and_then(|frames| ending(leaf, frames))
                        .map(|frames| tracks.push(make(track(span, frames))))
                }),
            (LeafKind::Animate, Some(attr)) if font_switch(attr).is_some() => font_switch(attr)
                .and_then(|(make, read)| {
                    switch_frames(leaf, read).map(|frames| tracks.push(make(track(span, frames))))
                }),
            (LeafKind::Animate, Some("rotate")) => frames(leaf, 0.0, &vars, degrees)
                .map(|frames| tracks.push(Keyframes::Rotate(track(span, frames)))),
            (LeafKind::AnimateTransform, _) => {
                let by = leaf.element.attr(qn!("smil:by"));
                let to = leaf.element.attr(qn!("smil:to"));
                let from = leaf.element.attr(qn!("smil:from"));
                match leaf.element.attr(qn!("svg:type")) {
                    Some("rotate") => {
                        let from_deg = from.and_then(value::number).unwrap_or(0.0);
                        let to_deg = match (to.and_then(value::number), by.and_then(value::number))
                        {
                            (Some(to), _) => Some(to),
                            (None, Some(by)) => Some(from_deg + by),
                            _ => None,
                        };
                        to_deg
                            .and_then(|to| two(degrees(from_deg)?, degrees(to)?))
                            .map(|frames| tracks.push(Keyframes::Rotate(track(span, frames))))
                    }
                    Some("scale") => {
                        let (fx, fy) = from.and_then(pair).unwrap_or((1.0, 1.0));
                        let target = to.or(by).and_then(pair);
                        target
                            .and_then(|(tx, ty)| two(scale(fx, fy)?, scale(tx, ty)?))
                            .map(|frames| tracks.push(Keyframes::Scale(track(span, frames))))
                    }
                    Some("translate") => {
                        let (fx, fy) = from.and_then(pair).unwrap_or((0.0, 0.0));
                        let target = to.or(by).and_then(pair);
                        let offset = |x: f64, y: f64| {
                            Some(Offset {
                                x: px(x * geometry.slide_w)?,
                                y: px(y * geometry.slide_h)?,
                            })
                        };
                        target
                            .and_then(|(tx, ty)| two(offset(fx, fy)?, offset(tx, ty)?))
                            .map(|frames| tracks.push(Keyframes::Translate(track(span, frames))))
                    }
                    _ => None,
                }
            }
            (LeafKind::AnimateMotion, _) => leaf
                .element
                .attr(qn!("svg:path"))
                .and_then(|path| motion_path(path, geometry))
                .and_then(|path| path_frames(&path))
                .map(|frames| tracks.push(Keyframes::Translate(track(span, frames)))),
            _ => None,
        };
        if compiled.is_none() {
            lost.push(name());
            continue;
        }
        // The tracks this leaf added play its own curve and time manipulations; its axes play them once joined.
        for axis in moves
            .iter_mut()
            .skip(moves_before)
            .chain(sizes.iter_mut().skip(sizes_before))
        {
            axis.retime = retime.clone();
            axis.easing = easing;
        }
        let added: Vec<Keyframes> = tracks.drain(before..).collect();
        for track in added {
            match track.eased(easing).folded(&retime, frames_fit) {
                Some((track, once)) => {
                    if once {
                        lost.push(format!("the repeat of {}", name()));
                    }
                    tracks.push(track);
                }
                None => lost.push(name()),
            }
        }
    }
    let translate = join(moves, 0.0, |x, y| {
        Some(Offset {
            x: px(x)?,
            y: px(y)?,
        })
    });
    match translate {
        Some(joined) => {
            for (track, retime) in joined {
                match Keyframes::Translate(track).folded(&retime, frames_fit) {
                    Some((track, once)) => {
                        if once {
                            lost.push("the repeat of its position animation".to_owned());
                        }
                        tracks.push(track);
                    }
                    None => lost.push("its position animation".to_owned()),
                }
            }
        }
        None => lost.push("its position animation".to_owned()),
    }
    let sized = join(sizes, 1.0, scale);
    match sized {
        Some(joined) => {
            for (track, retime) in joined {
                match Keyframes::Scale(track).folded(&retime, frames_fit) {
                    Some((track, once)) => {
                        if once {
                            lost.push("the repeat of its size animation".to_owned());
                        }
                        tracks.push(track);
                    }
                    None => lost.push("its size animation".to_owned()),
                }
            }
        }
        None => lost.push("its size animation".to_owned()),
    }
    if tracks.len() > TRACKS {
        tracks.truncate(TRACKS);
        if !lost.contains(&past_limit) {
            lost.push(past_limit);
        }
    }
    Compiled { tracks, lost }
}

/// Whether a track's frames fit the bounds (a clip or a switch has none).
fn frames_fit(track: &Keyframes) -> bool {
    (2..=KEYFRAMES).contains(&track.frame_count().unwrap_or(2))
}

/// A slide clip's translation: from a share of the box behind `dir` to the element's place, or from its place on
/// along `dir` when the clip closes.
fn slide_clip(dir: Dir, closes: bool, geometry: &Geometry) -> Option<Vec<Frame<Offset>>> {
    let (ux, uy) = unit(dir);
    let dx = ux * SLIGHT * geometry.width * geometry.slide_w;
    let dy = uy * SLIGHT * geometry.height * geometry.slide_h;
    let home = Offset {
        x: Px::ZERO,
        y: Px::ZERO,
    };
    if closes {
        two(
            home,
            Offset {
                x: px(dx)?,
                y: px(dy)?,
            },
        )
    } else {
        two(
            Offset {
                x: px(-dx)?,
                y: px(-dy)?,
            },
            home,
        )
    }
}

/// The unit vector of a direction, y down.
fn unit(dir: Dir) -> (f64, f64) {
    match dir {
        Dir::Left => (-1.0, 0.0),
        Dir::Right => (1.0, 0.0),
        Dir::Up => (0.0, -1.0),
        Dir::Down => (0.0, 1.0),
        Dir::LeftUp => (-1.0, -1.0),
        Dir::RightUp => (1.0, -1.0),
        Dir::LeftDown => (-1.0, 1.0),
        Dir::RightDown => (1.0, 1.0),
    }
}

/// A path's vertices as translation keyframes, timed by the distance travelled (curves by their end points).
fn path_frames(path: &[PathSeg]) -> Option<Vec<Frame<Offset>>> {
    let mut points: Vec<(f64, f64)> = Vec::new();
    let mut start = (0.0, 0.0);
    for segment in path {
        match *segment {
            PathSeg::MoveTo(x, y) => {
                start = (x.to_f64(), y.to_f64());
                points.push(start);
            }
            PathSeg::LineTo(x, y) | PathSeg::CurveTo(_, _, _, _, x, y) => {
                points.push((x.to_f64(), y.to_f64()));
            }
            PathSeg::Close => points.push(start),
        }
    }
    if points.len() > KEYFRAMES {
        return None;
    }
    if points.len() == 1 {
        let only = points.first().copied()?;
        points.push(only);
    }
    let lengths: Vec<f64> = points
        .windows(2)
        .map(|w| match w {
            [(x0, y0), (x1, y1)] => ((x1 - x0) * (x1 - x0) + (y1 - y0) * (y1 - y0)).sqrt(),
            _ => 0.0,
        })
        .collect();
    let total: f64 = lengths.iter().sum();
    let mut travelled = 0.0;
    let last = points.len() - 1;
    points
        .iter()
        .enumerate()
        .map(|(i, (x, y))| {
            let t = if i == last {
                1.0
            } else if total > 0.0 {
                travelled / total
            } else {
                i as f64 / last as f64
            };
            travelled += lengths.get(i).copied().unwrap_or(0.0);
            Some(Frame {
                offset: UnitRatio::quantise(t.min(1.0)).ok()?,
                value: Offset {
                    x: px(*x)?,
                    y: px(*y)?,
                },
            })
        })
        .collect()
}
