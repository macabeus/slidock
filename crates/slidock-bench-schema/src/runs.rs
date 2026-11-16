//! `runs.json` (`slidock-bench-runs@1`): the index of the runs in the cache, each with its key, identity and
//! summary only, so the views list every run without loading their results.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::de::optional;
use crate::error::{Error, Issues};
use crate::ids::{Lane, Sha256};
use crate::literal::{True, literal};
use crate::results::Summary;
use crate::stamp::CompilerStamp;

literal!(
    /// The runs index format.
    RunsSchema = "slidock-bench-runs@1"
);

/// One run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunEntry {
    /// The run key (`Stamp::run_key`): its folder under `runs/`.
    pub key: Sha256,
    /// The sha256 of the run's whole stamp.
    pub stamp: Sha256,
    pub lane: Lane,
    pub compiler: CompilerStamp,
    pub harness: Sha256,
    pub partial: bool,
    /// When the run finished, as RFC 3339 UTC (`2026-10-30T12:00:00Z`). Only the index
    /// holds a time; results files hold none.
    pub recorded_at: String,
    /// The run wrote the lane's committed baseline.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<True>", optional)]
    pub baseline: Option<True>,
    pub summary: Summary,
}

/// The runs index, oldest first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RunsIndex {
    pub schema: RunsSchema,
    pub runs: Vec<RunEntry>,
}

/// An RFC 3339 UTC time, ordered by the instant it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct UtcTime<'a> {
    /// `YYYY-MM-DDTHH:MM:SS`: fixed width, so text order is time order.
    seconds: &'a str,
    /// The fraction's digits without trailing zeros, so text order is value order.
    fraction: &'a str,
}

impl UtcTime<'_> {
    fn parse(text: &str) -> Option<UtcTime<'_>> {
        if !is_utc_timestamp(text) {
            return None;
        }
        let seconds = text.get(..19)?;
        let rest = text.get(19..)?.trim_end_matches('Z');
        let fraction = rest.strip_prefix('.').unwrap_or(rest).trim_end_matches('0');
        Some(UtcTime { seconds, fraction })
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ`, with an optional fraction of a second.
fn is_utc_timestamp(text: &str) -> bool {
    let bytes = text.as_bytes();
    let digits = |range: std::ops::Range<usize>| {
        bytes
            .get(range)
            .is_some_and(|part| part.iter().all(u8::is_ascii_digit))
    };
    let at = |i: usize, c: u8| bytes.get(i) == Some(&c);
    let shape = digits(0..4)
        && at(4, b'-')
        && digits(5..7)
        && at(7, b'-')
        && digits(8..10)
        && at(10, b'T')
        && digits(11..13)
        && at(13, b':')
        && digits(14..16)
        && at(16, b':')
        && digits(17..19);
    let rest = text.get(19..).unwrap_or("");
    let fraction_ok = match rest.strip_suffix('Z') {
        Some("") => true,
        Some(fraction) => fraction
            .strip_prefix('.')
            .is_some_and(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit())),
        None => false,
    };
    shape && fraction_ok
}

impl RunsIndex {
    pub fn parse(text: &str) -> Result<RunsIndex, Error> {
        let index: RunsIndex = crate::parse(text)?;
        index.validate()?;
        Ok(index)
    }

    pub fn to_canonical(&self) -> Result<String, Error> {
        self.validate()?;
        crate::to_canonical(self)
    }

    /// Each run once, oldest first, with a UTC timestamp.
    pub fn validate(&self) -> Result<(), Error> {
        let mut issues = Issues::default();
        let mut keys = BTreeSet::new();
        for run in &self.runs {
            if !keys.insert(&run.key) {
                issues.push(format!("run {} is listed twice", run.key));
            }
            if !is_utc_timestamp(&run.recorded_at) {
                issues.push(format!(
                    "run {}: recordedAt {:?} is not an RFC 3339 UTC time",
                    run.key, run.recorded_at
                ));
            }
        }
        let times: Vec<Option<UtcTime<'_>>> = self
            .runs
            .iter()
            .map(|run| UtcTime::parse(&run.recorded_at))
            .collect();
        if times
            .windows(2)
            .any(|pair| matches!(pair, [Some(a), Some(b)] if a > b))
        {
            issues.push("runs are listed oldest first");
        }
        issues.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_are_utc() {
        for ok in [
            "2026-10-30T12:00:00Z",
            "2026-10-30T12:00:00.5Z",
            "2026-10-30T12:00:00.123Z",
        ] {
            assert!(is_utc_timestamp(ok), "{ok}");
        }
        for bad in [
            "2026-10-30",
            "2026-10-30T12:00:00",
            "2026-10-30T12:00:00+01:00",
            "2026-10-30T12:00:00.Z",
            "2026-10-30 12:00:00Z",
            "2026-1a-30T12:00:00Z",
        ] {
            assert!(!is_utc_timestamp(bad), "{bad}");
        }
    }

    #[test]
    fn timestamps_order_by_the_instant() {
        let at = |t| UtcTime::parse(t).unwrap();
        let ordered = [
            "2026-10-30T12:00:50Z",
            "2026-10-30T12:00:50.05Z",
            "2026-10-30T12:00:50.123Z",
            "2026-10-30T12:00:50.5Z",
            "2026-10-30T12:00:51Z",
            "2026-10-31T00:00:00Z",
        ];
        for pair in ordered.windows(2) {
            if let [a, b] = pair {
                assert!(at(a) < at(b), "{a} {b}");
            }
        }
        assert_eq!(at("2026-10-30T12:00:50Z"), at("2026-10-30T12:00:50.000Z"));
        assert_eq!(at("2026-10-30T12:00:50.5Z"), at("2026-10-30T12:00:50.50Z"));
        assert!(UtcTime::parse("2026-10-30T12:00:50").is_none());
    }
}
