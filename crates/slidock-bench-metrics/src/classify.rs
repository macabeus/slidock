//! `classifier@2`: outcome classes before numbers. A row is `failed` (the compile failed or refused the
//! presentation, the slide is missing or failed, or its documents hold code), `declined` (a decline diagnostic
//! applies; unscored), `render-error` (the renderer or the oracle failed), or scored as `match`, `close` or
//! `mismatch` from the raw (unquantised) scores. Flags (`reference`, `knownLoss`) leave the class as it is.

use core::cmp::Ordering;

use crate::color::ColorScore;
use crate::js::utf16_cmp;
use crate::text::TextScore;

pub const CLASSIFIER_VERSION: &str = "classifier@2";

macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name { $($variant),+ }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $text),+ }
            }

            pub fn parse(text: &str) -> Option<Self> {
                match text { $($text => Some($name::$variant),)+ _ => None }
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

string_enum!(
    /// A row's class, in the order the results list them.
    Outcome {
        Match = "match",
        Close = "close",
        Mismatch = "mismatch",
        Declined = "declined",
        RenderError = "render-error",
        Failed = "failed",
    }
);

string_enum!(
    FailedReason {
        CompileCrash = "compile-crash",
        CompileTimeout = "compile-timeout",
        CompileRefused = "compile-refused",
        RowMissing = "row-missing",
        SlideFailed = "slide-failed",
        Lint = "lint",
    }
);

string_enum!(
    RenderErrorReason {
        RenderTimeout = "render-timeout",
        RenderCrash = "render-crash",
        OracleMissing = "oracle-missing",
        OraclePageCount = "oracle-page-count",
        OracleFontMissing = "oracle-font-missing",
        OracleIdentity = "oracle-identity",
        CorpusHashMismatch = "corpus-hash-mismatch",
        FontSetMismatch = "font-set-mismatch",
    }
);

/// The class of a rendered and scored row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScoredClass {
    Match,
    Close,
    Mismatch,
}

impl ScoredClass {
    pub fn outcome(self) -> Outcome {
        match self {
            ScoredClass::Match => Outcome::Match,
            ScoredClass::Close => Outcome::Close,
            ScoredClass::Mismatch => Outcome::Mismatch,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SsimSummary {
    pub mean: f64,
    pub p1: f64,
}

/// Every score of a rendered row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scores {
    pub ssim: SsimSummary,
    pub color: ColorScore,
    pub pixel: f64,
    pub text: TextScore,
}

/// The `match` thresholds, calibrated once on training rows; changing one is a classifier version bump.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MatchThresholds {
    pub ssim_mean: f64,
    pub ssim_p1: f64,
    pub color_mean: f64,
    pub text_recall: f64,
    pub p95_pm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CloseThresholds {
    pub ssim_mean: f64,
    pub color_mean: f64,
    pub text_recall: f64,
    pub p95_pm: f64,
}

pub const MATCH: MatchThresholds = MatchThresholds {
    ssim_mean: 0.97,
    ssim_p1: 0.6,
    color_mean: 3.0,
    text_recall: 0.98,
    p95_pm: 2.8,
};
pub const CLOSE: CloseThresholds = CloseThresholds {
    ssim_mean: 0.9,
    color_mean: 8.0,
    text_recall: 0.9,
    p95_pm: 11.0,
};

/// A decline diagnostic that applies to a row: the slide's own, or a container's (`via`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Decline {
    pub code: String,
    pub feature: Option<String>,
    pub via: Option<String>,
}

/// How the (presentation, frontend) compile ended, when it failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompileFailure {
    Crash,
    Timeout,
    Memory,
    Refused,
}

/// What the bench knows of a row before and after rendering it. An empty code counts as no code.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RowFacts {
    pub compile: Option<CompileFailure>,
    /// The code the compiler refused the presentation with.
    pub refusal_code: Option<String>,
    /// The compiled presentation lacks the row's slide.
    pub missing: bool,
    /// The code that failed the slide (its own failure, a failed layout or master, IR0901, a lint error).
    pub failed_code: Option<String>,
    /// The bench's own lint found code that would run in the slide's documents.
    pub lint_code: Option<String>,
    /// Decline diagnostics that apply to the row (slide, shown layout and master, presentation).
    pub declines: Vec<Decline>,
    pub render_error: Option<RenderErrorReason>,
    pub scores: Option<Scores>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Classification {
    /// `memory` is the record's `data.reason: "memory"` of a compile that ran out of memory.
    Failed {
        reason: FailedReason,
        code: Option<String>,
        memory: bool,
    },
    Declined(Decline),
    RenderError(RenderErrorReason),
    Scored(ScoredClass),
}

impl Classification {
    pub fn outcome(&self) -> Outcome {
        match self {
            Classification::Failed { .. } => Outcome::Failed,
            Classification::Declined(_) => Outcome::Declined,
            Classification::RenderError(_) => Outcome::RenderError,
            Classification::Scored(class) => class.outcome(),
        }
    }
}

/// The trailing number of a code (`KN1006` → 1006), +∞ without one.
fn code_number(code: &str) -> f64 {
    let digits = code.len() - code.bytes().rev().take_while(u8::is_ascii_digit).count();
    code.get(digits..)
        .filter(|d| !d.is_empty())
        .and_then(|d| d.parse().ok())
        .unwrap_or(f64::INFINITY)
}

fn decline_order(a: &Decline, b: &Decline) -> Ordering {
    let by_number = code_number(&a.code) - code_number(&b.code);
    if by_number < 0.0 {
        return Ordering::Less;
    }
    if by_number > 0.0 {
        return Ordering::Greater;
    }
    utf16_cmp(&a.code, &b.code)
        .then(a.via.is_some().cmp(&b.via.is_some()))
        .then_with(|| {
            utf16_cmp(
                a.via.as_deref().unwrap_or(""),
                b.via.as_deref().unwrap_or(""),
            )
        })
}

/// The decline a row is classed by: the lowest code number, then the code, then the slide's own before a
/// container's, then the container.
pub fn deciding_decline(declines: &[Decline]) -> Option<&Decline> {
    declines.iter().min_by(|a, b| decline_order(a, b))
}

/// The scored class of a rendered row.
pub fn scored_class(scores: &Scores) -> ScoredClass {
    let Scores {
        ssim, color, text, ..
    } = scores;
    let placed = |limit: f64| text.oracle_words == 0 || text.p95_pm.is_some_and(|p95| p95 <= limit);
    if ssim.mean >= MATCH.ssim_mean
        && ssim.p1 >= MATCH.ssim_p1
        && color.mean <= MATCH.color_mean
        && text.recall >= MATCH.text_recall
        && placed(MATCH.p95_pm)
    {
        return ScoredClass::Match;
    }
    if ssim.mean >= CLOSE.ssim_mean
        && color.mean <= CLOSE.color_mean
        && text.recall >= CLOSE.text_recall
        && placed(CLOSE.p95_pm)
    {
        return ScoredClass::Close;
    }
    ScoredClass::Mismatch
}

fn present(code: &Option<String>) -> Option<String> {
    code.as_ref().filter(|code| !code.is_empty()).cloned()
}

/// The class a row gets before any render (failed, declined, render-error), or `None` when it is to be scored.
pub fn unscored_class(facts: &RowFacts) -> Option<Classification> {
    let failed = |reason, code| {
        Some(Classification::Failed {
            reason,
            code,
            memory: false,
        })
    };
    match facts.compile {
        Some(CompileFailure::Timeout) => return failed(FailedReason::CompileTimeout, None),
        Some(CompileFailure::Memory) => {
            return Some(Classification::Failed {
                reason: FailedReason::CompileCrash,
                code: None,
                memory: true,
            });
        }
        Some(CompileFailure::Crash) => return failed(FailedReason::CompileCrash, None),
        Some(CompileFailure::Refused) => {
            return failed(FailedReason::CompileRefused, present(&facts.refusal_code));
        }
        None => {}
    }
    if facts.missing {
        return failed(FailedReason::RowMissing, None);
    }
    if let Some(code) = present(&facts.failed_code) {
        return failed(FailedReason::SlideFailed, Some(code));
    }
    if let Some(code) = present(&facts.lint_code) {
        return failed(FailedReason::Lint, Some(code));
    }
    if let Some(decline) = deciding_decline(&facts.declines) {
        return Some(Classification::Declined(decline.clone()));
    }
    facts.render_error.map(Classification::RenderError)
}

/// Decides a row's class in the fixed order: failed, declined, render-error, scored. A renderable row without
/// scores is a `render-crash`.
pub fn classify(facts: &RowFacts) -> Classification {
    if let Some(decided) = unscored_class(facts) {
        return decided;
    }
    match &facts.scores {
        Some(scores) => Classification::Scored(scored_class(scores)),
        None => Classification::RenderError(RenderErrorReason::RenderCrash),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scores(
        mean: f64,
        p1: f64,
        color: f64,
        recall: f64,
        oracle_words: usize,
        p95: Option<f64>,
    ) -> Scores {
        Scores {
            ssim: SsimSummary { mean, p1 },
            color: ColorScore {
                mean: color,
                p99: 0.0,
            },
            pixel: 1.0,
            text: TextScore {
                oracle_words,
                dom_words: oracle_words,
                matched: 0,
                recall,
                precision: 1.0,
                median_pm: p95,
                p95_pm: p95,
                height_ratio_median: p95.map(|_| 1.0),
            },
        }
    }

    fn decline(code: &str, via: Option<&str>) -> Decline {
        Decline {
            code: code.into(),
            feature: None,
            via: via.map(Into::into),
        }
    }

    #[test]
    fn match_needs_every_threshold() {
        assert_eq!(
            scored_class(&scores(0.97, 0.6, 3.0, 0.98, 4, Some(2.8))),
            ScoredClass::Match
        );
        assert_eq!(
            scored_class(&scores(0.97, 0.6, 3.0, 0.98, 0, None)),
            ScoredClass::Match
        );
        for broken in [
            scores(0.969_999, 0.6, 3.0, 0.98, 4, Some(2.8)),
            scores(0.97, 0.599_999, 3.0, 0.98, 4, Some(2.8)),
            scores(0.97, 0.6, 3.000_001, 0.98, 4, Some(2.8)),
            scores(0.97, 0.6, 3.0, 0.979_999, 4, Some(2.8)),
            scores(0.97, 0.6, 3.0, 0.98, 4, Some(2.800_001)),
        ] {
            assert_eq!(scored_class(&broken), ScoredClass::Close, "{broken:?}");
        }
        // Words that were all missed place nothing.
        assert_eq!(
            scored_class(&scores(0.97, 0.6, 3.0, 0.98, 4, None)),
            ScoredClass::Mismatch
        );
    }

    #[test]
    fn close_needs_its_thresholds_and_ignores_p1() {
        assert_eq!(
            scored_class(&scores(0.9, -1.0, 8.0, 0.9, 4, Some(11.0))),
            ScoredClass::Close
        );
        for broken in [
            scores(0.899_999, 1.0, 0.0, 1.0, 4, Some(0.0)),
            scores(1.0, 1.0, 8.000_001, 1.0, 4, Some(0.0)),
            scores(1.0, 1.0, 0.0, 0.899_999, 4, Some(0.0)),
            scores(1.0, 1.0, 0.0, 1.0, 4, Some(11.000_001)),
            scores(1.0, 1.0, 0.0, 1.0, 4, None),
        ] {
            assert_eq!(scored_class(&broken), ScoredClass::Mismatch, "{broken:?}");
        }
    }

    #[test]
    fn nan_scores_mismatch() {
        assert_eq!(
            scored_class(&scores(f64::NAN, 1.0, 0.0, 1.0, 0, None)),
            ScoredClass::Mismatch
        );
    }

    #[test]
    fn failures_come_first_in_order() {
        let scored = RowFacts {
            scores: Some(scores(1.0, 1.0, 0.0, 1.0, 0, None)),
            ..RowFacts::default()
        };
        let all = RowFacts {
            compile: Some(CompileFailure::Timeout),
            missing: true,
            failed_code: Some("IR0901".into()),
            lint_code: Some("script".into()),
            declines: vec![decline("PP1001", None)],
            render_error: Some(RenderErrorReason::OracleMissing),
            ..scored.clone()
        };
        let reason = |facts: &RowFacts| classify(facts);
        assert_eq!(
            reason(&all),
            Classification::Failed {
                reason: FailedReason::CompileTimeout,
                code: None,
                memory: false
            }
        );
        let memory = RowFacts {
            compile: Some(CompileFailure::Memory),
            ..all.clone()
        };
        assert_eq!(
            reason(&memory),
            Classification::Failed {
                reason: FailedReason::CompileCrash,
                code: None,
                memory: true
            }
        );
        let crash = RowFacts {
            compile: Some(CompileFailure::Crash),
            ..all.clone()
        };
        assert_eq!(classify(&crash).outcome(), Outcome::Failed);
        let refused = RowFacts {
            compile: Some(CompileFailure::Refused),
            refusal_code: Some("PP0001".into()),
            ..all.clone()
        };
        assert_eq!(
            reason(&refused),
            Classification::Failed {
                reason: FailedReason::CompileRefused,
                code: Some("PP0001".into()),
                memory: false
            }
        );
        let refused_bare = RowFacts {
            refusal_code: Some(String::new()),
            ..refused
        };
        assert_eq!(
            reason(&refused_bare),
            Classification::Failed {
                reason: FailedReason::CompileRefused,
                code: None,
                memory: false
            }
        );
        let missing = RowFacts {
            compile: None,
            ..all.clone()
        };
        assert_eq!(
            reason(&missing),
            Classification::Failed {
                reason: FailedReason::RowMissing,
                code: None,
                memory: false
            }
        );
        let failed = RowFacts {
            missing: false,
            ..missing
        };
        assert_eq!(
            reason(&failed),
            Classification::Failed {
                reason: FailedReason::SlideFailed,
                code: Some("IR0901".into()),
                memory: false
            }
        );
        let lint = RowFacts {
            failed_code: Some(String::new()),
            ..failed
        };
        assert_eq!(
            reason(&lint),
            Classification::Failed {
                reason: FailedReason::Lint,
                code: Some("script".into()),
                memory: false
            }
        );
        let declined = RowFacts {
            lint_code: None,
            ..lint
        };
        assert_eq!(
            reason(&declined),
            Classification::Declined(decline("PP1001", None))
        );
        let render = RowFacts {
            declines: vec![],
            ..declined
        };
        assert_eq!(
            reason(&render),
            Classification::RenderError(RenderErrorReason::OracleMissing)
        );
        let clean = RowFacts {
            render_error: None,
            ..render
        };
        assert_eq!(unscored_class(&clean), None);
        assert_eq!(reason(&clean), Classification::Scored(ScoredClass::Match));
        let unscored = RowFacts {
            scores: None,
            ..clean
        };
        assert_eq!(
            reason(&unscored),
            Classification::RenderError(RenderErrorReason::RenderCrash)
        );
    }

    #[test]
    fn deciding_decline_order() {
        let declines = [
            decline("ODP2001", None),
            decline("KN1006", Some("layout-2")),
            decline("PP1006", Some("layout-1")),
            decline("KN1006", None),
            decline("X", None),
        ];
        // Lowest number (1006), then the code (KN before PP), then the slide's own before a container's.
        assert_eq!(deciding_decline(&declines), Some(&decline("KN1006", None)));
        assert_eq!(
            deciding_decline(&declines[1..3]),
            Some(&decline("KN1006", Some("layout-2")))
        );
        let vias = [decline("KN1006", Some("b")), decline("KN1006", Some("a"))];
        assert_eq!(deciding_decline(&vias), Some(&vias[1]));
        // Codes without a number sort after numbered ones, then by code.
        let bare = [decline("Y", None), decline("X", None), decline("Z9", None)];
        assert_eq!(deciding_decline(&bare), Some(&bare[2]));
        assert_eq!(deciding_decline(&bare[..2]), Some(&bare[1]));
        assert_eq!(deciding_decline(&[]), None);
    }

    #[test]
    fn code_numbers() {
        assert_eq!(code_number("KN1006"), 1006.0);
        assert_eq!(code_number("A007"), 7.0);
        assert_eq!(code_number("A1B"), f64::INFINITY);
        assert_eq!(code_number(""), f64::INFINITY);
    }

    #[test]
    fn names_round_trip() {
        for outcome in Outcome::ALL {
            assert_eq!(Outcome::parse(outcome.as_str()), Some(*outcome));
        }
        assert_eq!(RenderErrorReason::ALL.len(), 8);
        assert_eq!(FailedReason::parse("lint"), Some(FailedReason::Lint));
        assert_eq!(Outcome::RenderError.to_string(), "render-error");
    }
}
