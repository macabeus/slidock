//! Timing values as the time-node tree writes them (ECMA-376 Part 1 §19.7.38 `ST_TLTime`, §19.7.39
//! `ST_TLTimeRepeatCount`…), read bounded: a value past its limit clamps to it, a value that is not one (`NaN`, a
//! negative number, a fraction where an integer belongs) is invalid. Both are recorded, so the step is reported and
//! plays with the clamped value or the declared default, never a coerced one.

use slidock_ir::effects::limits::REPEAT_COUNT;
use slidock_ir::units::Ms;

/// How a value read: as written, clamped to its limit, or not at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Read {
    Clamped,
    Invalid,
}

impl Read {
    pub fn token(self) -> &'static str {
        match self {
            Read::Clamped => "clamped",
            Read::Invalid => "invalid",
        }
    }
}

/// A value that did not read as written: which field, the value as written (in the field's own units), how it read
/// and the limit it clamped to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Hostile {
    pub field: &'static str,
    pub native: String,
    pub read: Read,
    pub limit: Option<&'static str>,
}

/// The limit a clamped duration or delay names.
pub(crate) const MS_LIMIT: &str = "slidock_ir::units::Ms::MAX";
/// The limit a clamped repeat count names.
pub(crate) const REPEAT_LIMIT: &str = "slidock_ir::effects::limits::REPEAT_COUNT";

/// An `ST_TLTime` value: milliseconds or `indefinite`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Time {
    Ms(Ms),
    Indefinite,
}

/// The decimal digits of an unsigned integer, surrounding whitespace allowed; `None` for anything else.
fn unsigned(text: &str) -> Option<&str> {
    let digits = text.trim_matches(|c: char| c.is_ascii_whitespace());
    let digits = digits.strip_prefix('+').unwrap_or(digits);
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then_some(digits)
}

/// The value of a digit string, saturating at `u64::MAX`.
fn saturating(digits: &str) -> u64 {
    digits.bytes().fold(0u64, |value, digit| {
        value
            .saturating_mul(10)
            .saturating_add(u64::from(digit - b'0'))
    })
}

/// Reads an `ST_TLTime`: `Ok(None)` when absent, the time (clamped to an hour) otherwise; a value that is not one is
/// `Err` with what it was.
pub(crate) fn time(
    field: &'static str,
    text: Option<&str>,
    hostile: &mut Vec<Hostile>,
) -> Result<Option<Time>, ()> {
    let Some(text) = text else {
        return Ok(None);
    };
    if text.trim_matches(|c: char| c.is_ascii_whitespace()) == "indefinite" {
        return Ok(Some(Time::Indefinite));
    }
    let Some(digits) = unsigned(text) else {
        hostile.push(Hostile {
            field,
            native: text.to_owned(),
            read: Read::Invalid,
            limit: None,
        });
        return Err(());
    };
    let value = saturating(digits);
    if value > u64::from(Ms::MAX) {
        hostile.push(Hostile {
            field,
            native: digits.to_owned(),
            read: Read::Clamped,
            limit: Some(MS_LIMIT),
        });
        return Ok(Some(Time::Ms(Ms(Ms::MAX))));
    }
    Ok(Some(Time::Ms(Ms(u32::try_from(value).unwrap_or(Ms::MAX)))))
}

/// A repeat count as written: `indefinite`, or a number of thousandths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RepeatCount {
    Indefinite,
    /// Thousandths of an iteration, at least 1 and at most [`REPEAT_COUNT`] iterations.
    Thousandths(u64),
}

/// Reads `repeatCount` (`ST_TLTime`, thousandths of an iteration): a count past [`REPEAT_COUNT`] iterations clamps,
/// zero or a value that is not a count is invalid.
pub(crate) fn repeat_count(text: Option<&str>, hostile: &mut Vec<Hostile>) -> Option<RepeatCount> {
    let text = text?;
    if text.trim_matches(|c: char| c.is_ascii_whitespace()) == "indefinite" {
        return Some(RepeatCount::Indefinite);
    }
    let invalid = |hostile: &mut Vec<Hostile>| {
        hostile.push(Hostile {
            field: "repeat",
            native: text.to_owned(),
            read: Read::Invalid,
            limit: None,
        });
        None
    };
    let Some(digits) = unsigned(text) else {
        return invalid(hostile);
    };
    let value = saturating(digits);
    if value == 0 {
        return invalid(hostile);
    }
    let limit = u64::from(REPEAT_COUNT) * 1_000;
    if value > limit {
        hostile.push(Hostile {
            field: "repeat",
            native: thousandths(value),
            read: Read::Clamped,
            limit: Some(REPEAT_LIMIT),
        });
        return Some(RepeatCount::Thousandths(limit));
    }
    Some(RepeatCount::Thousandths(value))
}

/// A number of thousandths as a decimal (`4294967000` → `4294967`, `2500` → `2.5`).
pub(crate) fn thousandths(value: u64) -> String {
    let (whole, fraction) = (value / 1_000, value % 1_000);
    if fraction == 0 {
        return whole.to_string();
    }
    let fraction = format!("{fraction:03}");
    format!("{whole}.{}", fraction.trim_end_matches('0'))
}

/// Reads a percentage in thousandths (`ST_PositiveFixedPercentage`, `accel`, `tmPct`: 0 to 100000): `None` when
/// absent, `Err` for a value out of range or not one.
pub(crate) fn fraction(text: Option<&str>) -> Result<Option<u32>, ()> {
    let Some(text) = text else {
        return Ok(None);
    };
    let digits = unsigned(text).ok_or(())?;
    let value = saturating(digits);
    if value > 100_000 {
        return Err(());
    }
    u32::try_from(value).map(Some).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_clamp_or_fail_closed() {
        let mut hostile = Vec::new();
        assert_eq!(time("delayMs", None, &mut hostile), Ok(None));
        assert_eq!(
            time("delayMs", Some(" 250 "), &mut hostile),
            Ok(Some(Time::Ms(Ms(250))))
        );
        assert_eq!(
            time("delayMs", Some("indefinite"), &mut hostile),
            Ok(Some(Time::Indefinite))
        );
        assert_eq!(
            time("delayMs", Some("3600000"), &mut hostile),
            Ok(Some(Time::Ms(Ms(Ms::MAX))))
        );
        assert!(hostile.is_empty());
        assert_eq!(
            time("delayMs", Some("99999999999999999999999"), &mut hostile),
            Ok(Some(Time::Ms(Ms(Ms::MAX))))
        );
        assert_eq!(
            time("delayMs", Some("3600001"), &mut hostile),
            Ok(Some(Time::Ms(Ms(Ms::MAX))))
        );
        for bad in ["NaN", "-500", "1.5", "", "1e3", "0x10", "-0"] {
            assert_eq!(
                time("durationMs", Some(bad), &mut hostile),
                Err(()),
                "{bad}"
            );
        }
        assert_eq!(hostile.len(), 9);
        assert_eq!(
            hostile[0],
            Hostile {
                field: "delayMs",
                native: "99999999999999999999999".into(),
                read: Read::Clamped,
                limit: Some(MS_LIMIT),
            }
        );
        assert_eq!(hostile[1].native, "3600001");
        assert_eq!(hostile[1].read, Read::Clamped);
        assert_eq!(hostile[2].native, "NaN");
        assert_eq!(hostile[2].read, Read::Invalid);
    }

    #[test]
    fn repeat_counts_are_thousandths() {
        let mut hostile = Vec::new();
        assert_eq!(
            repeat_count(Some("3000"), &mut hostile),
            Some(RepeatCount::Thousandths(3_000))
        );
        assert_eq!(
            repeat_count(Some("indefinite"), &mut hostile),
            Some(RepeatCount::Indefinite)
        );
        assert!(hostile.is_empty());
        assert_eq!(
            repeat_count(Some("4294967000"), &mut hostile),
            Some(RepeatCount::Thousandths(1_000_000))
        );
        assert_eq!(hostile[0].native, "4294967");
        assert_eq!(hostile[0].limit, Some(REPEAT_LIMIT));
        assert_eq!(repeat_count(Some("0"), &mut hostile), None);
        assert_eq!(repeat_count(Some("-1"), &mut hostile), None);
        assert_eq!(hostile.len(), 3);
        assert_eq!(thousandths(2_500), "2.5");
        assert_eq!(thousandths(1_005), "1.005");
    }

    #[test]
    fn fractions_stay_in_range() {
        assert_eq!(fraction(None), Ok(None));
        assert_eq!(fraction(Some("50000")), Ok(Some(50_000)));
        assert_eq!(fraction(Some("100000")), Ok(Some(100_000)));
        assert_eq!(fraction(Some("100001")), Err(()));
        assert_eq!(fraction(Some("-1")), Err(()));
    }
}
