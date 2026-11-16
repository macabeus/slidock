//! An effect's behaviours (ECMA-376 Part 1 §19.5: `p:set`, `p:anim`, `p:animEffect`, `p:animScale`, `p:animRot`,
//! `p:animMotion`, `p:animClr`, `p:cmd`), read into typed records: each one's common behaviour (`p:cBhvr`: its time
//! node, target and attribute names) and its own values. Nothing is evaluated here. A time, keyframe time,
//! acceleration or amount that is written but does not read is recorded as hostile and reads as absent (the
//! behaviour's default: the step's duration, evenly spread keyframes, linear, the preset's own amount).

use slidock_ir::effects::{HexColor, RepeatWritten, Retime, TimeFilter, UnfoldedRepeat};
use slidock_ir::units::Ms;

use super::values::{self, Hostile, Read, Time};
use crate::xml::{El, Element, Ns};

/// Which part of a shape a behaviour animates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    Whole,
    /// First and last paragraph, inclusive (`p:txEl/p:pRg`).
    Paragraphs(u32, u32),
    /// A character range (`p:txEl/p:charRg`), which the IR cannot target.
    Characters,
    /// The shape without its text (`p:bg`).
    Background,
    /// A part of a chart, diagram or OLE object (`p:subSp`, `p:oleChartEl`, `p:graphicEl`).
    Inside(&'static str),
}

/// What a behaviour animates (`p:tgtEl`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Shape {
        spid: String,
        part: Part,
    },
    /// The slide, a sound or ink: nothing the IR's steps target.
    Other(&'static str),
}

/// The common behaviour of every behaviour element (`p:cBhvr`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Common {
    /// Its start inside the effect (`p:cTn/p:stCondLst/p:cond@delay`), when it is a time.
    pub begin: Option<Time>,
    pub dur: Option<Time>,
    pub accel: Option<u32>,
    pub decel: Option<u32>,
    pub target: Option<Target>,
    pub attrs: Vec<String>,
    /// Whether the behaviour reverses on its own (`autoRev` on its `p:cTn`).
    pub auto_reverse: bool,
    /// Its time filter (`tmFilter`): the progress its values are read at over its time.
    pub filter: Option<TimeFilter>,
    /// Its own repeat (`repeatCount` on its `p:cTn`): a count of times, or indefinite.
    pub repeat_count: Option<RepeatWritten>,
    /// Its own repeat as a total time (`repeatDur`), in milliseconds, or indefinite.
    pub repeat_dur: Option<RepeatWritten>,
    /// Whether it keeps its value after it ends: `fill` hold, freeze or transition, or no end (no duration, or an
    /// indefinite one); `remove`, or no `fill`, takes the value back when a duration ends it.
    pub holds: bool,
}

/// A value a behaviour sets or passes through (`p:strVal`, `p:fltVal`, `p:intVal`, `p:boolVal`, `p:clrVal`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    Str(String),
    Num(f64),
    Bool(bool),
    /// An sRGB colour; `None` for a colour the frontend cannot resolve here (a scheme or system colour).
    Color(Option<HexColor>),
}

/// An `animClr` change by an amount (`p:by`): an HSL shift (hue in degrees, saturation and lightness as fractions,
/// each added), or an RGB one, which no track carries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ColorBy {
    Hsl { h: f64, s: f64, l: f64 },
    Rgb,
}

/// One keyframe of `p:anim` (`p:tav`): its time in thousandths of a percent, its value and its formula.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Tav {
    pub tm: Option<u32>,
    pub value: Option<Value>,
    pub fmla: Option<String>,
}

/// An `animScale`/`animRot` amount: `by`, `from`, `to`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Amounts<T> {
    pub by: Option<T>,
    pub from: Option<T>,
    pub to: Option<T>,
}

/// A behaviour.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Behavior {
    Set {
        common: Common,
        to: Option<Value>,
    },
    Anim {
        common: Common,
        /// `calcmode="discrete"`: each value holds until the next.
        discrete: bool,
        tavs: Vec<Tav>,
        from: Option<String>,
        to: Option<String>,
        by: Option<String>,
    },
    Effect {
        common: Common,
        /// `in`, `out` or `none`.
        transition: String,
        filter: Option<String>,
    },
    /// Scale factors (1 = the shape's size).
    Scale {
        common: Common,
        amounts: Amounts<(f64, f64)>,
    },
    /// Degrees, clockwise.
    Rotate {
        common: Common,
        amounts: Amounts<f64>,
    },
    Motion {
        common: Common,
        path: Option<String>,
    },
    Color {
        common: Common,
        from: Option<Value>,
        to: Option<Value>,
        by: Option<ColorBy>,
    },
    Command {
        common: Common,
        kind: String,
        cmd: String,
    },
    /// Any other element in a behaviour list.
    Unknown(String),
}

impl Behavior {
    pub fn common(&self) -> Option<&Common> {
        match self {
            Behavior::Set { common, .. }
            | Behavior::Anim { common, .. }
            | Behavior::Effect { common, .. }
            | Behavior::Scale { common, .. }
            | Behavior::Rotate { common, .. }
            | Behavior::Motion { common, .. }
            | Behavior::Color { common, .. }
            | Behavior::Command { common, .. } => Some(common),
            Behavior::Unknown(_) => None,
        }
    }

    /// Whether the behaviour switches the target's visibility (`p:set` of `style.visibility`).
    pub fn visibility(&self) -> Option<bool> {
        match self {
            Behavior::Set {
                common,
                to: Some(Value::Str(to)),
            } if common.attrs.iter().any(|attr| attr == "style.visibility") => {
                Some(to == "visible")
            }
            _ => None,
        }
    }

    /// What a diagnostic calls the behaviour: the attribute it animates, else its element.
    pub fn label(&self) -> String {
        if let Some(attr) = self.attr() {
            return attr.to_owned();
        }
        match self {
            Behavior::Set { .. } => "set",
            Behavior::Anim { .. } => "anim",
            Behavior::Effect { .. } => "animEffect",
            Behavior::Scale { .. } => "animScale",
            Behavior::Rotate { .. } => "animRot",
            Behavior::Motion { .. } => "animMotion",
            Behavior::Color { .. } => "animClr",
            Behavior::Command { .. } => "cmd",
            Behavior::Unknown(name) => name,
        }
        .to_owned()
    }

    /// The attribute the behaviour animates, its first `p:attrName`.
    pub fn attr(&self) -> Option<&str> {
        self.common()
            .and_then(|common| common.attrs.first())
            .map(String::as_str)
    }
}

fn target(tgt: Option<&Element>) -> Option<Target> {
    let tgt = tgt?;
    if let Some(sp) = tgt.child(Ns::P, "spTgt") {
        let spid = sp.attr("spid").unwrap_or("").to_owned();
        let part = if sp.child(Ns::P, "bg").is_some() {
            Part::Background
        } else if let Some(tx) = sp.child(Ns::P, "txEl") {
            match tx.child(Ns::P, "pRg") {
                Some(range) => {
                    let index = |name| {
                        range
                            .attr(name)
                            .and_then(|text| text.trim().parse::<u32>().ok())
                    };
                    match (index("st"), index("end")) {
                        (Some(st), Some(end)) => Part::Paragraphs(st, end),
                        _ => Part::Whole,
                    }
                }
                None if tx.child(Ns::P, "charRg").is_some() => Part::Characters,
                None => Part::Whole,
            }
        } else if sp.child(Ns::P, "subSp").is_some() {
            Part::Inside("subSp")
        } else if sp.child(Ns::P, "oleChartEl").is_some() {
            Part::Inside("oleChartEl")
        } else if sp.child(Ns::P, "graphicEl").is_some() {
            Part::Inside("graphicEl")
        } else {
            Part::Whole
        };
        return Some(Target::Shape { spid, part });
    }
    let kind = tgt.kids().next().map(|kid| &*kid.local);
    Some(Target::Other(match kind {
        Some("sldTgt") => "slide",
        Some("sndTgt") => "sound",
        Some("inkTgt") => "ink",
        _ => "unknown",
    }))
}

/// The time a `p:cond@delay` of a start-condition list gives, when it is a plain delay.
fn begin(ctn: Option<&Element>, hostile: &mut Vec<Hostile>) -> Option<Time> {
    let cond = ctn
        .child(Ns::P, "stCondLst")
        .children(Ns::P, "cond")
        .into_iter()
        .find(|cond| cond.kids().next().is_none())?;
    values::time("delayMs", cond.attr("delay"), hostile)
        .ok()
        .flatten()
}

/// Records a value that is written but does not read.
fn invalid(field: &'static str, text: &str, hostile: &mut Vec<Hostile>) {
    hostile.push(Hostile {
        field,
        native: text.to_owned(),
        read: Read::Invalid,
        limit: None,
    });
}

/// A percentage in thousandths (`accel`, `decel`, `tm`): absent, or recorded and absent when it does not read.
fn fraction(field: &'static str, text: Option<&str>, hostile: &mut Vec<Hostile>) -> Option<u32> {
    match values::fraction(text) {
        Ok(value) => value,
        Err(()) => {
            invalid(field, text.unwrap_or(""), hostile);
            None
        }
    }
}

fn common(element: &Element, hostile: &mut Vec<Hostile>) -> Common {
    let c_bhvr = element.child(Ns::P, "cBhvr").map(|rc| &**rc);
    let ctn = c_bhvr.child(Ns::P, "cTn");
    let dur = values::time("durationMs", ctn.attr("dur"), hostile)
        .ok()
        .flatten();
    Common {
        begin: begin(ctn, hostile),
        dur,
        accel: fraction("easing", ctn.attr("accel"), hostile),
        decel: fraction("easing", ctn.attr("decel"), hostile),
        target: target(c_bhvr.child(Ns::P, "tgtEl")),
        attrs: c_bhvr
            .child(Ns::P, "attrNameLst")
            .children(Ns::P, "attrName")
            .into_iter()
            .map(|name| name.text_content().trim().to_owned())
            .collect(),
        auto_reverse: ctn.bool_attr("autoRev") == Some(true),
        filter: ctn.attr("tmFilter").and_then(|text| {
            let filter = TimeFilter::parse(text);
            if filter.is_none() {
                invalid("timeFilter", text, hostile);
            }
            filter
        }),
        holds: matches!(ctn.attr("fill"), Some("hold" | "freeze" | "transition"))
            || !matches!(dur, Some(Time::Ms(_))),
        repeat_count: values::repeat_count(ctn.attr("repeatCount"), hostile).map(
            |count| match count {
                values::RepeatCount::Indefinite => RepeatWritten::Indefinite,
                values::RepeatCount::Thousandths(n) => RepeatWritten::Value(n as f64 / 1_000.0),
            },
        ),
        repeat_dur: values::time("repeatDur", ctn.attr("repeatDur"), hostile)
            .ok()
            .flatten()
            .map(|time| match time {
                Time::Indefinite => RepeatWritten::Indefinite,
                Time::Ms(ms) => RepeatWritten::Value(f64::from(ms.0)),
            }),
    }
}

impl Common {
    /// Its own time manipulations over a span of `duration` (its reversal, time filter and repeat), and whether a
    /// repeat it writes does not fold (an indefinite one, or a total time over no span): it then plays once.
    pub fn retime(&self, duration: Ms) -> (Retime, bool) {
        let (repeat, unfolded) = match Retime::repeat_of(
            self.repeat_count,
            self.repeat_dur,
            duration,
            self.auto_reverse,
        ) {
            Ok(repeat) => (repeat, false),
            Err(UnfoldedRepeat) => (None, true),
        };
        let retime = Retime {
            auto_reverse: self.auto_reverse,
            filter: self.filter.clone(),
            repeat,
        };
        (retime, unfolded)
    }
}

fn number(text: Option<&str>) -> Option<f64> {
    let value: f64 = text?.trim().parse().ok()?;
    value.is_finite().then_some(value)
}

/// A colour element (`a:srgbClr`, …) as sRGB, when it is one.
fn color(parent: Option<&Element>) -> Option<Value> {
    let kid = parent?.kids().next()?;
    if kid.is(Ns::A, "srgbClr") {
        let hex = kid
            .attr("val")
            .and_then(|val| HexColor::parse(&format!("#{val}")).ok());
        return Some(Value::Color(hex));
    }
    Some(Value::Color(None))
}

/// An `animClr` amount (`p:by`): `p:hsl` (`h` in 60000ths of a degree, `s` and `l` in thousandths of a percent) or
/// `p:rgb`. A part that does not read makes the whole amount absent, recorded as hostile.
fn color_by(by: Option<&Element>, hostile: &mut Vec<Hostile>) -> Option<ColorBy> {
    let kid = by?.kids().next()?;
    if kid.is(Ns::P, "rgb") {
        return Some(ColorBy::Rgb);
    }
    if !kid.is(Ns::P, "hsl") {
        return None;
    }
    let part = |name: &str, hostile: &mut Vec<Hostile>| {
        amount(Some(kid.attr(name).unwrap_or("0")), hostile)
    };
    let h = part("h", hostile)? / 60_000.0;
    let s = part("s", hostile)? / 100_000.0;
    let l = part("l", hostile)? / 100_000.0;
    Some(ColorBy::Hsl { h, s, l })
}

/// The value inside `p:to`, `p:val` and the like.
fn value(parent: Option<&Element>) -> Option<Value> {
    let kid = parent?.kids().next()?;
    match &*kid.local {
        "strVal" => Some(Value::Str(kid.attr("val").unwrap_or("").to_owned())),
        "fltVal" | "intVal" => number(kid.attr("val")).map(Value::Num),
        "boolVal" => kid.bool_attr("val").map(Value::Bool),
        "clrVal" => color(Some(kid)),
        _ => None,
    }
}

/// A number an amount attribute gives: absent, or recorded and absent when it does not read.
fn amount(text: Option<&str>, hostile: &mut Vec<Hostile>) -> Option<f64> {
    let text = text?;
    let value = number(Some(text));
    if value.is_none() {
        invalid("amount", text, hostile);
    }
    value
}

/// An `animScale` point (`x`/`y` in thousandths of a percent) as scale factors.
fn point(element: Option<&Element>, hostile: &mut Vec<Hostile>) -> Option<(f64, f64)> {
    let element = element?;
    let x = amount(Some(element.attr("x").unwrap_or("")), hostile)?;
    let y = amount(Some(element.attr("y").unwrap_or("")), hostile)?;
    Some((x / 100_000.0, y / 100_000.0))
}

/// An `animRot` angle (60000ths of a degree) in degrees.
fn angle(text: Option<&str>, hostile: &mut Vec<Hostile>) -> Option<f64> {
    amount(text, hostile).map(|value| value / 60_000.0)
}

/// Reads one behaviour element; a value that does not read is recorded in `hostile`.
pub(crate) fn read(element: &Element, hostile: &mut Vec<Hostile>) -> Behavior {
    if element.ns != Ns::P {
        return Behavior::Unknown(element.name.to_string());
    }
    match &*element.local {
        "set" => Behavior::Set {
            common: common(element, hostile),
            to: value(element.child(Ns::P, "to").map(|rc| &**rc)),
        },
        "anim" => Behavior::Anim {
            common: common(element, hostile),
            discrete: element.attr("calcmode") == Some("discrete"),
            tavs: element
                .child(Ns::P, "tavLst")
                .children(Ns::P, "tav")
                .into_iter()
                .map(|tav| Tav {
                    tm: fraction("keyframe", tav.attr("tm"), hostile),
                    value: value(Some(tav).child(Ns::P, "val")),
                    fmla: tav.attr("fmla").map(str::to_owned),
                })
                .collect(),
            from: element.attr("from").map(str::to_owned),
            to: element.attr("to").map(str::to_owned),
            by: element.attr("by").map(str::to_owned),
        },
        "animEffect" => Behavior::Effect {
            common: common(element, hostile),
            transition: element.attr("transition").unwrap_or("in").to_owned(),
            filter: element.attr("filter").map(str::to_owned),
        },
        "animScale" => Behavior::Scale {
            common: common(element, hostile),
            amounts: Amounts {
                by: point(element.child(Ns::P, "by").map(|rc| &**rc), hostile),
                from: point(element.child(Ns::P, "from").map(|rc| &**rc), hostile),
                to: point(element.child(Ns::P, "to").map(|rc| &**rc), hostile),
            },
        },
        "animRot" => Behavior::Rotate {
            common: common(element, hostile),
            amounts: Amounts {
                by: angle(element.attr("by"), hostile),
                from: angle(element.attr("from"), hostile),
                to: angle(element.attr("to"), hostile),
            },
        },
        "animMotion" => Behavior::Motion {
            common: common(element, hostile),
            path: element.attr("path").map(str::to_owned),
        },
        "animClr" => Behavior::Color {
            common: common(element, hostile),
            from: color(element.child(Ns::P, "from").map(|rc| &**rc)),
            to: color(element.child(Ns::P, "to").map(|rc| &**rc)),
            by: color_by(element.child(Ns::P, "by").map(|rc| &**rc), hostile),
        },
        "cmd" => Behavior::Command {
            common: common(element, hostile),
            kind: element.attr("type").unwrap_or("").to_owned(),
            cmd: element.attr("cmd").unwrap_or("").to_owned(),
        },
        other => Behavior::Unknown(other.to_owned()),
    }
}

/// A `p:set` of `style.visibility`, `fillcolor` or another attribute within a step that lasts only to switch it
/// (PowerPoint writes such switches with a 1 ms duration).
pub(crate) fn is_switch(behavior: &Behavior) -> bool {
    matches!(behavior, Behavior::Set { common, .. } if matches!(common.dur, Some(Time::Ms(ms)) if ms.0 <= 1))
}
