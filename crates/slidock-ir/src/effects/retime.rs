//! A behaviour's own time manipulations, folded into its track's frames: SMIL's `autoReverse` (the frames run
//! forward, then back, over twice the span), its acceleration and deceleration, a time filter (PowerPoint's
//! `tmFilter`, which LibreOffice writes as a leaf's `smil:keySplines`: pairs of time and progress, linear in
//! between), and its own repeat (`repeatCount`, `repeatDur`: the span, reversal included, played that many times,
//! a fraction of a time cut part-way). A track's time `t` (0 to 1 of one cycle) reads its frames at progress
//! `filter(ease(fold(t)))`; the result is exact for linear frames: a frame lands on every bend of the map, on every
//! time the map crosses a frame, and on both sides of each cycle's restart.

#[cfg(test)]
use super::keyframes::SetValue;
use super::keyframes::{
    ColorMix, ColorValue, Frame, HslShift, Keyframes, Offset, Scale, SetTrack, Track, mix_hex,
};
use super::limits;
use super::timing::Easing;
use crate::units::{Deg, Ms, Px, Ratio, UnitRatio};

/// Points an acceleration or deceleration ramp is sampled at.
const RAMP_POINTS: usize = 8;

/// A time filter: progress at times, both 0 to 1, times rising from 0 to 1 (PowerPoint's `tmFilter`,
/// `"0,0; .5,1; 1,0"`).
#[derive(Clone, Debug, PartialEq)]
pub struct TimeFilter(Vec<(f64, f64)>);

impl TimeFilter {
    /// Reads `t,p` pairs separated by `;`. `None` for a filter that does not read: a value that is not a number in
    /// [0, 1], times that fall, a first time other than 0 or a last other than 1, or more than
    /// [`limits::TIME_FILTER_POINTS`] pairs.
    pub fn parse(text: &str) -> Option<TimeFilter> {
        let mut points = Vec::new();
        for pair in text.split(';') {
            if points.len() == limits::TIME_FILTER_POINTS {
                return None;
            }
            let (t, p) = pair.split_once(',')?;
            let read = |s: &str| {
                s.trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|x| (0.0..=1.0).contains(x))
            };
            let (t, p) = (read(t)?, read(p)?);
            if points.last().is_some_and(|&(last, _)| t < last) {
                return None;
            }
            points.push((t, p));
        }
        let first = points.first()?.0;
        let last = points.last()?.0;
        (first == 0.0 && last == 1.0 && points.len() >= 2).then_some(TimeFilter(points))
    }

    fn at(&self, t: f64) -> f64 {
        piecewise(&self.0, t)
    }
}

/// What a behaviour does to its own time beyond its span.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Retime {
    /// SMIL `autoReverse` (PPTX `autoRev`): forward, then back, over twice the span.
    pub auto_reverse: bool,
    pub filter: Option<TimeFilter>,
    /// The times the span (reversal included) plays, more than 0, at most [`limits::REPEAT_COUNT`]; `None` is once.
    pub repeat: Option<f64>,
}

impl Retime {
    /// Whether it changes nothing.
    pub fn is_identity(&self) -> bool {
        !self.auto_reverse && self.filter.is_none() && self.repeat.is_none()
    }

    /// The times one span plays: the repeat, or 1.
    pub fn cycles(&self) -> f64 {
        self.repeat.unwrap_or(1.0)
    }

    /// How long a behaviour of `duration` plays with it: twice for a reversal, times its repeat; `None` past
    /// [`Ms::MAX`].
    pub fn length(&self, duration: Ms) -> Option<Ms> {
        let stretch = if self.auto_reverse { 2.0 } else { 1.0 };
        let ms = (f64::from(duration.0) * stretch * self.cycles()).round();
        (ms <= f64::from(Ms::MAX)).then_some(Ms(ms as u32))
    }

    /// The repeat a behaviour writes as a count or a total time, for one span of `duration` (`reversed` when it
    /// reverses on its own, which the span then includes): the smaller where both are written (SMIL 3.0
    /// §5.4.5). [`UnfoldedRepeat`] for a repeat that cannot fold: an indefinite one, a time over a span of 0, or a count past
    /// [`limits::REPEAT_COUNT`].
    pub fn repeat_of(
        count: Option<RepeatWritten>,
        total: Option<RepeatWritten>,
        duration: Ms,
        reversed: bool,
    ) -> Result<Option<f64>, UnfoldedRepeat> {
        let span = f64::from(duration.0) * if reversed { 2.0 } else { 1.0 };
        let from_total = match total {
            None => None,
            Some(RepeatWritten::Indefinite) => return Err(UnfoldedRepeat),
            Some(RepeatWritten::Value(ms)) if span > 0.0 => Some(ms / span),
            Some(RepeatWritten::Value(_)) => return Err(UnfoldedRepeat),
        };
        let from_count = match count {
            None => None,
            Some(RepeatWritten::Indefinite) => return Err(UnfoldedRepeat),
            Some(RepeatWritten::Value(n)) => Some(n),
        };
        let cycles = match (from_count, from_total) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => return Ok(None),
        };
        if !cycles.is_finite() || cycles <= 0.0 || cycles > f64::from(limits::REPEAT_COUNT) {
            return Err(UnfoldedRepeat);
        }
        Ok((cycles != 1.0).then_some(cycles))
    }
}

/// A behaviour's own repeat that does not fold into its track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnfoldedRepeat;

/// A repeat as a source writes it: indefinite, or a value (a count of times, or a total time in milliseconds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RepeatWritten {
    Indefinite,
    Value(f64),
}

/// The value of a polyline of rising `(x, y)` points at `x`, held flat past its ends.
fn piecewise(points: &[(f64, f64)], x: f64) -> f64 {
    let mut previous: Option<(f64, f64)> = None;
    for &(px, py) in points {
        if x <= px {
            return match previous {
                Some((qx, qy)) if px > qx => qy + (py - qy) * (x - qx) / (px - qx),
                _ => py,
            };
        }
        previous = Some((px, py));
    }
    previous.map_or(x, |(_, y)| y)
}

/// SMIL's accelerate/decelerate (SMIL 3.0 §12.3.4): constant acceleration over the first `a`, constant speed, then
/// constant deceleration over the last `d`; the speed is set so the curve ends at 1.
fn accel_decel(a: f64, d: f64, t: f64) -> f64 {
    let speed = 1.0 / (1.0 - a / 2.0 - d / 2.0);
    if t < a {
        speed * t * t / (2.0 * a)
    } else if t <= 1.0 - d {
        speed * (t - a / 2.0)
    } else {
        let rest = 1.0 - t;
        1.0 - speed * rest * rest / (2.0 * d)
    }
}

/// The map from a retimed track's time to its frames' progress, as rising-time points (exact where linear,
/// sampled along an easing's ramps). `None` for an easing it cannot sample (a named or bezier curve).
fn time_map(retime: &Retime, easing: Option<&Easing>) -> Option<Vec<(f64, f64)>> {
    // The simple time's breakpoints: the fold, and the ramps of an acceleration.
    let mut simple: Vec<f64> = vec![0.0, 1.0];
    let ease: Box<dyn Fn(f64) -> f64> = match easing {
        None | Some(Easing::Linear) => Box::new(|t| t),
        Some(Easing::AccelDecel { accel, decel }) => {
            let (a, d) = (accel.to_f64(), decel.to_f64());
            for i in 1..RAMP_POINTS {
                let f = i as f64 / RAMP_POINTS as f64;
                if a > 0.0 {
                    simple.push(a * f);
                }
                if d > 0.0 {
                    simple.push(1.0 - d + d * f);
                }
            }
            simple.extend([a, 1.0 - d]);
            Box::new(move |t| {
                if a == 0.0 && d == 0.0 {
                    t
                } else {
                    accel_decel(a, d, t)
                }
            })
        }
        Some(_) => return None,
    };
    let filter = |p: f64| retime.filter.as_ref().map_or(p, |filter| filter.at(p));
    // The filter's bends, at the simple times the eased progress reaches them (by bisection: the ease rises).
    if let Some(TimeFilter(points)) = &retime.filter {
        for &(x, _) in points {
            simple.push(invert(&ease, x));
        }
    }
    simple.retain(|t| (0.0..=1.0).contains(t));
    simple.sort_by(f64::total_cmp);
    simple.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    let times: Vec<f64> = if retime.auto_reverse {
        let forward = simple.iter().map(|t| t / 2.0);
        let back = simple.iter().rev().map(|t| 1.0 - t / 2.0);
        let mut times: Vec<f64> = forward.chain(back).collect();
        times.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        times
    } else {
        simple
    };
    let fold = |t: f64| {
        if retime.auto_reverse {
            if t <= 0.5 { 2.0 * t } else { 2.0 - 2.0 * t }
        } else {
            t
        }
    };
    let cycle: Vec<(f64, f64)> = times
        .into_iter()
        .map(|t| (t, filter(ease(fold(t)).clamp(0.0, 1.0))))
        .collect();
    let cycles = retime.cycles();
    if cycles == 1.0 {
        return Some(cycle);
    }
    // Each cycle in turn, over its share of the time; the next starts where the last ends (a jump back unless the
    // cycle ends where it starts), and a fraction of a cycle stops part-way.
    let whole = cycles.floor();
    let part = cycles - whole;
    if (whole as usize).saturating_mul(cycle.len()) > limits::KEYFRAMES {
        return None;
    }
    let mut map = Vec::new();
    for k in 0..whole as usize {
        map.extend(cycle.iter().map(|&(t, p)| ((k as f64 + t) / cycles, p)));
    }
    if part > 0.0 {
        map.extend(
            cycle
                .iter()
                .filter(|&&(t, _)| t < part)
                .map(|&(t, p)| ((whole + t) / cycles, p)),
        );
        map.push((1.0, piecewise(&cycle, part)));
    }
    Some(map)
}

/// The simple time a rising `ease` reaches `progress` at.
fn invert(ease: &dyn Fn(f64) -> f64, progress: f64) -> f64 {
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..48 {
        let mid = (lo + hi) / 2.0;
        if ease(mid) < progress {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi
}

/// A value frames interpolate.
pub(crate) trait Lerp: Sized + Copy + PartialEq {
    /// `self` toward `other` by `f` (0 to 1); `None` where the two do not mix.
    fn lerp(self, other: Self, f: f64) -> Option<Self>;
    /// Whether each value holds until the next frame instead of blending.
    const DISCRETE: bool = false;
}

fn steps(a: i64, b: i64, f: f64) -> i64 {
    (a as f64 + (b - a) as f64 * f).round() as i64
}

macro_rules! lerp_unit {
    ($($unit:ty),*) => {$(
        impl Lerp for $unit {
            fn lerp(self, other: Self, f: f64) -> Option<Self> {
                Some(<$unit>::from_steps(steps(self.steps(), other.steps(), f)))
            }
        }
    )*};
}

lerp_unit!(UnitRatio, Ratio, Deg, Px);

impl Lerp for Offset {
    fn lerp(self, other: Self, f: f64) -> Option<Self> {
        Some(Offset {
            x: self.x.lerp(other.x, f)?,
            y: self.y.lerp(other.y, f)?,
        })
    }
}

impl Lerp for Scale {
    fn lerp(self, other: Self, f: f64) -> Option<Self> {
        Some(Scale {
            x: self.x.lerp(other.x, f)?,
            y: self.y.lerp(other.y, f)?,
        })
    }
}

impl Lerp for ColorValue {
    fn lerp(self, other: Self, f: f64) -> Option<Self> {
        if f == 0.0 || self == other {
            return Some(self);
        }
        if f == 1.0 {
            return Some(other);
        }
        match (self, other) {
            (ColorValue::Hex(a), ColorValue::Hex(b)) => Some(ColorValue::Hex(mix_hex(a, b, f))),
            (ColorValue::Shift(a), ColorValue::Shift(b)) => Some(ColorValue::Shift(HslShift {
                h: a.h.lerp(b.h, f)?,
                s: a.s.lerp(b.s, f)?,
                l: a.l.lerp(b.l, f)?,
            })),
            // The element's own colour is the deck builder's to know: the mix waits for it.
            (ColorValue::Shift(shift), ColorValue::Hex(toward)) => {
                Some(ColorValue::Mix(ColorMix {
                    shift,
                    toward,
                    by: UnitRatio::quantise(f).ok()?,
                }))
            }
            (ColorValue::Hex(toward), ColorValue::Shift(shift)) => {
                Some(ColorValue::Mix(ColorMix {
                    shift,
                    toward,
                    by: UnitRatio::quantise(1.0 - f).ok()?,
                }))
            }
            _ => None,
        }
    }
}

impl Lerp for bool {
    fn lerp(self, _: Self, _: f64) -> Option<Self> {
        Some(self)
    }
    const DISCRETE: bool = true;
}

/// A track's value at progress `p`: linear between frames, or the last frame at or before `p` for a discrete value.
fn value_at<V: Lerp>(frames: &[Frame<V>], p: f64) -> Option<V> {
    let first = frames.first()?;
    let mut previous = first;
    for frame in frames {
        let at = frame.offset.to_f64();
        if V::DISCRETE {
            if at > p {
                return Some(previous.value);
            }
        } else if p <= at {
            let from = previous.offset.to_f64();
            if at <= from {
                return Some(frame.value);
            }
            return previous.value.lerp(frame.value, (p - from) / (at - from));
        }
        previous = frame;
    }
    Some(previous.value)
}

/// `track` read through `retime`. Frames land on every point of the time map and on every time the map crosses a
/// frame's offset; a discrete track's frame holds the value just after its time. A cycle's restart keeps both
/// frames at its time: the end of one cycle, then the start of the next.
fn retime_track<V: Lerp>(track: &Track<V>, retime: &Retime) -> Option<Track<V>> {
    let map = time_map(retime, track.easing.as_ref())?;
    let offsets: Vec<f64> = track.frames.iter().map(|f| f.offset.to_f64()).collect();
    let mut points: Vec<(f64, f64)> = Vec::with_capacity(map.len());
    for pair in map.windows(2) {
        let [(t0, p0), (t1, p1)] = *pair else {
            continue;
        };
        points.push((t0, p0));
        if t1 <= t0 {
            continue;
        }
        let (lo, hi) = if p0 <= p1 { (p0, p1) } else { (p1, p0) };
        let mut crossings: Vec<f64> = offsets
            .iter()
            .filter(|&&o| lo < o && o < hi)
            .map(|&o| t0 + (t1 - t0) * (o - p0) / (p1 - p0))
            .collect();
        crossings.sort_by(f64::total_cmp);
        for t in crossings {
            points.push((t, p0 + (p1 - p0) * (t - t0) / (t1 - t0)));
            if points.len() > 4 * limits::KEYFRAMES {
                return None;
            }
        }
    }
    points.extend(map.last());
    let mut frames: Vec<Frame<V>> = Vec::with_capacity(points.len());
    for (i, &(t, p)) in points.iter().enumerate() {
        let p = match points.get(i + 1) {
            Some(&(next_t, next_p)) if V::DISCRETE && next_t > t => (p + next_p) / 2.0,
            _ => p,
        };
        let frame = Frame {
            offset: UnitRatio::quantise(t.clamp(0.0, 1.0)).ok()?,
            value: value_at(&track.frames, p)?,
        };
        if frames.last() == Some(&frame) {
            continue;
        }
        // Of close times that quantise to one offset, the first stays; a restart keeps both.
        let restart = i > 0 && points.get(i - 1).is_some_and(|&(prev, _)| prev == t);
        if !restart
            && frames
                .last()
                .is_some_and(|last| last.offset == frame.offset)
        {
            continue;
        }
        frames.push(frame);
    }
    if frames.len() > limits::KEYFRAMES {
        return None;
    }
    if let Some(last) = frames.last_mut() {
        last.offset = UnitRatio::from_steps(100_000);
    }
    let eased = !matches!(track.easing, None | Some(Easing::Linear));
    Some(Track {
        begin_ms: track.begin_ms,
        duration_ms: retime.length(track.duration_ms)?,
        // A sampled ease leaves the frames linear; a track with no curve of its own keeps the step's.
        easing: if eased {
            Some(Easing::Linear)
        } else {
            track.easing
        },
        frames,
    })
}

impl Keyframes {
    /// The track with a behaviour's own time manipulations folded into its frames (an auto-reverse doubles its
    /// span). `None` where the result has no frames to hold it (a clip reveal), would need more than
    /// [`limits::KEYFRAMES`] frames or end after an hour, or the track's easing does not sample.
    pub fn retimed(&self, retime: &Retime) -> Option<Keyframes> {
        if retime.is_identity() {
            return Some(self.clone());
        }
        let out = match self {
            Keyframes::Opacity(t) => Keyframes::Opacity(retime_track(t, retime)?),
            Keyframes::Translate(t) => Keyframes::Translate(retime_track(t, retime)?),
            Keyframes::Rotate(t) => Keyframes::Rotate(retime_track(t, retime)?),
            Keyframes::Scale(t) => Keyframes::Scale(retime_track(t, retime)?),
            Keyframes::Blur(t) => Keyframes::Blur(retime_track(t, retime)?),
            Keyframes::Text(t) => Keyframes::Text(retime_track(t, retime)?),
            Keyframes::Fill(t) => Keyframes::Fill(retime_track(t, retime)?),
            Keyframes::Line(t) => Keyframes::Line(retime_track(t, retime)?),
            Keyframes::Bold(t) => Keyframes::Bold(retime_track(t, retime)?),
            Keyframes::Italic(t) => Keyframes::Italic(retime_track(t, retime)?),
            Keyframes::Underline(t) => Keyframes::Underline(retime_track(t, retime)?),
            // A switch holds its value over its span whichever way and however often the span runs.
            Keyframes::Set(set) => Keyframes::Set(SetTrack {
                duration_ms: retime.length(set.duration_ms)?,
                ..set.clone()
            }),
            Keyframes::ClipPath(_) => return None,
        };
        let (begin, duration) = out.span();
        (u64::from(begin.0) + u64::from(duration.0) <= u64::from(Ms::MAX)).then_some(out)
    }

    /// [`Keyframes::retimed`] where the result `fits`; else, for a repeat that does not, the track retimed once,
    /// with `true` (the repeat is left out). `None` when neither fits.
    pub fn folded(
        &self,
        retime: &Retime,
        fits: impl Fn(&Keyframes) -> bool,
    ) -> Option<(Keyframes, bool)> {
        if let Some(track) = self.retimed(retime).filter(&fits) {
            return Some((track, false));
        }
        retime.repeat?;
        let once = Retime {
            repeat: None,
            ..retime.clone()
        };
        self.retimed(&once).filter(&fits).map(|track| (track, true))
    }
}

#[cfg(test)]
mod tests {
    use super::super::params::HexColor;
    use super::*;

    fn unit(x: f64) -> UnitRatio {
        UnitRatio::quantise(x).unwrap()
    }

    fn opacity(frames: &[(f64, f64)], easing: Option<Easing>) -> Track<UnitRatio> {
        Track {
            begin_ms: Ms(100),
            duration_ms: Ms(500),
            easing,
            frames: frames
                .iter()
                .map(|&(o, v)| Frame {
                    offset: unit(o),
                    value: unit(v),
                })
                .collect(),
        }
    }

    fn pairs(track: &Track<UnitRatio>) -> Vec<(f64, f64)> {
        track
            .frames
            .iter()
            .map(|f| (f.offset.to_f64(), f.value.to_f64()))
            .collect()
    }

    fn reverse() -> Retime {
        Retime {
            auto_reverse: true,
            ..Retime::default()
        }
    }

    #[test]
    fn an_auto_reverse_runs_the_frames_forward_then_back_over_twice_the_span() {
        let track = opacity(&[(0.0, 1.0), (0.5, 0.2), (1.0, 0.0)], None);
        let Some(Keyframes::Opacity(out)) = Keyframes::Opacity(track).retimed(&reverse()) else {
            panic!("an opacity track folds");
        };
        assert_eq!(out.begin_ms, Ms(100));
        assert_eq!(out.duration_ms, Ms(1000));
        assert_eq!(out.easing, None);
        assert_eq!(
            pairs(&out),
            [(0.0, 1.0), (0.25, 0.2), (0.5, 0.0), (0.75, 0.2), (1.0, 1.0)]
        );
    }

    #[test]
    fn a_time_filter_reads_the_frames_at_its_progress() {
        // PowerPoint's flash bulb fade: out to half and back.
        let filter = TimeFilter::parse("0,0; .2, .5; .8,.5; 1,0").unwrap();
        let track = opacity(&[(0.0, 1.0), (1.0, 0.0)], None);
        let retime = Retime {
            filter: Some(filter),
            ..Retime::default()
        };
        let Some(Keyframes::Opacity(out)) = Keyframes::Opacity(track).retimed(&retime) else {
            panic!("an opacity track filters");
        };
        assert_eq!(out.duration_ms, Ms(500));
        assert_eq!(
            pairs(&out),
            [(0.0, 1.0), (0.2, 0.5), (0.8, 0.5), (1.0, 1.0)]
        );
    }

    #[test]
    fn an_eased_fold_samples_the_ramps_and_turns_linear() {
        let easing = Easing::AccelDecel {
            accel: unit(0.5),
            decel: UnitRatio::ZERO,
        };
        let track = opacity(&[(0.0, 0.0), (1.0, 1.0)], Some(easing));
        let Some(Keyframes::Opacity(out)) = Keyframes::Opacity(track).retimed(&reverse()) else {
            panic!("an eased track folds");
        };
        assert_eq!(out.easing, Some(Easing::Linear));
        let frames = pairs(&out);
        assert_eq!(frames.first(), Some(&(0.0, 0.0)));
        assert_eq!(frames.last(), Some(&(1.0, 0.0)));
        // The middle is the forward end, and the ramp starts slow: a quarter of the way in, under a quarter.
        assert!(frames.contains(&(0.5, 1.0)));
        let quarter = frames.iter().find(|(o, _)| *o == 0.125).unwrap();
        assert!(quarter.1 < 0.25, "{quarter:?}");
        // The way back mirrors the way out.
        for (o, v) in &frames {
            let mirror = frames
                .iter()
                .find(|(m, _)| (m - (1.0 - o)).abs() < 1e-6)
                .unwrap();
            assert!((mirror.1 - v).abs() < 1e-4, "{o} {v} {mirror:?}");
        }
    }

    #[test]
    fn a_discrete_track_holds_each_value_through_the_fold() {
        let track = Track {
            begin_ms: Ms(0),
            duration_ms: Ms(400),
            easing: None,
            frames: vec![
                Frame {
                    offset: UnitRatio::ZERO,
                    value: false,
                },
                Frame {
                    offset: unit(0.5),
                    value: true,
                },
                Frame {
                    offset: unit(1.0),
                    value: true,
                },
            ],
        };
        let Some(Keyframes::Bold(out)) = Keyframes::Bold(track).retimed(&reverse()) else {
            panic!("a switch track folds");
        };
        let frames: Vec<(f64, bool)> = out
            .frames
            .iter()
            .map(|f| (f.offset.to_f64(), f.value))
            .collect();
        assert_eq!(
            frames,
            [
                (0.0, false),
                (0.25, true),
                (0.5, true),
                (0.75, false),
                (1.0, false)
            ]
        );
        assert_eq!(out.duration_ms, Ms(800));
    }

    #[test]
    fn a_set_doubles_its_span_and_a_clip_does_not_fold() {
        let set = Keyframes::Set(SetTrack {
            begin_ms: Ms(0),
            duration_ms: Ms(250),
            to: SetValue::Bold(true),
        });
        let Some(Keyframes::Set(out)) = set.retimed(&reverse()) else {
            panic!("a set folds");
        };
        assert_eq!(out.duration_ms, Ms(500));
        assert_eq!(set.retimed(&Retime::default()), Some(set.clone()));
    }

    #[test]
    fn colours_mix_by_channel_and_shifts_by_part() {
        let red = ColorValue::Hex(HexColor::rgb(0xff, 0, 0));
        let blue = ColorValue::Hex(HexColor::rgb(0, 0, 0xff));
        assert_eq!(
            red.lerp(blue, 0.5),
            Some(ColorValue::Hex(HexColor::rgb(0x80, 0, 0x80)))
        );
        let own = ColorValue::Shift(HslShift::NONE);
        assert_eq!(own.lerp(red, 1.0), Some(red));
        // The own colour toward a colour waits for the element's colour, either way round.
        let mixed = own.lerp(red, 0.25).unwrap();
        assert_eq!(
            mixed,
            ColorValue::Mix(ColorMix {
                shift: HslShift::NONE,
                toward: HexColor::rgb(0xff, 0, 0),
                by: unit(0.25),
            })
        );
        assert_eq!(red.lerp(own, 0.75), Some(mixed));
        assert_eq!(
            mixed.on(HexColor::rgb(0, 0, 0xff)),
            HexColor::rgb(0x40, 0, 0xbf)
        );
        assert_eq!(mixed.lerp(red, 0.5), None);
    }

    /// LibreOffice's vertical highlight: its own colour to #333399, accelerated, reversing on its own.
    #[test]
    fn a_colour_from_its_own_folds_through_an_ease_and_a_reversal() {
        let to = HexColor::rgb(0x33, 0x33, 0x99);
        let track = Track {
            begin_ms: Ms(0),
            duration_ms: Ms(500),
            easing: Some(Easing::AccelDecel {
                accel: unit(0.5),
                decel: UnitRatio::ZERO,
            }),
            frames: vec![
                Frame {
                    offset: UnitRatio::ZERO,
                    value: ColorValue::Shift(HslShift::NONE),
                },
                Frame {
                    offset: unit(1.0),
                    value: ColorValue::Hex(to),
                },
            ],
        };
        let Some(Keyframes::Fill(out)) = Keyframes::Fill(track).retimed(&reverse()) else {
            panic!("a colour from its own folds");
        };
        assert_eq!(out.duration_ms, Ms(1000));
        let own = HexColor::WHITE;
        let colours: Vec<HexColor> = out.frames.iter().map(|f| f.value.on(own)).collect();
        assert_eq!(colours.first(), Some(&own));
        assert_eq!(colours.last(), Some(&own));
        assert!(colours.contains(&to));
        assert!(
            out.frames
                .iter()
                .any(|f| matches!(f.value, ColorValue::Mix(_)))
        );
    }

    fn repeat(cycles: f64) -> Retime {
        Retime {
            repeat: Some(cycles),
            ..Retime::default()
        }
    }

    #[test]
    fn a_repeat_plays_each_cycle_in_turn_and_restarts_at_each_end() {
        let track = opacity(&[(0.0, 1.0), (1.0, 0.0)], None);
        let Some(Keyframes::Opacity(out)) = Keyframes::Opacity(track.clone()).retimed(&repeat(2.0))
        else {
            panic!("a repeat folds");
        };
        assert_eq!(out.duration_ms, Ms(1000));
        assert_eq!(
            pairs(&out),
            [(0.0, 1.0), (0.5, 0.0), (0.5, 1.0), (1.0, 0.0)]
        );
        // A fraction of a cycle stops part-way.
        let Some(Keyframes::Opacity(out)) = Keyframes::Opacity(track.clone()).retimed(&repeat(1.5))
        else {
            panic!("a fractional repeat folds");
        };
        assert_eq!(out.duration_ms, Ms(750));
        let frames = pairs(&out);
        assert_eq!(frames.len(), 4, "{frames:?}");
        assert_eq!(frames.last(), Some(&(1.0, 0.5)));
        // A reversal repeats with its span: it ends where it starts, so no restart shows.
        let retime = Retime {
            auto_reverse: true,
            repeat: Some(3.0),
            ..Retime::default()
        };
        let Some(Keyframes::Opacity(out)) = Keyframes::Opacity(track).retimed(&retime) else {
            panic!("a reversed repeat folds");
        };
        assert_eq!(out.duration_ms, Ms(3000));
        assert_eq!(out.frames.first().map(|f| f.value), Some(unit(1.0)));
        assert_eq!(out.frames.last().map(|f| f.value), Some(unit(1.0)));
        assert_eq!(
            out.frames
                .iter()
                .filter(|f| f.value == UnitRatio::ZERO)
                .count(),
            3
        );
    }

    #[test]
    fn a_switch_repeats_its_span_and_a_repeat_past_the_frames_fails() {
        let set = Keyframes::Set(SetTrack {
            begin_ms: Ms(0),
            duration_ms: Ms(250),
            to: SetValue::Bold(true),
        });
        assert_eq!(
            set.retimed(&repeat(3.0)).map(|s| s.span()),
            Some((Ms(0), Ms(750)))
        );
        let track = opacity(&[(0.0, 1.0), (1.0, 0.0)], None);
        assert_eq!(Keyframes::Opacity(track).retimed(&repeat(40.0)), None);
    }

    #[test]
    fn a_written_repeat_reads_as_cycles_and_fails_closed() {
        use RepeatWritten::{Indefinite, Value};
        let ms = Ms(500);
        assert_eq!(Retime::repeat_of(None, None, ms, false), Ok(None));
        assert_eq!(
            Retime::repeat_of(Some(Value(3.0)), None, ms, false),
            Ok(Some(3.0))
        );
        assert_eq!(
            Retime::repeat_of(Some(Value(1.0)), None, ms, false),
            Ok(None)
        );
        // A total time counts spans; a reversal is in the span.
        assert_eq!(
            Retime::repeat_of(None, Some(Value(1500.0)), ms, false),
            Ok(Some(3.0))
        );
        assert_eq!(
            Retime::repeat_of(None, Some(Value(1500.0)), ms, true),
            Ok(Some(1.5))
        );
        // Both: the smaller.
        assert_eq!(
            Retime::repeat_of(Some(Value(4.0)), Some(Value(1000.0)), ms, false),
            Ok(Some(2.0))
        );
        for (count, total, dur) in [
            (Some(Indefinite), None, ms),
            (None, Some(Indefinite), ms),
            (None, Some(Value(100.0)), Ms(0)),
            (Some(Value(0.0)), None, ms),
            (Some(Value(f64::NAN)), None, ms),
            (Some(Value(1_001.0)), None, ms),
        ] {
            assert_eq!(
                Retime::repeat_of(count, total, dur, false),
                Err(UnfoldedRepeat),
                "{count:?} {total:?}"
            );
        }
    }

    #[test]
    fn a_time_filter_reads_bounded_and_fails_closed() {
        assert!(TimeFilter::parse("0,0;1,1").is_some());
        assert!(TimeFilter::parse("0,0; 0.5,1; 1,0").is_some());
        for bad in [
            "",
            "0,0",
            "0,0;0.5",
            "0,0;2,1",
            "0.1,0;1,1",
            "0,0;0.9,1",
            "0,0;0.6,1;0.5,0;1,1",
            "0,NaN;1,1",
            "0,0;1,-1",
        ] {
            assert_eq!(TimeFilter::parse(bad), None, "{bad}");
        }
        let long = (0..=limits::TIME_FILTER_POINTS)
            .map(|i| format!("{},0", i as f64 / limits::TIME_FILTER_POINTS as f64))
            .collect::<Vec<_>>()
            .join(";");
        assert_eq!(TimeFilter::parse(&long), None);
    }

    #[test]
    fn a_fold_past_the_frame_bound_or_an_hour_fails() {
        let many: Vec<(f64, f64)> = (0..40).map(|i| (i as f64 / 39.0, 0.5)).collect();
        assert_eq!(
            Keyframes::Opacity(opacity(&many, None)).retimed(&reverse()),
            None
        );
        let mut long = opacity(&[(0.0, 0.0), (1.0, 1.0)], None);
        long.duration_ms = Ms(Ms::MAX / 2 + 1);
        assert_eq!(Keyframes::Opacity(long).retimed(&reverse()), None);
    }
}
