//! Parity with slidock's goldens: the reference (`slidock-parity-ref@2`), the golden
//! compile output as this bench renders and scores it; the parity report a run writes (`slidock-parity@2`, levels B
//! compiler and C IR); the allowlist of reviewed divergences; and the row rules the `parity` commands apply.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::compare;
use crate::de::optional;
use crate::error::{Error, Issues};
use crate::ids::{Lane, RowId, Sha256};
use crate::literal::literal;
use crate::results::{Decline, Outcome, Reason, RowRecord, Scores};
use crate::stamp::Stamp;

literal!(
    /// The reference format.
    ReferenceSchema = "slidock-parity-ref@2"
);
literal!(
    /// The parity report format.
    ReportSchema = "slidock-parity@2"
);

/// The golden set a reference was rendered from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct GoldenIdentity {
    /// The commit whose CLI recorded the goldens.
    pub commit: String,
    /// The sha256 of the committed golden manifest, which pins every golden file.
    pub manifest: Sha256,
}

/// A reference row: what the golden compile produced for it, hashes and numbers only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ReferenceRow {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Sha256>", optional)]
    pub html: Option<Sha256>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Sha256>", optional)]
    pub render: Option<Sha256>,
    pub outcome: Outcome,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Reason>", optional)]
    pub reason: Option<Reason>,
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
    pub diagnostics: BTreeMap<String, u32>,
    /// The sha256 of the slide's canonical timeline JSON.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Sha256>", optional)]
    pub timeline: Option<Sha256>,
    /// The sha256 of the canonical JSON of the transition into the slide.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Sha256>", optional)]
    pub transition: Option<Sha256>,
}

/// The reference: the golden compile output rendered and scored by this bench (`parity reference`), hash-only,
/// keyed under the harness, renderer recipe, font set, metric versions and classifier it was scored under.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParityReference {
    pub schema: ReferenceSchema,
    pub golden: GoldenIdentity,
    /// The harness hash of the bench that rendered and scored it.
    pub harness: Sha256,
    pub renderer_recipe: Sha256,
    pub font_set_id: Sha256,
    pub metrics: BTreeMap<String, String>,
    pub classifier: String,
    pub rows: BTreeMap<RowId, ReferenceRow>,
}

impl ReferenceRow {
    /// What classes the row when it has no render.
    fn unrendered(&self) -> Unrendered<'_> {
        Unrendered {
            outcome: Some(self.outcome),
            reason: self.reason,
            code: self.code.as_deref(),
            data: self.data.as_ref(),
            declined: self.declined.as_ref(),
        }
    }
}

/// A row without a render: its class and everything that explains it.
#[derive(PartialEq)]
struct Unrendered<'a> {
    outcome: Option<Outcome>,
    reason: Option<Reason>,
    code: Option<&'a str>,
    data: Option<&'a BTreeMap<String, String>>,
    declined: Option<&'a Decline>,
}

impl<'a> From<&'a RowRecord> for Unrendered<'a> {
    fn from(row: &'a RowRecord) -> Unrendered<'a> {
        Unrendered {
            outcome: row.outcome.classifier(),
            reason: row.reason,
            code: row.code.as_deref(),
            data: row.data.as_ref(),
            declined: row.declined.as_ref(),
        }
    }
}

impl ParityReference {
    pub fn parse(text: &str) -> Result<ParityReference, Error> {
        crate::parse(text)
    }

    /// The stamp fields of a run that disagree with what the reference is keyed under: its harness, renderer recipe,
    /// font set, metric versions and classifier, and the static lane. Non-empty means `parity check` refuses the run.
    pub fn mismatches(&self, stamp: &Stamp) -> Vec<String> {
        let metrics = &stamp.metrics;
        let stamped = [
            ("color", &metrics.color),
            ("pixel", &metrics.pixel),
            ("ssim", &metrics.ssim),
            ("text", &metrics.text),
        ];
        let metrics_equal = self.metrics.len() == stamped.len()
            && stamped
                .iter()
                .all(|(name, version)| self.metrics.get(*name) == Some(*version));
        [
            ("harness", self.harness == stamp.harness),
            (
                "renderer.recipe",
                self.renderer_recipe == stamp.renderer.recipe,
            ),
            (
                "fonts.fontSetId",
                self.font_set_id == stamp.fonts.font_set_id,
            ),
            ("metrics", metrics_equal),
            ("classifier", self.classifier == stamp.classifier),
            ("lane", stamp.lane == Lane::Static),
        ]
        .into_iter()
        .filter(|(_, equal)| !equal)
        .map(|(name, _)| name.to_owned())
        .collect()
    }

    pub fn to_canonical(&self) -> Result<String, Error> {
        crate::to_canonical(self)
    }
}

/// B, per row: slidock's compile against the golden's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum ParityStatus {
    /// The composed document and the render are equal.
    Identical,
    /// The render is equal, the document differs.
    PixelIdentical,
    /// The render differs and the class moved up.
    DriftBetter,
    /// The render differs and the class stayed, within the erosion guards.
    DriftSameClass,
    /// The class moved down, or stayed and broke an erosion guard.
    Regressed,
    /// The run lacks a row the reference has.
    Missing,
    /// The run scores a row the reference declined or failed.
    New,
}

impl ParityStatus {
    /// Statuses `parity check` fails on unless the allowlist lists the row.
    pub const fn needs_review(self) -> bool {
        matches!(
            self,
            ParityStatus::DriftBetter
                | ParityStatus::DriftSameClass
                | ParityStatus::Regressed
                | ParityStatus::Missing
        )
    }

    /// The status of the run's row against the reference's, with the guards it broke.
    pub fn of(reference: &ReferenceRow, ours: Option<&RowRecord>) -> (ParityStatus, Vec<String>) {
        let Some(ours) = ours else {
            return (ParityStatus::Missing, Vec::new());
        };
        let Some(outcome) = ours.outcome.classifier() else {
            return (ParityStatus::Regressed, Vec::new());
        };
        if !reference.outcome.is_scored()
            && reference.outcome != Outcome::RenderError
            && outcome.is_scored()
        {
            return (ParityStatus::New, Vec::new());
        }
        let render_equal = match (&reference.render, &ours.render) {
            (Some(a), Some(b)) => a == b,
            (None, None) => reference.unrendered() == Unrendered::from(ours),
            _ => false,
        };
        if render_equal {
            let status = if reference.html == ours.html {
                ParityStatus::Identical
            } else {
                ParityStatus::PixelIdentical
            };
            return (status, Vec::new());
        }
        match compare::Movement::between(reference.outcome, outcome) {
            compare::Movement::Improved | compare::Movement::Undeclined => {
                (ParityStatus::DriftBetter, Vec::new())
            }
            compare::Movement::Regressed => (ParityStatus::Regressed, Vec::new()),
            compare::Movement::Same => {
                let guards = match (&reference.scores, &ours.scores) {
                    (Some(base), Some(current)) => compare::erosion_guards(base, current),
                    _ => Vec::new(),
                };
                let status = if guards.is_empty() {
                    ParityStatus::DriftSameClass
                } else {
                    ParityStatus::Regressed
                };
                (status, guards)
            }
        }
    }
}

/// The render of the run's row against the reference's, both at metric size.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenderVsRender {
    pub ssim: f64,
    pub delta_e: f64,
}

/// B, one row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompilerRow {
    pub status: ParityStatus,
    /// The reference's class.
    pub reference: Outcome,
    /// The run's class; absent for a missing row.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Outcome>", optional)]
    pub outcome: Option<Outcome>,
    /// The erosion guards a same-class drift broke.
    pub guards: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<RenderVsRender>", optional)]
    pub render_vs_render: Option<RenderVsRender>,
    /// Level C: the first differing path of the row's IR slice against the golden IR.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub first_ir_diff: Option<String>,
    /// The run's diagnostic counts minus the reference's, non-zero codes only.
    pub diagnostics: BTreeMap<String, i64>,
    /// The allowlist entry that accepts the row.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<AllowEntry>", optional)]
    pub allowed: Option<AllowEntry>,
}

impl CompilerRow {
    /// Classifies the row and fills what the records alone give: status, classes, guards, diagnostic deltas and
    /// the allowlist entry.
    pub fn compare(
        id: &RowId,
        reference: &ReferenceRow,
        ours: Option<&RowRecord>,
        allowlist: &Allowlist,
    ) -> CompilerRow {
        let (status, guards) = ParityStatus::of(reference, ours);
        let mut diagnostics = BTreeMap::new();
        if let Some(ours) = ours {
            let codes: BTreeSet<&String> = reference
                .diagnostics
                .keys()
                .chain(ours.diagnostics.keys())
                .collect();
            for code in codes {
                let count =
                    |map: &BTreeMap<String, u32>| i64::from(map.get(code).copied().unwrap_or(0));
                let delta = count(&ours.diagnostics) - count(&reference.diagnostics);
                if delta != 0 {
                    diagnostics.insert(code.clone(), delta);
                }
            }
        }
        CompilerRow {
            status,
            reference: reference.outcome,
            outcome: ours.and_then(|row| row.outcome.classifier()),
            guards,
            render_vs_render: None,
            first_ir_diff: None,
            diagnostics,
            allowed: allowlist.entry(id).cloned(),
        }
    }

    /// The row fails `parity check`: it needs review and no allowlist entry accepts its status (`--strict`: it is
    /// not identical).
    pub fn fails(&self, strict: bool) -> bool {
        if strict {
            return self.status != ParityStatus::Identical;
        }
        self.status.needs_review()
            && !self
                .allowed
                .as_ref()
                .is_some_and(|entry| entry.status.is_none_or(|status| status == self.status))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompilerParity {
    /// One per reference row.
    pub rows: BTreeMap<RowId, CompilerRow>,
    /// The run's rows the reference lacks: a reference that does not cover the run compares none of them, so they
    /// fail the check.
    pub unreferenced: Vec<RowId>,
    pub counts: BTreeMap<ParityStatus, u32>,
    /// Rows `parity check` fails on.
    pub failing: Vec<RowId>,
    pub strict: bool,
}

/// What `parity check` exits with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Exit 0.
    Pass,
    /// Exit 1: a row needs review.
    Fail,
    /// Exit 2: the run's stamp disagrees with what the reference is keyed under, in these fields.
    Refused(Vec<String>),
    /// Exit 3: no row was compared.
    NothingCompared,
}

impl Verdict {
    /// `parity check` on a run stamped `stamp`: refused when the stamp disagrees with what `reference` is keyed
    /// under, else the rows' verdict.
    pub fn check(reference: &ParityReference, stamp: &Stamp, parity: &CompilerParity) -> Verdict {
        let mismatches = reference.mismatches(stamp);
        if mismatches.is_empty() {
            parity.verdict()
        } else {
            Verdict::Refused(mismatches)
        }
    }

    pub const fn exit_code(&self) -> u8 {
        match self {
            Verdict::Pass => 0,
            Verdict::Fail => 1,
            Verdict::Refused(_) => 2,
            Verdict::NothingCompared => 3,
        }
    }
}

impl CompilerParity {
    /// `rows` holds one compared row per reference row; `ours` is the run's rows (the holdout's left out), whose ids
    /// outside `rows` are unreferenced and fail.
    pub fn build(
        rows: BTreeMap<RowId, CompilerRow>,
        ours: &BTreeMap<RowId, RowRecord>,
        strict: bool,
    ) -> CompilerParity {
        let mut counts = BTreeMap::new();
        for row in rows.values() {
            *counts.entry(row.status).or_insert(0) += 1;
        }
        let unreferenced: Vec<RowId> = ours
            .keys()
            .filter(|id| !rows.contains_key(*id))
            .cloned()
            .collect();
        let mut failing: Vec<RowId> = rows
            .iter()
            .filter(|(_, row)| row.fails(strict))
            .map(|(id, _)| id.clone())
            .collect();
        failing.extend(unreferenced.iter().cloned());
        failing.sort();
        CompilerParity {
            rows,
            unreferenced,
            counts,
            failing,
            strict,
        }
    }

    /// The rows' verdict: pass, fail, or nothing compared.
    pub fn verdict(&self) -> Verdict {
        let compared = self
            .rows
            .values()
            .any(|row| row.status != ParityStatus::Missing);
        if !compared {
            Verdict::NothingCompared
        } else if self.failing.is_empty() {
            Verdict::Pass
        } else {
            Verdict::Fail
        }
    }
}

/// C, per container.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum IrStatus {
    Equal,
    Differs,
    /// One side lacks the container.
    Missing,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct IrEntry {
    pub status: IrStatus,
    /// The first differing JSON path (`$.elements[2].fill`).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub path: Option<String>,
}

/// C: keyed `<presentation>:<frontend>:<container id>`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct IrParity {
    pub containers: BTreeMap<String, IrEntry>,
}

/// The first JSON path where two values differ, walking object keys in canonical (UTF-16) order; `None` when they
/// are equal. Numbers compare by value.
pub fn first_difference(ours: &serde_json::Value, reference: &serde_json::Value) -> Option<String> {
    fn walk(a: &serde_json::Value, b: &serde_json::Value, path: &mut String) -> bool {
        use serde_json::Value;
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => {
                let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
                keys.sort_by(|p, q| slidock_canon::utf16::cmp(p, q));
                keys.dedup();
                for key in keys {
                    let len = path.len();
                    path.push('.');
                    path.push_str(key);
                    let differs = match (x.get(key), y.get(key)) {
                        (Some(p), Some(q)) => walk(p, q, path),
                        _ => true,
                    };
                    if differs {
                        return true;
                    }
                    path.truncate(len);
                }
                false
            }
            (Value::Array(x), Value::Array(y)) => {
                for (i, (p, q)) in x.iter().zip(y).enumerate() {
                    let len = path.len();
                    path.push_str(&format!("[{i}]"));
                    if walk(p, q, path) {
                        return true;
                    }
                    path.truncate(len);
                }
                if x.len() != y.len() {
                    path.push_str(&format!("[{}]", x.len().min(y.len())));
                    return true;
                }
                false
            }
            (Value::Number(x), Value::Number(y)) => x.as_f64() != y.as_f64(),
            _ => a != b,
        }
    }
    let mut path = String::from("$");
    walk(ours, reference, &mut path).then_some(path)
}

/// The reference a report compared against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportReference {
    /// The sha256 of the reference file.
    pub sha256: Sha256,
    pub golden: GoldenIdentity,
    pub renderer_recipe: Sha256,
    pub font_set_id: Sha256,
}

/// `runs/<key>/parity.json`: every level the run checked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ParityReport {
    pub schema: ReportSchema,
    pub reference: ReportReference,
    /// The key of the run compared (`Stamp::run_key`): `runs/<run>/` holds this file.
    pub run: Sha256,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<CompilerParity>", optional)]
    pub compiler: Option<CompilerParity>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<IrParity>", optional)]
    pub ir: Option<IrParity>,
}

impl ParityReport {
    pub fn parse(text: &str) -> Result<ParityReport, Error> {
        crate::parse(text)
    }

    pub fn to_canonical(&self) -> Result<String, Error> {
        crate::to_canonical(self)
    }

    /// The hash the run's stamp records.
    pub fn sha256(&self) -> Result<Sha256, Error> {
        crate::sha256_json(self)
    }
}

/// A reviewed divergence (`bench/parity/allow.json`): a row whose output differs from the goldens on purpose, with
/// its reason.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllowEntry {
    pub row: RowId,
    pub reason: String,
    /// The start of the row's first differing IR path (or `composed line N` when its IR slice is identical).
    pub path: String,
    /// `YYYY-MM-DD`: when the entry is reviewed again.
    pub review_by: String,
    /// The parity status the entry accepts; any status needing review when absent.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<ParityStatus>", optional)]
    pub status: Option<ParityStatus>,
}

/// The allowlist: one entry per row.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(transparent)]
pub struct Allowlist(pub Vec<AllowEntry>);

/// `YYYY-MM-DD` with a month of 1–12 and a day of 1–31.
fn is_date(text: &str) -> bool {
    let mut parts = text.split('-');
    let number = |part: Option<&str>, len: usize| {
        part.filter(|p| p.len() == len && p.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|p| p.parse::<u32>().ok())
    };
    let year = number(parts.next(), 4);
    let month = number(parts.next(), 2);
    let day = number(parts.next(), 2);
    year.is_some()
        && month.is_some_and(|m| (1..=12).contains(&m))
        && day.is_some_and(|d| (1..=31).contains(&d))
        && parts.next().is_none()
}

impl Allowlist {
    pub fn parse(text: &str) -> Result<Allowlist, Error> {
        let list: Allowlist = crate::parse(text)?;
        list.validate()?;
        Ok(list)
    }

    /// Each row once, with a reason, a path and a review date.
    pub fn validate(&self) -> Result<(), Error> {
        let mut issues = Issues::default();
        let mut rows = BTreeSet::new();
        for entry in &self.0 {
            let row = &entry.row;
            if !rows.insert(row) {
                issues.push(format!("{row}: listed twice"));
            }
            if entry.reason.is_empty() || entry.path.is_empty() {
                issues.push(format!(
                    "{row}: an allowlist entry gives its reason and path"
                ));
            }
            if !is_date(&entry.review_by) {
                issues.push(format!("{row}: reviewBy is a YYYY-MM-DD date"));
            }
            if entry.status.is_some_and(|status| !status.needs_review()) {
                issues.push(format!(
                    "{row}: only a status that needs review can be allowed"
                ));
            }
        }
        issues.finish()
    }

    pub fn entry(&self, row: &RowId) -> Option<&AllowEntry> {
        self.0.iter().find(|entry| &entry.row == row)
    }
}
