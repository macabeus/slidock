//! What `regression` and `diff` report, and the row-level rules both share.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::de::optional;
use crate::ids::RowId;
use crate::lanes::FontsPairReport;
use crate::results::{FontsOutcome, Outcome, RowOutcome, RowRecord, Scores};

/// How a row's class moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum Movement {
    Same,
    Regressed,
    Improved,
    /// A declined row that is now scored: its own bucket, for review.
    Undeclined,
}

impl Movement {
    /// How a fonts-lane row moved: failed 0 < render-error 1 < fonts-fallback 2 < fonts-complete 3.
    pub fn between_fonts(base: RowOutcome, current: RowOutcome) -> Movement {
        let (b, c) = (fonts_rank(base), fonts_rank(current));
        if base == current {
            Movement::Same
        } else if c > b {
            Movement::Improved
        } else {
            Movement::Regressed
        }
    }

    pub fn between(base: Outcome, current: Outcome) -> Movement {
        if base == current {
            Movement::Same
        } else if base == Outcome::Declined && current.is_scored() {
            Movement::Undeclined
        } else if current.rank() > base.rank() {
            Movement::Improved
        } else {
            Movement::Regressed
        }
    }
}

/// Within-class guards: a row may keep its class and still lose at most this much.
pub mod erosion {
    pub const SSIM_MEAN: f64 = -0.005;
    pub const TEXT_RECALL: f64 = -0.01;
    pub const TEXT_P95_PM: f64 = 1.4;
    pub const COLOR_MEAN: f64 = 1.0;
    /// `shadow.worstRmse` may grow this much.
    pub const SHADOW_WORST_RMSE: f64 = 0.01;
    /// `gradient.worstRmse` may grow this much.
    pub const GRADIENT_WORST_RMSE: f64 = 0.01;
}

/// The shadow-not-drawn codes (Keynote, PPTX, ODP): a row whose count of any grows regressed, whatever its class did.
pub const SHADOW_NOT_DRAWN: [&str; 3] = ["KN1304", "PX2111", "OD9104"];

/// The shadow-not-drawn codes whose count grew from `base` to `current`, as `<code> <base> → <current>`.
pub fn shadow_losses(base: &RowRecord, current: &RowRecord) -> Vec<String> {
    grown_counts(&SHADOW_NOT_DRAWN, base, current)
}

/// The gradient-approximated codes (Keynote, PPTX, ODP): a row whose count of any grows regressed, whatever its class
/// did.
pub const GRADIENT_APPROXIMATED: [&str; 3] = ["KN1301", "PX2106", "OD9103"];

/// The gradient-approximated codes whose count grew from `base` to `current`, as `<code> <base> → <current>`.
pub fn gradient_losses(base: &RowRecord, current: &RowRecord) -> Vec<String> {
    grown_counts(&GRADIENT_APPROXIMATED, base, current)
}

fn grown_counts(codes: &[&str], base: &RowRecord, current: &RowRecord) -> Vec<String> {
    codes
        .iter()
        .filter_map(|code| {
            let count = |row: &RowRecord| row.diagnostics.get(*code).copied().unwrap_or(0);
            let (b, c) = (count(base), count(current));
            (c > b).then(|| format!("{code} {b} → {c}"))
        })
        .collect()
}

/// A fonts-lane class's rank; a class of another lane ranks with the failures.
pub const fn fonts_rank(outcome: RowOutcome) -> u8 {
    match outcome {
        RowOutcome::Fonts(FontsOutcome::FontsComplete) => 3,
        RowOutcome::Fonts(FontsOutcome::FontsFallback) => 2,
        RowOutcome::Classifier(Outcome::RenderError) => 1,
        _ => 0,
    }
}

/// Fonts-lane guards: a row may keep its class and lose at most this much; a pair's fonts may grow this much.
pub mod fonts_erosion {
    /// The share of a row's glyphs drawn by an exact face.
    pub const EXACT_SHARE: f64 = -0.001;
    /// The shipped WOFF2 bytes of a pair, as a fraction of the base's.
    pub const FONT_BYTES_GROWTH: f64 = 0.1;
}

/// The fonts guards a row of unchanged class broke: fewer exact glyphs, more fallback glyphs, more coverage gaps.
pub fn fonts_eroded(base: &RowRecord, current: &RowRecord) -> Vec<String> {
    let (Some(b), Some(c)) = (&base.fonts, &current.fonts) else {
        return Vec::new();
    };
    if base.outcome != current.outcome {
        return Vec::new();
    }
    let n = slidock_canon::to_js_string;
    let mut broken = Vec::new();
    if let (Some(bs), Some(cs)) = (b.exact_share, c.exact_share)
        && cs - bs < fonts_erosion::EXACT_SHARE
    {
        broken.push(format!("exactShare {} → {}", n(bs), n(cs)));
    }
    if c.total.fallback > b.total.fallback {
        broken.push(format!(
            "fallback {} → {}",
            b.total.fallback, c.total.fallback
        ));
    }
    if c.total.coverage_gaps > b.total.coverage_gaps {
        broken.push(format!(
            "coverageGaps {} → {}",
            b.total.coverage_gaps, c.total.coverage_gaps
        ));
    }
    broken
}

/// The fonts guards a pair broke: its shipped bytes grew past the allowance, or faces it shipped a file for lost it.
pub fn fonts_pair_guards(
    pair: &str,
    base: &FontsPairReport,
    current: &FontsPairReport,
) -> Vec<String> {
    let mut broken = Vec::new();
    let limit = base.font_bytes as f64 * (1.0 + fonts_erosion::FONT_BYTES_GROWTH);
    if current.font_bytes as f64 > limit {
        broken.push(format!(
            "{pair} fontBytes {} → {}",
            base.font_bytes, current.font_bytes
        ));
    }
    let lost: Vec<String> = base
        .requested
        .iter()
        .filter(|face| face.source.is_some())
        .filter(|face| {
            current.requested.iter().any(|now| {
                now.source.is_none()
                    && now.family == face.family
                    && now.weight == face.weight
                    && now.style == face.style
            })
        })
        .map(|face| format!("{} {}", face.family, face.weight))
        .collect();
    if !lost.is_empty() {
        broken.push(format!("{pair} no file for {}", lost.join(", ")));
    }
    broken
}

/// Per frontend: the largest allowed drop of the ssim median or 10th percentile.
pub const FRONTEND_SSIM_DROP: f64 = 0.002;

/// The guards a scored row of unchanged class broke, as `<field> <base> → <current>`: its scores', then its
/// shadow rings': fewer rings (a shadow gone from the IR without a not-drawn diagnostic, which keeps its ring), or a
/// worse worst ring; then its gradient regions', alike.
pub fn eroded(base: &RowRecord, current: &RowRecord) -> Vec<String> {
    let mut broken = match (&base.scores, &current.scores) {
        (Some(b), Some(c)) if base.outcome == current.outcome => erosion_guards(b, c),
        _ => return Vec::new(),
    };
    let rings = |row: &RowRecord| row.shadow.as_ref().map_or(0, |shadow| shadow.elements);
    if rings(current) < rings(base) {
        broken.push(format!("shadow rings {} → {}", rings(base), rings(current)));
    }
    if let (Some(b), Some(c)) = (&base.shadow, &current.shadow)
        && c.worst_rmse - b.worst_rmse > erosion::SHADOW_WORST_RMSE
    {
        let n = slidock_canon::to_js_string;
        broken.push(format!(
            "shadow.worstRmse {} → {} ({})",
            n(b.worst_rmse),
            n(c.worst_rmse),
            c.worst_el
        ));
    }
    let regions = |row: &RowRecord| {
        row.gradient
            .as_ref()
            .map_or(0, |gradient| gradient.elements)
    };
    if regions(current) < regions(base) {
        broken.push(format!(
            "gradient regions {} → {}",
            regions(base),
            regions(current)
        ));
    }
    if let (Some(b), Some(c)) = (&base.gradient, &current.gradient)
        && c.worst_rmse - b.worst_rmse > erosion::GRADIENT_WORST_RMSE
    {
        let n = slidock_canon::to_js_string;
        broken.push(format!(
            "gradient.worstRmse {} → {} ({})",
            n(b.worst_rmse),
            n(c.worst_rmse),
            c.worst_el
        ));
    }
    broken
}

/// The erosion guards `current` breaks against `base`.
pub fn erosion_guards(b: &Scores, c: &Scores) -> Vec<String> {
    let n = slidock_canon::to_js_string;
    let mut broken = Vec::new();
    if c.ssim.mean - b.ssim.mean < erosion::SSIM_MEAN {
        broken.push(format!("ssim.mean {} → {}", n(b.ssim.mean), n(c.ssim.mean)));
    }
    if c.text.recall - b.text.recall < erosion::TEXT_RECALL {
        broken.push(format!(
            "text.recall {} → {}",
            n(b.text.recall),
            n(c.text.recall)
        ));
    }
    if let (Some(bp), Some(cp)) = (b.text.p95_pm, c.text.p95_pm)
        && cp - bp > erosion::TEXT_P95_PM
    {
        broken.push(format!("text.p95Pm {} → {}", n(bp), n(cp)));
    }
    if c.color.mean - b.color.mean > erosion::COLOR_MEAN {
        broken.push(format!(
            "color.mean {} → {}",
            n(b.color.mean),
            n(c.color.mean)
        ));
    }
    broken
}

/// A row whose scores eroded within its class.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Eroded {
    pub row: RowId,
    pub guards: Vec<String>,
}

/// `regression --base <rev>`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegressionReport {
    pub regressed: Vec<RowId>,
    pub improved: Vec<RowId>,
    pub undeclined: Vec<RowId>,
    /// Base rows missing from a pair the run covered.
    pub vanished: Vec<RowId>,
    pub new: Vec<RowId>,
    pub eroded: Vec<Eroded>,
    /// Rows that regressed because more shadows went undrawn ([`shadow_losses`]), with the counts that grew; each is
    /// also in `regressed`.
    pub shadow_losses: Vec<Eroded>,
    /// Rows that regressed because more gradients were approximated ([`gradient_losses`]), with the counts that grew;
    /// each is also in `regressed`.
    pub gradient_losses: Vec<Eroded>,
    /// Base rows of pairs outside the run.
    pub not_run: u32,
    pub frontend_guards: Vec<String>,
    /// Fonts lane: the pairs whose shipped fonts grew past the allowance or lost a face's file.
    pub pair_guards: Vec<String>,
    /// Rows whose change stays out of acceptance: the baseline flags them reference or known loss.
    pub flagged: Vec<RowId>,
    /// Rows the run flags and the baseline does not: a change to review.
    pub newly_flagged: Vec<RowId>,
    /// Rows present in both.
    pub compared: u32,
    /// Stamp fields that differ beyond the compiler: non-empty means incomparable.
    pub incomparable: Vec<String>,
}

/// A record field's value as `diff` flattens it: arrays join with `,`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
pub enum FieldValue {
    Bool(bool),
    Number(f64),
    Text(String),
}

/// One moved field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct FieldChange {
    pub row: RowId,
    /// The dotted path in the record (`scores.ssim.mean`).
    pub field: String,
    /// Absent when the base record lacks the field.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<FieldValue>", optional)]
    pub base: Option<FieldValue>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<FieldValue>", optional)]
    pub current: Option<FieldValue>,
}

/// `diff --base <rev>`: every field that moved (numbers by more than 1e-4, `content` ignored).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiffReport {
    pub compared: u32,
    pub changes: Vec<FieldChange>,
    pub vanished: Vec<RowId>,
    /// Rows only the run holds.
    pub added: Vec<RowId>,
    pub not_run: u32,
    /// Stamp fields that differ beyond the compiler.
    pub stamp: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(diagnostics: &[(&str, u32)], worst: Option<f64>) -> RowRecord {
        let mut value = serde_json::json!({
            "presentation": "p", "slide": "s", "frontend": "keynote", "oracle": {"page": 1},
            "outcome": "close", "content": "a".repeat(64),
            "diagnostics": diagnostics.iter().map(|(c, n)| ((*c).to_owned(), *n)).collect::<std::collections::BTreeMap<_, _>>(),
            "scores": {"ssim": {"mean": 0.95, "p1": 0.5}, "color": {"mean": 2.0, "p99": 5.0}, "pixel": 0.9,
                "text": {"oracleWords": 0, "domWords": 0, "matched": 0, "recall": 1, "precision": 1,
                    "medianPm": null, "p95Pm": null, "heightRatioMedian": null}},
        });
        if let Some(worst) = worst {
            value["shadow"] = serde_json::json!({"elements": 2, "worstRmse": worst, "meanRmse": 0.01, "worstEl": "e"});
        }
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn shadow_rings_erode_past_the_guard() {
        assert!(eroded(&record(&[], Some(0.05)), &record(&[], Some(0.06))).is_empty());
        assert_eq!(
            eroded(&record(&[], Some(0.05)), &record(&[], Some(0.0601))),
            ["shadow.worstRmse 0.05 → 0.0601 (e)"]
        );
        // A row that gains or loses its rings is not eroded: `diff` shows it.
        assert!(eroded(&record(&[], None), &record(&[], Some(0.3))).is_empty());
    }

    #[test]
    fn more_undrawn_shadows_are_losses() {
        let base = record(&[("KN1304", 1), ("PX2101", 1)], None);
        assert!(shadow_losses(&base, &record(&[("KN1304", 1)], None)).is_empty());
        assert!(shadow_losses(&base, &record(&[("PX2101", 4)], None)).is_empty());
        assert_eq!(
            shadow_losses(&base, &record(&[("KN1304", 2), ("OD9104", 1)], None)),
            ["KN1304 1 → 2", "OD9104 0 → 1"]
        );
    }

    /// A scored row with `gradient{elements, worstRmse}` when given.
    fn gradient_record(diagnostics: &[(&str, u32)], gradient: Option<(u32, f64)>) -> RowRecord {
        let mut row = record(diagnostics, None);
        row.gradient = gradient.map(|(elements, worst)| crate::results::GradientMetrics {
            elements,
            worst_rmse: worst,
            mean_rmse: worst / 2.0,
            worst_el: "g".into(),
        });
        row
    }

    #[test]
    fn gradient_regions_erode_past_the_guard_or_when_one_goes() {
        let base = gradient_record(&[], Some((2, 0.05)));
        assert!(eroded(&base, &gradient_record(&[], Some((2, 0.06)))).is_empty());
        assert!(eroded(&base, &gradient_record(&[], Some((3, 0.01)))).is_empty());
        assert_eq!(
            eroded(&base, &gradient_record(&[], Some((2, 0.0601)))),
            ["gradient.worstRmse 0.05 → 0.0601 (g)"]
        );
        // A gradient gone from the IR without a word.
        assert_eq!(
            eroded(&base, &gradient_record(&[], Some((1, 0.05)))),
            ["gradient regions 2 → 1"]
        );
        assert_eq!(
            eroded(&base, &gradient_record(&[], None)),
            ["gradient regions 2 → 0"]
        );
        // A row that gains its regions is not eroded: `diff` shows it.
        assert!(eroded(&gradient_record(&[], None), &base).is_empty());
    }

    #[test]
    fn more_approximated_gradients_are_losses() {
        let base = gradient_record(&[("KN1301", 1), ("KN1304", 1)], None);
        assert!(gradient_losses(&base, &gradient_record(&[("KN1301", 1)], None)).is_empty());
        assert!(gradient_losses(&base, &gradient_record(&[], None)).is_empty());
        assert!(gradient_losses(&base, &gradient_record(&[("KN1304", 3)], None)).is_empty());
        assert_eq!(
            gradient_losses(
                &base,
                &gradient_record(&[("KN1301", 2), ("PX2106", 1), ("OD9103", 4)], None)
            ),
            ["KN1301 1 → 2", "PX2106 0 → 1", "OD9103 0 → 4"]
        );
    }

    #[test]
    fn movement_follows_the_rank_and_isolates_undeclines() {
        use Outcome::*;
        assert_eq!(Movement::between(Match, Match), Movement::Same);
        assert_eq!(Movement::between(Close, Match), Movement::Improved);
        assert_eq!(Movement::between(Match, Mismatch), Movement::Regressed);
        assert_eq!(Movement::between(Declined, Mismatch), Movement::Undeclined);
        assert_eq!(Movement::between(Declined, Failed), Movement::Regressed);
        assert_eq!(Movement::between(Failed, RenderError), Movement::Improved);
        assert_eq!(Movement::between(RenderError, Declined), Movement::Improved);
    }
}
