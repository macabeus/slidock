//! The wire types of `slidock-bench ui`'s read-only API: what each GET returns beyond the
//! files themselves (`results.json`, `runs.json`, `parity.json`, which the UI reads as [`crate::results::Results`],
//! [`crate::runs::RunsIndex`] and [`crate::parity::ParityReport`]). A type's doc names its route only when the
//! server (`slidock-bench`'s `server.rs`) serves one; a test there holds the two to each other.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::corpus::{Feature, KnownLoss};
use crate::de::{nullable, optional};
use crate::ids::{Frontend, Lane, PresentationId, RowId, Sha256, Tier};
use crate::parity::{AllowEntry, ParityReport};
use crate::results::{HoldoutAggregate, Results, RowRecord};
use crate::stamp::{CompilerStamp, FontsStamp, MetricsStamp, RendererStamp};

/// `classifier@2`'s match thresholds, for the outcome legend.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MatchThresholds {
    pub ssim_mean: f64,
    pub ssim_p1: f64,
    pub color_mean: f64,
    pub text_recall: f64,
    pub p95_pm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloseThresholds {
    pub ssim_mean: f64,
    pub color_mean: f64,
    pub text_recall: f64,
    pub p95_pm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Thresholds {
    #[serde(rename = "match")]
    #[ts(rename = "match")]
    pub matched: MatchThresholds,
    pub close: CloseThresholds,
}

/// HEAD's ids, to flag a results file stale for HEAD.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct HeadIds {
    pub commit: String,
    pub content: Sha256,
    pub harness: Sha256,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LatestRun {
    pub partial: bool,
}

/// `GET /api/status`
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub head: HeadIds,
    /// The lanes with a baseline.
    pub baselines: Vec<Lane>,
    /// `runs/latest`, when the cache has one.
    #[serde(with = "nullable")]
    #[ts(as = "Option<LatestRun>")]
    pub latest: Option<LatestRun>,
    /// A holdout aggregate is recorded.
    pub holdout: bool,
    /// A parity report exists for the latest run.
    pub parity: bool,
    pub thresholds: Thresholds,
}

/// A source of a [`PublicManifest`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct PublicSource {
    pub frontend: Frontend,
    pub file: String,
    #[serde(with = "nullable")]
    #[ts(as = "Option<u64>")]
    pub bytes: Option<u64>,
    pub rows: u32,
    pub reference: bool,
    /// The corpus folder holds a file with the source's sha256.
    pub available: bool,
    #[serde(rename = "knownLoss")]
    #[ts(rename = "knownLoss")]
    pub known_loss: Vec<KnownLoss>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct PublicPresentation {
    pub id: PresentationId,
    /// Never `holdout`.
    pub tier: Tier,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Vec<Feature>>", optional)]
    pub features: Option<Vec<Feature>>,
    pub sources: Vec<PublicSource>,
}

/// The corpus manifest without the holdout. No route serves it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct PublicManifest {
    pub presentations: Vec<PublicPresentation>,
}

/// Declined rows by code and feature, most rows first (`slidock-bench histogram`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct HistogramEntry {
    pub code: String,
    pub feature: String,
    pub rows: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum ResultsSource {
    Baseline,
    Latest,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Declines {
    /// Without the reference and known-loss rows.
    pub default: Vec<HistogramEntry>,
    pub all: Vec<HistogramEntry>,
}

/// A results file with the holdout's rows, pairs and aggregate removed, and its declines. No route serves it:
/// `GET /api/runs/<run>/results` answers the [`Results`] alone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ResultsView {
    pub source: ResultsSource,
    pub results: Results,
    pub declines: Declines,
}

/// The part of a holdout run's stamp that names no holdout pair.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct HoldoutStamp {
    pub compiler: CompilerStamp,
    pub harness: Sha256,
    pub renderer: RendererStamp,
    pub fonts: FontsStamp,
    pub metrics: MetricsStamp,
    pub classifier: String,
}

/// The holdout's aggregate and the part of its run's stamp that names no holdout pair. No route serves it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct HoldoutView {
    pub holdout: HoldoutAggregate,
    pub stamp: HoldoutStamp,
}

/// Where an image came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum ImageFrom {
    /// The row's run folder.
    Run,
    /// `slidock-bench explain` of some run.
    Explain,
    OracleCache,
    /// `parity reference`: the golden compile output rendered by this bench.
    GoldenReference,
}

/// An image the cache holds, or the command that produces it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(untagged)]
pub enum ImageRef {
    Found {
        url: String,
        from: ImageFrom,
        /// For `explain`: the run it explained.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
        #[ts(as = "Option<String>", optional)]
        run: Option<String>,
    },
    Missing {
        /// The command that produces the image.
        missing: String,
    },
}

/// The images of a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum ImageKind {
    Oracle,
    Render,
    Heatmap,
    /// The golden compile output's render of the row.
    Golden,
    /// The difference between the run's render and the golden's.
    GoldenDiff,
    /// Motion rows: the first, middle and last frame of what the row plays (`motion.frames`).
    Start,
    Mid,
    End,
    /// Stage rows: the player's still render of the position.
    Still,
}

/// The state of the run a record was scored in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum RunState {
    /// Its files are the record's.
    Present,
    /// `gc` removed the folder.
    Collected,
    /// A later overlay scored the row again in it.
    Rescored,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RowRun {
    #[serde(with = "nullable")]
    #[ts(as = "Option<Sha256>")]
    pub key: Option<Sha256>,
    pub state: RunState,
}

/// A diagnostic that applies to a row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct AppliedDiagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub feature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub container: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<u32>", optional)]
    pub count: Option<u32>,
    /// The layout, master or `presentation` it comes through; absent for the slide's own.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub via: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Moved {
    pub dx_pm: f64,
    pub dy_pm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Resized {
    pub height_ratio: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Restyled {
    pub words: u32,
    pub max_delta_e: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reflowed {
    pub oracle_lines: u32,
    pub render_lines: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct FontFinding {
    pub family: String,
    pub binding: String,
    pub kind: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageFinding {
    pub ssim: f64,
    pub delta_e: f64,
}

/// `explain`'s findings for one element, worst first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ElementFinding {
    pub el: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub name: Option<String>,
    pub origin: String,
    pub words: u32,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Moved>", optional)]
    pub moved: Option<Moved>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Resized>", optional)]
    pub resized: Option<Resized>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Restyled>", optional)]
    pub restyled: Option<Restyled>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Reflowed>", optional)]
    pub reflowed: Option<Reflowed>,
    pub fonts: Vec<FontFinding>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<ImageFinding>", optional)]
    pub image: Option<ImageFinding>,
    pub missing: u32,
    pub extra: u32,
}

/// Missing oracle words, grouped by oracle line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MissingLine {
    pub text: String,
    pub x_pm: f64,
    pub y_pm: f64,
    /// The element nearest the line.
    #[serde(with = "nullable")]
    #[ts(as = "Option<String>")]
    pub el: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Explain {
    #[serde(with = "nullable")]
    #[ts(as = "Option<Sha256>")]
    pub run: Option<Sha256>,
    /// False when the findings come from another record of the row.
    pub matches_row: bool,
    pub elements: Vec<ElementFinding>,
    pub missing: Vec<MissingLine>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RowCommands {
    pub explain: String,
    pub run: String,
}

/// The row's source file, which the consumer app can open while the corpus holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RowViewer {
    pub available: bool,
    pub file: String,
}

/// `GET /api/runs/<run>/rows/<row>`
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RowDetail {
    pub id: RowId,
    pub source: ResultsSource,
    pub run: RowRun,
    pub record: RowRecord,
    pub known_loss: Vec<KnownLoss>,
    /// The diagnostics that apply to the row; null without the run's report.
    #[serde(with = "nullable")]
    #[ts(as = "Option<Vec<AppliedDiagnostic>>")]
    pub diagnostics: Option<Vec<AppliedDiagnostic>>,
    pub images: BTreeMap<ImageKind, ImageRef>,
    #[serde(with = "nullable")]
    #[ts(as = "Option<Explain>")]
    pub explain: Option<Explain>,
    pub commands: RowCommands,
    pub viewer: RowViewer,
}

/// `GET /api/runs/<run>/parity`: the run's parity report with the bench's allowlist (`bench/parity/allow.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ParityView {
    pub report: ParityReport,
    pub allowlist: Vec<AllowEntry>,
}
