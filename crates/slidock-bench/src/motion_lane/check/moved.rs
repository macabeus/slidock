//! How a step or a transition moved, from its pauses: whether it moves at all, which way, and whether its animated
//! nodes end where they settle.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use slidock_bench_schema::lanes::{ANTIALIAS_CHANNEL, ANTIALIAS_PIXELS};

use super::{Frames, NOISE_CHANNEL, NOISE_PIXELS, STILL_PX};
use crate::motion_lane::drive::{Animation, NodeProbe, Sample};
use crate::motion_lane::model::{Switch, Switched};

/// A span's five pauses: start, a quarter, the middle, three quarters, end.
pub struct Pauses<'a> {
    pub start: &'a Sample,
    pub mids: [&'a Sample; 3],
    pub end: &'a Sample,
}

impl Pauses<'_> {
    fn all(&self) -> impl Iterator<Item = &Sample> {
        std::iter::once(self.start)
            .chain(self.mids.iter().copied())
            .chain(std::iter::once(self.end))
    }

    pub fn middle(&self) -> &Sample {
        self.mids[1]
    }
}

/// The union of `nodes`' boxes over every pause, one pixel wider each side; `None` when none has an area.
pub fn region(pauses: &Pauses<'_>, nodes: &BTreeSet<u64>) -> Option<[f64; 4]> {
    let samples: Vec<Sample> = pauses.all().cloned().collect();
    boxes(&samples, nodes)
}

/// The union of `nodes`' boxes in `samples`, one pixel wider each side; `None` when none has an area.
pub fn boxes(samples: &[Sample], nodes: &BTreeSet<u64>) -> Option<[f64; 4]> {
    let mut bounds: Option<(f64, f64, f64, f64)> = None;
    for probe in samples.iter().flat_map(|s| s.nodes.iter()) {
        let [x, y, w, h] = probe.rect;
        if !nodes.contains(&probe.node) || w <= 0.0 || h <= 0.0 {
            continue;
        }
        bounds = Some(match bounds {
            None => (x, y, x + w, y + h),
            Some((a, b, c, d)) => (a.min(x), b.min(y), c.max(x + w), d.max(y + h)),
        });
    }
    bounds.map(|(a, b, c, d)| [a - 1.0, b - 1.0, c - a + 2.0, d - b + 2.0])
}

/// Whether some animated property of `nodes` but those of `switched` (a set's key, which switches) is, at a quarter,
/// the middle or three quarters, at neither its start nor its end value.
fn poses_move(pauses: &Pauses<'_>, nodes: &BTreeSet<u64>, switched: &BTreeSet<&str>) -> bool {
    nodes.iter().any(|node| {
        let (Some(start), Some(end)) = (pauses.start.node(*node), pauses.end.node(*node)) else {
            return false;
        };
        pauses.mids.iter().any(|mid| {
            mid.node(*node).is_some_and(|probe| {
                probe.pose.iter().any(|(key, value)| {
                    key != MASK_IMAGE
                        && !switched.contains(key.as_str())
                        && start.pose.get(key) != Some(value)
                        && end.pose.get(key) != Some(value)
                })
            })
        })
    })
}

/// Whether the span moves: its poses but `switched`'s do (see [`poses_move`]), and, where its frames change at all inside `region`,
/// some middle frame differs from both its first and its last. Two frames differ when more than a twentieth of the
/// span's largest change differs (so a composited layer's noise next to a large change is none), or when more pixels
/// than the antialias bound differ beyond its channel bound (a change the player itself would not call
/// antialiasing): a small change, the text of a by-character build, still counts next to a large jump at an end, the
/// shape that hides when such an exit ends. `Err` says how it does not.
pub fn moves(
    frames: &Frames<'_>,
    pauses: &Pauses<'_>,
    nodes: &BTreeSet<u64>,
    region: Option<[f64; 4]>,
    switched: &BTreeSet<&str>,
) -> Result<(), String> {
    if !poses_move(pauses, nodes, switched) {
        return Err(
            "no property it animates is between its start and end values at a quarter, the middle or three quarters"
                .to_owned(),
        );
    }
    let pixels = |a: &Sample, b: &Sample| -> u32 {
        if a.frame == b.frame {
            return 0;
        }
        (frames.change)(&a.frame, &b.frame, region, NOISE_CHANNEL).map_or(0, |c| c.pixels)
    };
    let ends = pixels(pauses.start, pauses.end);
    let middles: Vec<(u32, u32)> = pauses
        .mids
        .iter()
        .map(|mid| (pixels(mid, pauses.start), pixels(mid, pauses.end)))
        .collect();
    let most = middles
        .iter()
        .map(|(a, b)| (*a).max(*b))
        .fold(ends, u32::max);
    if most <= NOISE_PIXELS {
        // Nothing it animates shows a change (a transparent shape, an empty text box): its poses decide.
        return Ok(());
    }
    let bound = NOISE_PIXELS.max(most / 20);
    let differs = |a: &Sample, b: &Sample, pixels: u32| {
        pixels > bound
            || (pixels > ANTIALIAS_PIXELS
                && (frames.change)(&a.frame, &b.frame, region, ANTIALIAS_CHANNEL)
                    .is_some_and(|c| c.pixels > ANTIALIAS_PIXELS))
    };
    let moved = pauses
        .mids
        .iter()
        .zip(&middles)
        .any(|(mid, (a, b))| differs(mid, pauses.start, *a) && differs(mid, pauses.end, *b));
    if moved {
        Ok(())
    } else {
        Err("no frame between its ends differs from both".to_owned())
    }
}

/// A `dir` token as a unit vector in frame coordinates (y down).
pub fn direction(token: &str) -> Option<(f64, f64)> {
    let d = std::f64::consts::FRAC_1_SQRT_2;
    Some(match token {
        "l" => (-1.0, 0.0),
        "r" => (1.0, 0.0),
        "u" => (0.0, -1.0),
        "d" => (0.0, 1.0),
        "lu" => (-d, -d),
        "ru" => (d, -d),
        "ld" => (-d, d),
        "rd" => (d, d),
        _ => return None,
    })
}

/// A displacement's length.
fn norm(v: (f64, f64)) -> f64 {
    (v.0 * v.0 + v.1 * v.1).sqrt()
}

/// The `dir` token nearest a displacement, or `none` when it is too small.
pub fn token(v: (f64, f64)) -> &'static str {
    let length = norm(v);
    if length <= STILL_PX {
        return "none";
    }
    let tokens = ["l", "r", "u", "d", "lu", "ru", "ld", "rd"];
    let mut best = ("none", f64::NEG_INFINITY);
    for t in tokens {
        if let Some((x, y)) = direction(t) {
            let dot = (v.0 * x + v.1 * y) / length;
            if dot > best.1 {
                best = (t, dot);
            }
        }
    }
    best.0
}

/// The `dir` an effect's params name, when the effect's `dir` is the direction its content moves.
pub fn directed<'a>(
    effect: Option<&str>,
    params: Option<&'a Value>,
    effects: &[&str],
) -> Option<&'a str> {
    let effect = effect?;
    if !effects.contains(&effect) {
        return None;
    }
    params?.get("dir")?.as_str()
}

/// How content moved over a span, measured on `nodes` (the mean centre of their boxes) or, when their boxes stay put
/// (a clip), on where the frames changed inside `region`. An entrance (`arriving`) is compared at its middle with its
/// end, so the displacement points back where it comes from; an exit with its start, so it points where it goes.
/// Returns the direction the content moves, or `None` when nothing measures it.
pub fn motion(
    frames: &Frames<'_>,
    pauses: &Pauses<'_>,
    nodes: &BTreeSet<u64>,
    region: Option<[f64; 4]>,
    arriving: bool,
) -> Option<(f64, f64)> {
    let reference = if arriving { pauses.end } else { pauses.start };
    let middle = pauses.middle();
    let centre = |sample: &Sample| -> Option<(f64, f64)> {
        let probes: Vec<&NodeProbe> = nodes
            .iter()
            .filter_map(|n| sample.node(*n))
            .filter(|p| p.rect[2] > 0.0 && p.rect[3] > 0.0)
            .collect();
        if probes.is_empty() {
            return None;
        }
        let n = probes.len() as f64;
        let (x, y) = probes.iter().fold((0.0, 0.0), |(x, y), p| {
            let (cx, cy) = p.center();
            (x + cx, y + cy)
        });
        Some((x / n, y / n))
    };
    let flip = |v: (f64, f64)| if arriving { (-v.0, -v.1) } else { v };
    if let (Some(a), Some(b)) = (centre(middle), centre(reference)) {
        let v = (a.0 - b.0, a.1 - b.1);
        if norm(v) > STILL_PX {
            return Some(flip(v));
        }
    }
    // A clip: where the middle frame has changed from the side the span leaves, against where the whole span does.
    let (from, to) = if arriving {
        (pauses.start, pauses.end)
    } else {
        (pauses.end, pauses.start)
    };
    let part = (frames.change)(&middle.frame, &from.frame, region, NOISE_CHANNEL)?;
    let whole = (frames.change)(&to.frame, &from.frame, region, NOISE_CHANNEL)?;
    if part.pixels <= NOISE_PIXELS || whole.pixels <= NOISE_PIXELS {
        return None;
    }
    let (Some(a), Some(b)) = (part.center, whole.center) else {
        return None;
    };
    Some(flip((a.0 - b.0, a.1 - b.1)))
}

/// Whether content that moved by `seen` moved along `dir`: `Err` names where it went.
pub fn along(dir: &str, seen: Option<(f64, f64)>) -> Option<Result<(), String>> {
    let want = direction(dir)?;
    let seen = seen?;
    let dot = seen.0 * want.0 + seen.1 * want.1;
    Some(if dot > STILL_PX {
        Ok(())
    } else {
        Err(format!("its content moves {}", token(seen)))
    })
}

/// The numbers in a computed value, and the text between them.
fn split(value: &str) -> (String, Vec<f64>) {
    let mut skeleton = String::new();
    let mut numbers = Vec::new();
    let mut number = String::new();
    let flush = |number: &mut String, skeleton: &mut String, numbers: &mut Vec<f64>| {
        if !number.is_empty() {
            match number.parse::<f64>() {
                Ok(n) => {
                    numbers.push(n);
                    skeleton.push('#');
                }
                Err(_) => skeleton.push_str(number),
            }
            number.clear();
        }
    };
    let mut previous = ' ';
    for c in value.chars() {
        // A digit inside a name (`matrix3d`) is part of the name.
        let starts = number.is_empty()
            && !previous.is_ascii_alphabetic()
            && (c.is_ascii_digit() || c == '.' || c == '-');
        let continues = !number.is_empty() && (c.is_ascii_digit() || c == '.' || c == 'e');
        if starts || continues {
            number.push(c);
        } else {
            flush(&mut number, &mut skeleton, &mut numbers);
            skeleton.push(c);
        }
        previous = c;
    }
    flush(&mut number, &mut skeleton, &mut numbers);
    (skeleton, numbers)
}

/// A computed value with its identity forms written `none`: an identity matrix, a zero translation or rotation, a unit
/// scale, a clip inset by nothing.
fn canonical(key: &str, value: &str) -> String {
    let value = value.trim();
    let (skeleton, numbers) = split(value);
    let all = |n: f64| numbers.iter().all(|x| (x - n).abs() <= 1e-3);
    let identity = match key {
        "transform" => match skeleton.as_str() {
            "matrix(#, #, #, #, #, #)" => {
                numbers.len() == 6
                    && [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
                        .iter()
                        .zip(&numbers)
                        .all(|(a, b)| (a - b).abs() <= 1e-3)
            }
            // A perspective alone (m34) moves nothing of a flat element.
            _ if skeleton.starts_with("matrix3d(") => {
                numbers.len() == 16
                    && numbers.iter().enumerate().all(|(i, n)| {
                        let want = if i % 5 == 0 { 1.0 } else { 0.0 };
                        i == 11 || (n - want).abs() <= 1e-3
                    })
            }
            _ => false,
        },
        "translate" | "rotate" => all(0.0),
        "scale" => all(1.0),
        "clipPath" => skeleton.starts_with("inset(") && all(0.0),
        _ => false,
    };
    if identity && !numbers.is_empty() {
        "none".to_owned()
    } else {
        value.to_owned()
    }
}

/// Whether two computed values of one property are the same: equal once identities read `none`, numbers within a
/// thousandth (a hundredth of a pixel for lengths).
pub fn same_value(key: &str, a: &str, b: &str) -> bool {
    let (a, b) = (canonical(key, a), canonical(key, b));
    if a == b {
        return true;
    }
    let ((sa, na), (sb, nb)) = (split(&a), split(&b));
    sa == sb && na.len() == nb.len() && na.iter().zip(&nb).all(|(x, y)| (x - y).abs() <= 1e-3)
}

/// The animated nodes at the end of their animations against the same nodes once settled.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EndPoses {
    /// What differs: `<node> <property> ends at …, settles at …`, or a node that shows on one side only.
    pub differences: Vec<String>,
    /// Every animated property of every node compared: none was a mask's custom property read by another mask image
    /// once settled, and no node was gone.
    pub complete: bool,
}

/// How the animated nodes at the end of their animations differ from the same nodes once settled: `None` when no node
/// compares (none was probed in both). A node gone once settled (a split unit) does not compare; neither does a
/// registered custom property (a mask's sweep) when the mask image reading it is another once settled.
pub fn end_poses(
    ends: &BTreeMap<u64, &NodeProbe>,
    settled: &[NodeProbe],
    name: &dyn Fn(u64) -> String,
) -> Option<EndPoses> {
    let mut compared = false;
    let mut complete = true;
    let mut differences = Vec::new();
    for (node, end) in ends {
        let Some(after) = settled.iter().find(|p| p.node == *node) else {
            complete = false;
            continue;
        };
        compared = true;
        let who = name(*node);
        if !after.connected {
            // A unit split out for its build: shown at its end, it must be at rest, as its text settles.
            if !end.shown {
                complete = false;
                continue;
            }
            for (key, value) in &end.pose {
                match rest(key) {
                    Some(at_rest) if same_value(key, value, at_rest) => {}
                    Some(at_rest) => differences
                        .push(format!("{who} {key} ends at {value}, settles at {at_rest}")),
                    None => complete = false,
                }
            }
            continue;
        }
        if !after.shown {
            if end.shown {
                differences.push(format!("{who} shows at its end, hidden once settled"));
            }
            continue;
        }
        if !end.shown {
            differences.push(format!("{who} is hidden at its end, shows once settled"));
            continue;
        }
        let mask = |probe: &NodeProbe| probe.pose.get(MASK_IMAGE).cloned();
        for (key, value) in &end.pose {
            if key == MASK_IMAGE {
                continue;
            }
            if key.starts_with("--") && mask(end) != mask(after) {
                complete = false;
                continue;
            }
            if let Some(settles) = after.pose.get(key)
                && !same_value(key, value, settles)
            {
                differences.push(format!("{who} {key} ends at {value}, settles at {settles}"));
            }
        }
    }
    compared.then_some(EndPoses {
        differences,
        complete,
    })
}

/// A property's value at rest on an element that sets none: what a split unit settles to.
fn rest(key: &str) -> Option<&'static str> {
    match key {
        "opacity" => Some("1"),
        "transform" | "translate" | "rotate" | "scale" | "clipPath" | "filter" => Some("none"),
        _ => None,
    }
}

/// The pose key the page adds for a node whose custom properties a mask image reads.
pub const MASK_IMAGE: &str = "maskImage";

/// One switch of a step on one unit: from `at` (batch ms), the unit's pose `key` shows `to`.
#[derive(Clone, Debug, PartialEq)]
pub struct Timed {
    pub node: u64,
    pub key: &'static str,
    pub at: f64,
    pub to: Switched,
}

/// A step's switches on each unit it animates. A unit starts its switches of a key where its first animation of that
/// key begins (a staggered unit later), each switch at its own begin after the first. `Err` names a unit that plays
/// no animation of a key the step switches.
pub fn timed_switches(
    switches: &[Switch],
    own: &[&Animation],
    name: &dyn Fn(u64) -> String,
) -> Result<Vec<Timed>, String> {
    let mut nodes: Vec<u64> = Vec::new();
    for animation in own {
        if !nodes.contains(&animation.node) {
            nodes.push(animation.node);
        }
    }
    let mut keys: Vec<&'static str> = Vec::new();
    for switch in switches {
        if !keys.contains(&switch.key) {
            keys.push(switch.key);
        }
    }
    let mut out = Vec::new();
    let mut missing = Vec::new();
    for node in nodes {
        for key in &keys {
            let first_animation = own
                .iter()
                .filter(|a| a.node == node && a.keys.iter().any(|k| k == key))
                .map(|a| a.delay)
                .reduce(f64::min);
            let Some(first_animation) = first_animation else {
                missing.push(format!("{} plays no {key} switch", name(node)));
                continue;
            };
            let of_key = || switches.iter().filter(|s| s.key == *key);
            let first_switch = of_key().map(|s| s.begin).reduce(f64::min).unwrap_or(0.0);
            out.extend(of_key().map(|switch| Timed {
                node,
                key,
                at: first_animation + switch.begin - first_switch,
                to: switch.to.clone(),
            }));
        }
    }
    if missing.is_empty() {
        Ok(out)
    } else {
        Err(missing.join("; "))
    }
}

/// The most switch times of one step the lane pauses around; a step with more (a staggered unit per character) is
/// seen at this many, spread from its first to its last.
pub const SWITCHES_SEEN: usize = 12;

/// The switch times of a step the lane pauses around: each distinct `at`, at most [`SWITCHES_SEEN`].
pub fn seen_switches(timed: &[Timed]) -> Vec<f64> {
    use crate::motion_lane::drive::SAME_MS;
    let mut times: Vec<f64> = timed.iter().map(|t| t.at).collect();
    times.sort_by(f64::total_cmp);
    times.dedup_by(|b, a| (*b - *a).abs() <= SAME_MS);
    let n = times.len();
    if n <= SWITCHES_SEEN {
        return times;
    }
    (0..SWITCHES_SEEN)
        .filter_map(|i| times.get(i * (n - 1) / (SWITCHES_SEEN - 1)).copied())
        .collect()
}

/// Where the lane pauses a step's switches: just before ([`SWITCH_MS`](crate::motion_lane::drive::SWITCH_MS)) and
/// just after each of [`seen_switches`] (the instant before the batch stands for a switch at its start).
pub fn switch_pauses(timed: &[Timed]) -> Vec<f64> {
    use crate::motion_lane::drive::SWITCH_MS;
    seen_switches(timed)
        .into_iter()
        .flat_map(|at| [at - SWITCH_MS, at + SWITCH_MS])
        .filter(|t| *t > 0.0)
        .collect()
}

/// Whether a step's switches switch where the deck says (a set is a switch at its begin, as an appear
/// is). For each unit and key, at every pause of `samples`:
/// - from a switch's begin to the next switch of that key, the key shows the switch's `to`, and the last switch holds
///   (to the end of the batch, or until another animation of the batch on that key begins);
/// - before its first switch, the key shows what it showed at the instant before the batch, unless another animation
///   of the batch animates it before then (an earlier step leaves its own value);
/// - a pause within [`SAME_MS`](crate::motion_lane::drive::SAME_MS) of a begin is either side, and each switch the
///   lane pauses around ([`seen_switches`]) has its pause just after its begin.
///
/// `batch` is every animation of the batch, which tells the step's own switch animations from the others. `Err`
/// names the first differences.
pub fn switches(
    samples: &[Sample],
    batch: &[Animation],
    timed: &[Timed],
    name: &dyn Fn(u64) -> String,
) -> Result<(), String> {
    use crate::motion_lane::drive::{SAME_MS, SWITCH_MS};
    let Some(before) = samples.iter().find(|s| s.at_ms < 0.0) else {
        return Err("no pause before its begin".to_owned());
    };
    let mut differences = Vec::new();
    let near = |a: f64, b: f64| (a - b).abs() <= SAME_MS;
    for at in seen_switches(timed) {
        if !samples.iter().any(|s| near(s.at_ms, at + SWITCH_MS)) {
            differences.push(format!("no pause just after its switch at {at} ms"));
        }
    }
    let mut pairs: Vec<(u64, &str)> = Vec::new();
    for t in timed {
        if !pairs.contains(&(t.node, t.key)) {
            pairs.push((t.node, t.key));
        }
    }
    for (node, key) in pairs {
        let mut sequence: Vec<&Timed> = timed
            .iter()
            .filter(|t| t.node == node && t.key == key)
            .collect();
        sequence.sort_by(|a, b| a.at.total_cmp(&b.at));
        let Some(first) = sequence.first().map(|t| t.at) else {
            continue;
        };
        let others: Vec<&Animation> = batch
            .iter()
            .filter(|a| a.node == node && a.keys.iter().any(|k| k == key))
            .filter(|a| !sequence.iter().any(|t| near(a.delay, t.at)))
            .collect();
        let driven_before = others.iter().any(|a| a.delay < first - SAME_MS);
        let stop = others
            .iter()
            .map(|a| a.delay)
            .filter(|delay| *delay > first + SAME_MS)
            .fold(f64::INFINITY, f64::min);
        let who = name(node);
        for sample in samples {
            let t = sample.at_ms;
            if t >= stop - SAME_MS || sequence.iter().any(|s| near(t, s.at)) {
                continue;
            }
            let Some(value) = sample.node(node).and_then(|probe| probe.pose.get(key)) else {
                continue;
            };
            match sequence.iter().rev().find(|s| s.at < t) {
                Some(switch) => {
                    if !switch.to.shows(value) {
                        differences.push(format!(
                            "{who} {key} is {value} at {t} ms, not the {} it switched to at {} ms",
                            switch.to, switch.at
                        ));
                    }
                }
                None if driven_before => {}
                None => {
                    let Some(was) = before.node(node).and_then(|probe| probe.pose.get(key)) else {
                        continue;
                    };
                    if !same_value(key, value, was) {
                        differences.push(format!(
                            "{who} {key} is {value} at {t} ms, before its switch at {first} ms (it was {was})"
                        ));
                    }
                }
            }
        }
    }
    if differences.is_empty() {
        return Ok(());
    }
    let more = differences.len().saturating_sub(3);
    differences.truncate(3);
    if more > 0 {
        differences.push(format!("{more} more"));
    }
    Err(differences.join("; "))
}

/// The pauses of a span `start..end` ms of `samples` at [`STEP_PM`](crate::motion_lane::drive::STEP_PM).
pub fn pauses_of<'a>(
    sample: impl Fn(f64) -> Option<&'a Sample>,
    start: f64,
    end: f64,
) -> Option<Pauses<'a>> {
    use crate::motion_lane::drive::{STEP_PM, at};
    let span = end - start;
    if span <= 0.0 {
        return None;
    }
    let [s, q, m, t, e] = STEP_PM.map(|pm| sample(at(start, span, pm)));
    Some(Pauses {
        start: s?,
        mids: [q?, m?, t?],
        end: e?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_read_none_and_numbers_compare_within_a_thousandth() {
        assert!(same_value("transform", "matrix(1, 0, 0, 1, 0, 0)", "none"));
        assert!(same_value(
            "transform",
            "matrix(1, 0, 0, 1, 0.0004, 0)",
            "matrix(1, 0, 0, 1, 0, 0)"
        ));
        assert!(!same_value(
            "transform",
            "matrix(1, 0, 0, 1, 12, 0)",
            "none"
        ));
        assert!(!same_value(
            "transform",
            "matrix(0.5, 0, 0, 0.5, 0, 0)",
            "none"
        ));
        assert!(same_value("opacity", "1", "1"));
        assert!(!same_value("opacity", "0.6", "1"));
        assert!(same_value("translate", "0px", "none"));
        assert!(same_value("scale", "1", "none"));
        assert!(same_value("clipPath", "inset(0px)", "none"));
        assert!(!same_value("clipPath", "inset(0px 40% 0px 0px)", "none"));
        assert!(same_value(
            "transform",
            "matrix3d(1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1)",
            "none"
        ));
        // A perspective that is all that is left of a 3D turn.
        assert!(same_value(
            "transform",
            "matrix3d(1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, -0.00125, 0, 0, 0, 1)",
            "none"
        ));
        assert!(!same_value(
            "transform",
            "matrix3d(0.5, 0, -0.86, 0, 0, 1, 0, 0, 0.86, 0, 0.5, -0.00125, 0, 0, 0, 1)",
            "none"
        ));
        assert_eq!(
            split("rotate(-12.5deg)"),
            ("rotate(#deg)".to_owned(), vec![-12.5])
        );
    }

    #[test]
    fn a_displacement_reads_as_its_nearest_direction() {
        assert_eq!(token((-10.0, 0.4)), "l");
        assert_eq!(token((3.0, 3.0)), "rd");
        assert_eq!(token((0.1, 0.2)), "none");
        assert_eq!(along("r", Some((5.0, 0.0))), Some(Ok(())));
        assert_eq!(
            along("r", Some((-5.0, 0.0))),
            Some(Err("its content moves l".to_owned()))
        );
        // A diagonal mirrored on one axis does not move along it.
        assert!(matches!(along("lu", Some((4.0, -4.0))), Some(Err(_))));
        assert_eq!(along("x", Some((1.0, 0.0))), None);
        assert_eq!(along("r", None), None);
    }

    fn animation(node: u64, delay: f64) -> Animation {
        Animation {
            node,
            element: Some("a".into()),
            paragraph: None,
            split: false,
            layer: None,
            role: "current".into(),
            whole: false,
            keys: vec!["--sd-anim-bold".into()],
            delay,
            duration: 0.0,
            iterations: 1.0,
            direction: None,
            easing: None,
            fill: Some("forwards".into()),
            end: Some(delay),
        }
    }

    fn sample(at_ms: f64, bold: bool) -> Sample {
        Sample {
            at_ms,
            frame: slidock_bench_schema::Sha256::of(format!("{at_ms}")),
            nodes: vec![NodeProbe {
                node: 1,
                rect: [0.0, 0.0, 10.0, 10.0],
                pose: BTreeMap::from([(
                    "--sd-anim-bold".to_owned(),
                    if bold { "bold" } else { "normal" }.to_owned(),
                )]),
                shown: true,
                connected: true,
            }],
        }
    }

    fn bold_at(begin: f64) -> Switch {
        Switch {
            begin,
            key: "--sd-anim-bold",
            to: Switched::Word("bold"),
        }
    }

    #[test]
    fn a_switch_shows_its_value_from_its_begin_and_holds() {
        let name = |_: u64| "a".to_owned();
        let timed = |delay: f64| {
            let a = animation(1, delay);
            timed_switches(&[bold_at(0.0)], &[&a], &name).unwrap()
        };
        let batch = [animation(1, 0.0)];
        // A held set of no length is seen just after its begin, and holds.
        let at_once = [sample(-1.0, false), sample(1.0, true), sample(201.0, true)];
        assert_eq!(switches(&at_once, &batch, &timed(0.0), &name), Ok(()));
        // Switched 200 ms before its begin.
        let late = [animation(1, 200.0)];
        assert_eq!(
            switches(&at_once, &late, &timed(200.0), &name),
            Err(
                "a --sd-anim-bold is bold at 1 ms, before its switch at 200 ms (it was normal)"
                    .into()
            )
        );
        // Not held: back to normal after its begin.
        let dropped = [sample(-1.0, false), sample(1.0, true), sample(201.0, false)];
        assert_eq!(
            switches(&dropped, &batch, &timed(0.0), &name),
            Err("a --sd-anim-bold is normal at 201 ms, not the bold it switched to at 0 ms".into())
        );
        // Never switched.
        let never = [
            sample(-1.0, false),
            sample(1.0, false),
            sample(201.0, false),
        ];
        assert!(switches(&never, &batch, &timed(0.0), &name).is_err());
        // No pause just after its begin.
        let unseen = [sample(-1.0, false), sample(201.0, true)];
        assert_eq!(
            switches(&unseen, &batch, &timed(0.0), &name),
            Err("no pause just after its switch at 0 ms".into())
        );
    }

    #[test]
    fn a_staggered_unit_switches_from_its_own_first_animation() {
        let name = |n: u64| format!("u{n}");
        let (one, two) = (animation(1, 0.0), animation(2, 300.0));
        let switches = [bold_at(0.0), bold_at(500.0)];
        let timed = timed_switches(&switches, &[&one, &two], &name).unwrap();
        let at: Vec<(u64, f64)> = timed.iter().map(|t| (t.node, t.at)).collect();
        assert_eq!(at, [(1, 0.0), (1, 500.0), (2, 300.0), (2, 800.0)]);
        assert_eq!(
            switch_pauses(&timed),
            [1.0, 299.0, 301.0, 499.0, 501.0, 799.0, 801.0]
        );
        // A unit that plays no animation of the key the step switches.
        let mut other = animation(3, 0.0);
        other.keys = vec!["opacity".into()];
        assert_eq!(
            timed_switches(&switches, &[&one, &other], &name),
            Err("u3 plays no --sd-anim-bold switch".into())
        );
    }

    #[test]
    fn a_step_of_many_switches_is_seen_at_a_bounded_number() {
        let name = |_: u64| String::new();
        let units: Vec<Animation> = (0..100)
            .map(|i| animation(i, f64::from(i as u32) * 20.0))
            .collect();
        let own: Vec<&Animation> = units.iter().collect();
        let timed = timed_switches(&[bold_at(0.0)], &own, &name).unwrap();
        let seen = seen_switches(&timed);
        assert_eq!(seen.len(), SWITCHES_SEEN);
        assert_eq!((seen.first(), seen.last()), (Some(&0.0), Some(&1980.0)));
        assert_eq!(switch_pauses(&timed).len(), 2 * SWITCHES_SEEN - 1);
    }
}
