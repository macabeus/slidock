//! The timing values an ODF timing tree writes: SMIL clock values (`smil:dur`, `smil:begin` offsets,
//! `smil:repeatCount`) and ISO 8601 durations (`presentation:duration`, `anim:iterate-interval`). Every value is
//! untrusted: a reader returns the value, the value clamped to its limit, or `Invalid` (NaN, negative, unparsable),
//! and never guesses.

use slidock_ir::effects::limits::REPEAT_COUNT;
use slidock_ir::units::Ms;

/// The longest timing value read; anything longer is invalid.
const VALUE_BYTES: usize = 64;

/// A timing value as read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Read<T> {
    Value(T),
    /// Past its limit: the limit stands in.
    Clamped(T),
    /// NaN, negative or unparsable.
    Invalid,
}

impl<T: Copy> Read<T> {
    /// The value, clamped or not; `None` when invalid.
    pub(crate) fn value(self) -> Option<T> {
        match self {
            Read::Value(value) | Read::Clamped(value) => Some(value),
            Read::Invalid => None,
        }
    }

    /// Whether the value is not as written: clamped or invalid.
    pub(crate) fn approximated(self) -> bool {
        !matches!(self, Read::Value(_))
    }
}

/// A non-negative decimal number: digits, an optional fraction, nothing else (no sign, exponent, `NaN` or `inf`).
fn decimal(text: &str) -> Option<f64> {
    let (int, frac) = text.split_once('.').unwrap_or((text, ""));
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    if (int.is_empty() && frac.is_empty()) || !digits(int) || !digits(frac) {
        return None;
    }
    text.parse::<f64>().ok().filter(|x| x.is_finite())
}

/// Seconds as milliseconds, rounded, clamped to an hour.
fn seconds_ms(seconds: f64) -> Read<Ms> {
    let ms = slidock_canon::js_round(seconds * 1000.0);
    if !ms.is_finite() || ms < 0.0 {
        Read::Invalid
    } else if ms > f64::from(Ms::MAX) {
        Read::Clamped(Ms(Ms::MAX))
    } else {
        Read::Value(Ms(ms as u32))
    }
}

/// Whether a value is short enough to read and written without a sign: a leading `-` makes it negative.
fn bounded(text: &str) -> Option<&str> {
    let text = slidock_canon::js_trim(text);
    (text.len() <= VALUE_BYTES && !text.is_empty()).then_some(text)
}

/// A SMIL clock value (`0.5s`, `500ms`, `1.5min`, `1h`, `2` seconds, `00:01.5`, `0:00:01`) as milliseconds. An ISO
/// 8601 duration (`PT0.5S`) reads too, as some writers put one where a clock value belongs.
pub(crate) fn clock(text: &str) -> Read<Ms> {
    let Some(text) = bounded(text) else {
        return Read::Invalid;
    };
    if text.starts_with(['P', 'p', '-', '+']) && text.contains(['P', 'p']) {
        return iso_duration(text);
    }
    if text.contains(':') {
        let parts: Vec<&str> = text.split(':').collect();
        let numbers: Option<Vec<f64>> = parts.iter().map(|part| decimal(part)).collect();
        let Some(numbers) = numbers else {
            return Read::Invalid;
        };
        let seconds = match numbers.as_slice() {
            [m, s] => m * 60.0 + s,
            [h, m, s] => h * 3600.0 + m * 60.0 + s,
            _ => return Read::Invalid,
        };
        return seconds_ms(seconds);
    }
    let units: [(&str, f64); 5] = [
        ("ms", 0.001),
        ("min", 60.0),
        ("h", 3600.0),
        ("s", 1.0),
        ("", 1.0),
    ];
    for (suffix, scale) in units {
        if let Some(number) = text.strip_suffix(suffix)
            && let Some(x) = decimal(number)
        {
            return seconds_ms(x * scale);
        }
    }
    Read::Invalid
}

/// An ISO 8601 duration (`PT00H00M05S`, `PT0.05S`, `P1DT2H`) as milliseconds; a negative one is invalid.
pub(crate) fn iso_duration(text: &str) -> Read<Ms> {
    let Some(text) = bounded(text) else {
        return Read::Invalid;
    };
    let Some(rest) = text.strip_prefix('P').or_else(|| text.strip_prefix('p')) else {
        return Read::Invalid;
    };
    let (date, time) = match rest.split_once(['T', 't']) {
        Some((date, time)) if !time.is_empty() => (date, time),
        Some(_) => return Read::Invalid,
        None => (rest, ""),
    };
    let mut seconds = 0.0;
    let mut any = false;
    for (part, designators) in [
        (date, &[('D', 86_400.0)][..]),
        (time, &[('H', 3600.0), ('M', 60.0), ('S', 1.0)][..]),
    ] {
        let mut left = part;
        for &(designator, scale) in designators {
            if let Some((number, tail)) =
                left.split_once([designator, designator.to_ascii_lowercase()])
            {
                let Some(x) = decimal(number) else {
                    return Read::Invalid;
                };
                seconds += x * scale;
                any = true;
                left = tail;
            }
        }
        if !left.is_empty() {
            return Read::Invalid;
        }
    }
    if !any {
        return Read::Invalid;
    }
    seconds_ms(seconds)
}

/// A begin or end condition of a time node.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Begin<'a> {
    Offset(Read<Ms>),
    /// `next`: the next click.
    Next,
    Indefinite,
    /// `<node>.<event>` (`id3.end`, `page1.begin`, `shape2.click`), possibly with an offset.
    Event {
        node: &'a str,
        event: &'a str,
    },
}

/// A `smil:begin` value; the first of a `;`-separated list. Absent, a node begins at once.
pub(crate) fn begin(text: Option<&str>) -> Begin<'_> {
    let Some(text) = text else {
        return Begin::Offset(Read::Value(Ms(0)));
    };
    let first = slidock_canon::js_trim(text.split(';').next().unwrap_or_default());
    match first {
        "next" => Begin::Next,
        "indefinite" => Begin::Indefinite,
        _ => {
            let event_dot = first.match_indices('.').rev().find_map(|(at, _)| {
                let (node, event) = (first.get(..at)?, first.get(at + 1..)?);
                (!node.is_empty()
                    && event
                        .bytes()
                        .next()
                        .is_some_and(|b| b.is_ascii_alphabetic()))
                .then_some((node, event))
            });
            if let Some((node, event)) = event_dot {
                let event = event.split(['+', '-']).next().unwrap_or(event);
                return Begin::Event { node, event };
            }
            Begin::Offset(clock(first))
        }
    }
}

/// How often a node repeats (`smil:repeatCount`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RepeatCount {
    Times(Read<f64>),
    Indefinite,
}

/// A `smil:repeatCount`: a positive count, clamped to [`REPEAT_COUNT`], or `indefinite`.
pub(crate) fn repeat_count(text: &str) -> RepeatCount {
    let Some(text) = bounded(text) else {
        return RepeatCount::Times(Read::Invalid);
    };
    if text == "indefinite" {
        return RepeatCount::Indefinite;
    }
    RepeatCount::Times(match decimal(text) {
        Some(x) if x > f64::from(REPEAT_COUNT) => Read::Clamped(f64::from(REPEAT_COUNT)),
        Some(x) if x > 0.0 => Read::Value(x),
        _ => Read::Invalid,
    })
}

/// A fraction in [0, 1] (`smil:accelerate`, `smil:decelerate`).
pub(crate) fn fraction(text: &str) -> Option<f64> {
    decimal(bounded(text)?).filter(|x| *x <= 1.0)
}

/// A number with an optional sign (`smil:by="-360"`, `smil:to="0.5"`).
pub(crate) fn number(text: &str) -> Option<f64> {
    let text = bounded(text)?;
    let (negative, digits) = match text.strip_prefix('-') {
        Some(digits) => (true, digits),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let x = decimal(digits)?;
    Some(if negative { -x } else { x })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_values() {
        for (text, ms) in [
            ("0.5s", 500),
            ("500ms", 500),
            ("1.5min", 90_000),
            ("1h", 3_600_000),
            ("2", 2000),
            ("00:01.5", 1500),
            ("0:00:02", 2000),
            ("0.001s", 1),
            ("PT0.05S", 50),
            (" 0.75s ", 750),
        ] {
            assert_eq!(clock(text), Read::Value(Ms(ms)), "{text}");
        }
        for text in [
            "NaN", "-0.5s", "inf", "1e3s", "", "s", "0.5x", "-PT5S", "1:2:3:4", ".s",
        ] {
            assert_eq!(clock(text), Read::Invalid, "{text}");
        }
        assert_eq!(clock("4294967.295s"), Read::Clamped(Ms(Ms::MAX)));
        assert_eq!(clock("99999999999s"), Read::Clamped(Ms(Ms::MAX)));
        assert_eq!(clock(&"9".repeat(65)), Read::Invalid);
    }

    #[test]
    fn iso_durations() {
        assert_eq!(iso_duration("PT00H00M03S"), Read::Value(Ms(3000)));
        assert_eq!(iso_duration("PT0.05S"), Read::Value(Ms(50)));
        assert_eq!(iso_duration("PT1M"), Read::Value(Ms(60_000)));
        assert_eq!(iso_duration("P1D"), Read::Clamped(Ms(Ms::MAX)));
        for text in ["-PT5S", "PT", "P", "T5S", "PT5", "PTXS", "PT5S5"] {
            assert_eq!(iso_duration(text), Read::Invalid, "{text}");
        }
    }

    #[test]
    fn begin_conditions() {
        assert_eq!(begin(None), Begin::Offset(Read::Value(Ms(0))));
        assert_eq!(begin(Some("next")), Begin::Next);
        assert_eq!(begin(Some("indefinite")), Begin::Indefinite);
        assert_eq!(begin(Some("0.25s")), Begin::Offset(Read::Value(Ms(250))));
        assert_eq!(
            begin(Some("id3.end")),
            Begin::Event {
                node: "id3",
                event: "end"
            }
        );
        assert_eq!(
            begin(Some("s175-sh0.click+0.5s")),
            Begin::Event {
                node: "s175-sh0",
                event: "click"
            }
        );
        // A number with a fraction is an offset, not an event.
        assert_eq!(begin(Some("1.5")), Begin::Offset(Read::Value(Ms(1500))));
        assert_eq!(begin(Some("NaN")), Begin::Offset(Read::Invalid));
    }

    #[test]
    fn repeat_counts() {
        assert_eq!(repeat_count("3"), RepeatCount::Times(Read::Value(3.0)));
        assert_eq!(repeat_count("2.5"), RepeatCount::Times(Read::Value(2.5)));
        assert_eq!(repeat_count("indefinite"), RepeatCount::Indefinite);
        assert_eq!(
            repeat_count("4294967"),
            RepeatCount::Times(Read::Clamped(1000.0))
        );
        for text in ["0", "-1", "NaN", "x"] {
            assert_eq!(
                repeat_count(text),
                RepeatCount::Times(Read::Invalid),
                "{text}"
            );
        }
    }

    #[test]
    fn numbers_and_fractions() {
        assert_eq!(number("-360"), Some(-360.0));
        assert_eq!(number("+0.5"), Some(0.5));
        assert_eq!(number("--1"), None);
        assert_eq!(fraction("0.5"), Some(0.5));
        assert_eq!(fraction("1.5"), None);
    }
}
