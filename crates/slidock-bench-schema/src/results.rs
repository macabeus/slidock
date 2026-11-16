//! `results.json` (`slidock-bench-results@1`): the
//! stamp, one record per row, and the summaries. Canonical JSON, rows keyed by id, numbers on a 1e-4 grid, free of
//! timestamps and paths, so equal runs write equal bytes. Each lane writes its own file (`results.json` for
//! `static`, `motion-results.json`, `fonts-results.json`, …), named by `stamp.lane`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::corpus::LossKind;
use crate::de::{nullable, optional};
use crate::error::{Error, Issues};
use crate::ids::{Frontend, Lane, PairKey, PresentationId, RowId, RowStep, Sha256, SlideId};
use crate::lanes::{
    FontsPairReport, FontsRowMetrics, HostCheck, LaneSummaries, MotionPairReport, MotionRowMetrics,
    ShellCheck, ShellPairCheck, TimelineFidelity,
};
use crate::literal::{True, literal};
use crate::stamp::Stamp;

literal!(
    /// The results format.
    ResultsSchema = "slidock-bench-results@1"
);

/// `classifier@2`'s classes, outcome before numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Match,
    Close,
    Mismatch,
    /// A decline diagnostic applies; unscored.
    Declined,
    /// The renderer or the oracle failed.
    RenderError,
    /// The compile failed or refused, the slide is missing or failed, or its documents hold code.
    Failed,
}

impl Outcome {
    pub const ALL: [Outcome; 6] = [
        Outcome::Match,
        Outcome::Close,
        Outcome::Mismatch,
        Outcome::Declined,
        Outcome::RenderError,
        Outcome::Failed,
    ];

    /// The movement rank: failed 0 < render-error 1 < declined 2 < mismatch 3 < close 4 < match 5.
    pub const fn rank(self) -> u8 {
        match self {
            Outcome::Failed => 0,
            Outcome::RenderError => 1,
            Outcome::Declined => 2,
            Outcome::Mismatch => 3,
            Outcome::Close => 4,
            Outcome::Match => 5,
        }
    }

    pub const fn is_scored(self) -> bool {
        matches!(self, Outcome::Match | Outcome::Close | Outcome::Mismatch)
    }
}

/// The classes of the motion lane's transition rows, and of its stage rows where no stages oracle scores them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum MotionOutcome {
    MotionMatch,
    MotionClose,
    MotionMismatch,
    /// A fallback effect played and matched the fallback's model.
    MotionFallback,
}

/// The classes of fonts-lane rows (`fonts@1`) that rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum FontsOutcome {
    /// Every glyph was drawn by a face the deck ships.
    FontsComplete,
    /// The renderer's own fonts drew some glyphs.
    FontsFallback,
}

/// A row's class: `classifier@2`'s, a motion class for transition rows, or a fonts class for fonts-lane rows that
/// rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(untagged)]
pub enum RowOutcome {
    Classifier(Outcome),
    Motion(MotionOutcome),
    Fonts(FontsOutcome),
}

impl RowOutcome {
    pub const fn classifier(self) -> Option<Outcome> {
        match self {
            RowOutcome::Classifier(outcome) => Some(outcome),
            RowOutcome::Motion(_) | RowOutcome::Fonts(_) => None,
        }
    }
}

impl From<FontsOutcome> for RowOutcome {
    fn from(outcome: FontsOutcome) -> RowOutcome {
        RowOutcome::Fonts(outcome)
    }
}

impl From<Outcome> for RowOutcome {
    fn from(outcome: Outcome) -> RowOutcome {
        RowOutcome::Classifier(outcome)
    }
}

impl From<MotionOutcome> for RowOutcome {
    fn from(outcome: MotionOutcome) -> RowOutcome {
        RowOutcome::Motion(outcome)
    }
}

/// Why a row failed or hit a render error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum Reason {
    CompileCrash,
    CompileTimeout,
    CompileRefused,
    RowMissing,
    SlideFailed,
    Lint,
    RenderTimeout,
    RenderCrash,
    OracleMissing,
    OraclePageCount,
    OracleFontMissing,
    OracleIdentity,
    CorpusHashMismatch,
    FontSetMismatch,
}

impl Reason {
    /// The reasons of `failed` rows; the others are `render-error`'s.
    pub const fn is_failed(self) -> bool {
        matches!(
            self,
            Reason::CompileCrash
                | Reason::CompileTimeout
                | Reason::CompileRefused
                | Reason::RowMissing
                | Reason::SlideFailed
                | Reason::Lint
        )
    }
}

/// The decline a row is classed by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Decline {
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub feature: Option<String>,
    /// The container (layout, master) or `presentation` the diagnostic comes through; absent for the slide's own.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub via: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SsimScore {
    pub mean: f64,
    pub p1: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ColorScore {
    /// Mean CIEDE2000 over 8×8 tiles.
    pub mean: f64,
    pub p99: f64,
}

/// `text@2`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextScore {
    pub oracle_words: u32,
    pub dom_words: u32,
    pub matched: u32,
    pub recall: f64,
    pub precision: f64,
    /// Median centre distance of matched words, in per-mille of the slide's long side.
    #[serde(with = "nullable")]
    #[ts(as = "Option<f64>")]
    pub median_pm: Option<f64>,
    #[serde(with = "nullable")]
    #[ts(as = "Option<f64>")]
    pub p95_pm: Option<f64>,
    #[serde(with = "nullable")]
    #[ts(as = "Option<f64>")]
    pub height_ratio_median: Option<f64>,
}

/// A row's scores, quantised to the 1e-4 grid.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Scores {
    pub ssim: SsimScore,
    pub color: ColorScore,
    /// `pixel@1`: reported only.
    pub pixel: f64,
    pub text: TextScore,
}

/// `shadow@1`: the rings around a row's shadowed elements, quantised.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShadowMetrics {
    /// The rings measured: elements with a shadow in the IR or named by a shadow-not-drawn diagnostic.
    pub elements: u32,
    /// The largest ring RMSE (RGB, 0 to 1) of render against oracle.
    pub worst_rmse: f64,
    pub mean_rmse: f64,
    /// The element of the worst ring.
    pub worst_el: String,
}

/// `gradient@1`: the regions of a row's gradient elements, quantised.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GradientMetrics {
    /// The regions measured: elements (and the slide background, `bg`) with a gradient fill in the IR or named by a
    /// gradient-approximated diagnostic.
    pub elements: u32,
    /// The largest region RMSE (RGB, 0 to 1) of render against oracle.
    pub worst_rmse: f64,
    pub mean_rmse: f64,
    /// The element of the worst region.
    pub worst_el: String,
}

/// The oracle page a row is scored against.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct OraclePage {
    /// 1-based.
    pub page: u32,
}

/// One row's record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RowRecord {
    pub presentation: PresentationId,
    pub slide: SlideId,
    pub frontend: Frontend,
    pub oracle: OraclePage,
    pub outcome: RowOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Reason>", optional)]
    pub reason: Option<Reason>,
    /// The code that failed the slide or refused the compile.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<BTreeMap<String, String>>", optional)]
    pub data: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Decline>", optional)]
    pub declined: Option<Decline>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Scores>", optional)]
    pub scores: Option<Scores>,
    /// `shadow@1`, on scored rows that show a shadowed element.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<ShadowMetrics>", optional)]
    pub shadow: Option<ShadowMetrics>,
    /// `gradient@1`, on scored rows that show a gradient.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<GradientMetrics>", optional)]
    pub gradient: Option<GradientMetrics>,
    /// Counts of the diagnostics that apply to the row (its slide, shown layout and master, and the presentation).
    pub diagnostics: BTreeMap<String, u32>,
    /// sha256 of the composed document.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Sha256>", optional)]
    pub html: Option<Sha256>,
    /// sha256 of the render.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Sha256>", optional)]
    pub render: Option<Sha256>,
    /// The compiler content id the row was produced with (an overlay may mix them).
    pub content: Sha256,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<True>", optional)]
    pub reference: Option<True>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Vec<LossKind>>", optional)]
    pub known_loss: Option<Vec<LossKind>>,
    /// Stage rows (`#s<k>`): the build stage.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<u32>", optional)]
    pub stage: Option<u32>,
    /// Transition rows (`#t@<pm>`): the frame's per-mille of the transition.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<u16>", optional)]
    pub transition_pm: Option<u16>,
    /// Host lane.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<HostCheck>", optional)]
    pub host: Option<HostCheck>,
    /// Fonts lane.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<FontsRowMetrics>", optional)]
    pub fonts: Option<FontsRowMetrics>,
    /// Motion lane.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<MotionRowMetrics>", optional)]
    pub motion: Option<MotionRowMetrics>,
    /// Shell lane.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<ShellCheck>", optional)]
    pub shell: Option<ShellCheck>,
}

impl RowRecord {
    /// The id its fields make (`stage` wins over `transitionPm`; [`Results::validate`] refuses a row with both).
    pub fn id(&self) -> RowId {
        let id = RowId::new(self.presentation.clone(), self.slide.clone(), self.frontend);
        match (self.stage, self.transition_pm) {
            (Some(k), _) => id.with_step(RowStep::Stage(k)),
            (None, Some(pm)) => id.with_step(RowStep::Transition(pm)),
            (None, None) => id,
        }
    }

    /// Flagged rows (reference, known loss) are reported and stay out of acceptance.
    pub fn flagged(&self) -> bool {
        self.reference.is_some() || self.known_loss.as_ref().is_some_and(|k| !k.is_empty())
    }
}

/// A quantised median and 10th percentile, `null` over no row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Distribution {
    #[serde(with = "nullable")]
    #[ts(as = "Option<f64>")]
    pub median: Option<f64>,
    #[serde(with = "nullable")]
    #[ts(as = "Option<f64>")]
    pub p10: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Mean {
    #[serde(with = "nullable")]
    #[ts(as = "Option<f64>")]
    pub mean: Option<f64>,
}

/// Class counts, ssim median and 10th percentile over scored rows, and mean text recall.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrontendSummary {
    pub rows: u32,
    #[serde(rename = "match")]
    #[ts(rename = "match")]
    pub matched: u32,
    pub close: u32,
    pub mismatch: u32,
    pub declined: u32,
    #[serde(rename = "render-error")]
    #[ts(rename = "render-error")]
    pub render_error: u32,
    pub failed: u32,
    pub ssim: Distribution,
    pub text_recall: Mean,
}

impl FrontendSummary {
    pub fn count(&self, outcome: Outcome) -> u32 {
        match outcome {
            Outcome::Match => self.matched,
            Outcome::Close => self.close,
            Outcome::Mismatch => self.mismatch,
            Outcome::Declined => self.declined,
            Outcome::RenderError => self.render_error,
            Outcome::Failed => self.failed,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Summary {
    /// Per frontend, reference rows apart.
    pub by_frontend: BTreeMap<Frontend, FrontendSummary>,
    pub reference: BTreeMap<Frontend, FrontendSummary>,
    /// Declined rows (reference rows apart) by deciding code.
    pub declines: BTreeMap<String, u32>,
    /// The lane's own summaries, per frontend (every lane but `static`).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<LaneSummaries>", optional)]
    pub lanes: Option<LaneSummaries>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HoldoutScored {
    pub rows: u32,
    pub classes: BTreeMap<Outcome, u32>,
    pub ssim: Distribution,
    pub text_recall: Mean,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct HoldoutReference {
    pub rows: u32,
    pub classes: BTreeMap<Outcome, u32>,
    pub ssim: Distribution,
}

/// The holdout gate's aggregate: counts and distributions only, no row.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct HoldoutAggregate {
    pub scored: HoldoutScored,
    pub reference: HoldoutReference,
}

/// A results file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Results {
    pub schema: ResultsSchema,
    pub stamp: Stamp,
    pub rows: BTreeMap<RowId, RowRecord>,
    pub summary: Summary,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<HoldoutAggregate>", optional)]
    pub holdout: Option<HoldoutAggregate>,
    /// Motion lane: timeline fidelity per slide, keyed by the slide's static row id.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<BTreeMap<RowId, TimelineFidelity>>", optional)]
    pub timelines: Option<BTreeMap<RowId, TimelineFidelity>>,
    /// Fonts lane: what each pair's compile resolved.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<BTreeMap<PairKey, FontsPairReport>>", optional)]
    pub font_pairs: Option<BTreeMap<PairKey, FontsPairReport>>,
    /// Shell lane: each pair's zip file list check.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<BTreeMap<PairKey, ShellPairCheck>>", optional)]
    pub shell_pairs: Option<BTreeMap<PairKey, ShellPairCheck>>,
    /// Motion lane: each deck's transitions and steps by verdict.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<BTreeMap<PairKey, MotionPairReport>>", optional)]
    pub motion_pairs: Option<BTreeMap<PairKey, MotionPairReport>>,
    /// Motion lane: the settled frame of every shown slide at every position (`#s<k>`, steps counted from 0), the
    /// frames the shell lane compares the unzipped deck with.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<BTreeMap<RowId, Sha256>>", optional)]
    pub positions: Option<BTreeMap<RowId, Sha256>>,
}

impl Results {
    /// Parses and validates a results file.
    pub fn parse(text: &str) -> Result<Results, Error> {
        let results: Results = crate::parse(text)?;
        results.validate()?;
        Ok(results)
    }

    /// The file's canonical text.
    pub fn to_canonical(&self) -> Result<String, Error> {
        self.validate()?;
        crate::to_canonical(self)
    }

    /// Each row sits under its own id, its outcome and reason agree, and lane fields appear only in their lane.
    pub fn validate(&self) -> Result<(), Error> {
        let mut issues = Issues::default();
        let lane = self.stamp.lane;
        for (id, row) in &self.rows {
            let own = row.id();
            if &own != id {
                issues.push(format!("{id}: the record is row {own}"));
            }
            if row.stage.is_some() && row.transition_pm.is_some() {
                issues.push(format!(
                    "{id}: a row is a stage or a transition frame, not both"
                ));
            }
            if row.oracle.page == 0 {
                issues.push(format!("{id}: oracle pages are 1-based"));
            }
            if let (Some(reason), Some(outcome)) = (row.reason, row.outcome.classifier()) {
                let expected = if reason.is_failed() {
                    Outcome::Failed
                } else {
                    Outcome::RenderError
                };
                if outcome != expected {
                    issues.push(format!("{id}: a {outcome:?} row has reason {reason:?}"));
                }
            }
            // A transition row always takes a motion class; a stage row takes one where no stages oracle scores it.
            match (row.outcome, id.step()) {
                (RowOutcome::Motion(_), RowStep::Transition(_) | RowStep::Stage(_))
                | (RowOutcome::Classifier(_), RowStep::Static | RowStep::Stage(_))
                | (RowOutcome::Fonts(_), RowStep::Static) => {}
                (RowOutcome::Motion(_), RowStep::Static) | (_, RowStep::Transition(_)) => {
                    issues.push(format!(
                        "{id}: transition rows take a motion class, static rows never do"
                    ));
                }
                (RowOutcome::Fonts(_), RowStep::Stage(_)) => {
                    issues.push(format!("{id}: a stage row takes no fonts class"));
                }
            }
            if let RowOutcome::Fonts(class) = row.outcome {
                match &row.fonts {
                    _ if lane != Lane::Fonts => {
                        issues.push(format!("{id}: a fonts class in a {lane:?} results file"));
                    }
                    None => issues.push(format!("{id}: a fonts class without fonts metrics")),
                    Some(fonts) => {
                        let complete = fonts.total.fallback == 0;
                        if complete != (class == FontsOutcome::FontsComplete) {
                            issues.push(format!(
                                "{id}: {class:?} with {} glyphs drawn by fallback fonts",
                                fonts.total.fallback
                            ));
                        }
                    }
                }
            }
            if let Some(shadow) = &row.shadow {
                if row.scores.is_none() {
                    issues.push(format!("{id}: shadow metrics on an unscored row"));
                }
                if shadow.elements == 0 {
                    issues.push(format!("{id}: shadow metrics without a ring"));
                }
            }
            if let Some(gradient) = &row.gradient {
                if row.scores.is_none() {
                    issues.push(format!("{id}: gradient metrics on an unscored row"));
                }
                if gradient.elements == 0 {
                    issues.push(format!("{id}: gradient metrics without a region"));
                }
            }
            for (present, wanted, name) in [
                (row.shadow.is_some(), Lane::Static, "shadow"),
                (row.gradient.is_some(), Lane::Static, "gradient"),
                (row.host.is_some(), Lane::Host, "host"),
                (row.fonts.is_some(), Lane::Fonts, "fonts"),
                (row.motion.is_some(), Lane::Motion, "motion"),
                (row.shell.is_some(), Lane::Shell, "shell"),
                (
                    matches!(id.step(), RowStep::Transition(_)),
                    Lane::Motion,
                    "a transition",
                ),
            ] {
                if present && lane != wanted {
                    issues.push(format!("{id}: {name} metrics in a {lane:?} results file"));
                }
            }
            if matches!(id.step(), RowStep::Stage(_)) && !matches!(lane, Lane::Motion | Lane::Shell)
            {
                issues.push(format!("{id}: a stage row in a {lane:?} results file"));
            }
        }
        for (present, wanted, name) in [
            (self.timelines.is_some(), Lane::Motion, "timelines"),
            (self.font_pairs.is_some(), Lane::Fonts, "fontPairs"),
            (self.shell_pairs.is_some(), Lane::Shell, "shellPairs"),
            (self.motion_pairs.is_some(), Lane::Motion, "motionPairs"),
            (self.positions.is_some(), Lane::Motion, "positions"),
        ] {
            if present && lane != wanted {
                issues.push(format!("{name} in a {lane:?} results file"));
            }
        }
        for id in self.positions.iter().flat_map(BTreeMap::keys) {
            if !matches!(id.step(), RowStep::Stage(_)) {
                issues.push(format!("positions: {id} is not a stage"));
            }
        }
        issues.finish()
    }
}
