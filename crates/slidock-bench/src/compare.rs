//! Comparing two results files: `regression` decides acceptance from how each row moved,
//! `diff` lists every field that moved. Both compare only results whose stamps agree on everything but the compiler.

use std::collections::{BTreeMap, BTreeSet};

use slidock_bench_schema::compare::{
    DiffReport, Eroded, FRONTEND_SSIM_DROP, FieldChange, FieldValue, Movement, RegressionReport,
    eroded, fonts_eroded, fonts_pair_guards, gradient_losses, shadow_losses,
};
use slidock_bench_schema::results::{Outcome, Results, RowRecord};
use slidock_bench_schema::{Frontend, Lane, PairKey, PresentationId, RowId};

use crate::results::summarise;

/// The pairs both results hold rows of: the corpus stamps compared.
pub fn common_pairs(base: &Results, current: &Results) -> Vec<PairKey> {
    let pairs: BTreeSet<PairKey> = base
        .rows
        .keys()
        .filter(|id| current.rows.contains_key(*id))
        .map(RowId::pair)
        .collect();
    pairs.into_iter().collect()
}

/// The stamp fields beyond the compiler in which two results differ, over the pairs they share.
pub fn incomparable(base: &Results, current: &Results) -> Vec<String> {
    base.stamp
        .incomparable(&current.stamp, &common_pairs(base, current))
}

fn outcome(row: &RowRecord) -> Outcome {
    row.outcome.classifier().unwrap_or(Outcome::Failed)
}

/// What narrows a regression.
#[derive(Clone, Debug, Default)]
pub struct Scope {
    pub only: Option<Vec<RowId>>,
    pub presentations: Option<Vec<PresentationId>>,
}

impl Scope {
    fn contains(&self, id: &RowId) -> bool {
        self.only.as_ref().is_none_or(|only| only.contains(id))
            && self
                .presentations
                .as_ref()
                .is_none_or(|p| p.contains(id.presentation()))
    }
}

/// Compares two comparable results row by row. Fonts-lane results move by their fonts classes and guard the
/// rows' glyphs and the pairs' shipped fonts.
pub fn regression(base: &Results, current: &Results, scope: &Scope) -> RegressionReport {
    let fonts = base.stamp.lane == Lane::Fonts && current.stamp.lane == Lane::Fonts;
    let ran: BTreeSet<&PairKey> = current.stamp.corpus.keys().collect();
    let mut report = RegressionReport::default();
    let mut compared: Vec<(&RowRecord, &RowRecord)> = Vec::new();
    for (id, before) in base.rows.iter().filter(|(id, _)| scope.contains(id)) {
        let Some(after) = current.rows.get(id) else {
            if ran.contains(&id.pair()) {
                report.vanished.push(id.clone());
            } else {
                report.not_run += 1;
            }
            continue;
        };
        compared.push((before, after));
        let (movement, guards, losses, approximated) = if fonts {
            (
                Movement::between_fonts(before.outcome, after.outcome),
                fonts_eroded(before, after),
                Vec::new(),
                Vec::new(),
            )
        } else {
            let losses = shadow_losses(before, after);
            let approximated = gradient_losses(before, after);
            let movement = match Movement::between(outcome(before), outcome(after)) {
                _ if !losses.is_empty() || !approximated.is_empty() => Movement::Regressed,
                movement => movement,
            };
            (movement, eroded(before, after), losses, approximated)
        };
        let changed = movement != Movement::Same || !guards.is_empty();
        if after.flagged() && !before.flagged() {
            report.newly_flagged.push(id.clone());
        }
        if changed && before.flagged() {
            report.flagged.push(id.clone());
            continue;
        }
        match movement {
            Movement::Regressed => report.regressed.push(id.clone()),
            Movement::Improved => report.improved.push(id.clone()),
            Movement::Undeclined => report.undeclined.push(id.clone()),
            Movement::Same => {}
        }
        if !guards.is_empty() {
            report.eroded.push(Eroded {
                row: id.clone(),
                guards,
            });
        }
        if !losses.is_empty() {
            report.shadow_losses.push(Eroded {
                row: id.clone(),
                guards: losses,
            });
        }
        if !approximated.is_empty() {
            report.gradient_losses.push(Eroded {
                row: id.clone(),
                guards: approximated,
            });
        }
    }
    report.new = current
        .rows
        .keys()
        .filter(|id| scope.contains(id) && !base.rows.contains_key(*id))
        .cloned()
        .collect();
    report.compared = compared.len() as u32;
    if let (Some(base_pairs), Some(current_pairs)) = (&base.font_pairs, &current.font_pairs) {
        for (pair, before) in base_pairs {
            let in_scope = scope
                .presentations
                .as_ref()
                .is_none_or(|p| p.contains(pair.presentation()));
            if let (true, Some(after)) = (in_scope, current_pairs.get(pair)) {
                report
                    .pair_guards
                    .extend(fonts_pair_guards(&pair.to_string(), before, after));
            }
        }
    }
    let frontends: BTreeSet<Frontend> = compared.iter().map(|(row, _)| row.frontend).collect();
    let n = slidock_canon::to_js_string;
    for frontend in frontends {
        let rows: Vec<&(&RowRecord, &RowRecord)> = compared
            .iter()
            .filter(|(row, _)| row.frontend == frontend && row.reference.is_none())
            .collect();
        if rows.is_empty() {
            continue;
        }
        let before = summarise(&rows.iter().map(|(b, _)| *b).collect::<Vec<_>>()).ssim;
        let after = summarise(&rows.iter().map(|(_, c)| *c).collect::<Vec<_>>()).ssim;
        for (key, b, c) in [
            ("median", before.median, after.median),
            ("p10", before.p10, after.p10),
        ] {
            if let (Some(b), Some(c)) = (b, c)
                && b - c > FRONTEND_SSIM_DROP
            {
                report
                    .frontend_guards
                    .push(format!("{frontend} ssim {key} {} → {}", n(b), n(c)));
            }
        }
    }
    report
}

/// Whether anything moved that acceptance cares about.
pub fn regression_failed(report: &RegressionReport, accept_undeclines: &[RowId]) -> bool {
    !report.regressed.is_empty()
        || !report.improved.is_empty()
        || !report.vanished.is_empty()
        || !report.eroded.is_empty()
        || !report.newly_flagged.is_empty()
        || report
            .undeclined
            .iter()
            .any(|row| !accept_undeclines.contains(row))
        || !report.frontend_guards.is_empty()
        || !report.pair_guards.is_empty()
}

const SCORE_TOLERANCE: f64 = 1e-4;

/// A record as `path → value`, without `content`: nested objects flatten to dotted paths, arrays join with `,`.
fn fields(row: &RowRecord) -> BTreeMap<String, Option<FieldValue>> {
    fn scalar(value: &serde_json::Value) -> Option<FieldValue> {
        use serde_json::Value;
        match value {
            Value::Null => None,
            Value::Bool(b) => Some(FieldValue::Bool(*b)),
            Value::Number(n) => n.as_f64().map(FieldValue::Number),
            Value::String(s) => Some(FieldValue::Text(s.clone())),
            Value::Array(items) => Some(FieldValue::Text(
                items
                    .iter()
                    .map(|item| match item {
                        Value::String(s) => s.clone(),
                        Value::Null => String::new(),
                        other => other.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(","),
            )),
            Value::Object(_) => None,
        }
    }
    fn visit(
        value: &serde_json::Value,
        path: &str,
        out: &mut BTreeMap<String, Option<FieldValue>>,
    ) {
        if let serde_json::Value::Object(map) = value {
            for (key, item) in map {
                let inner = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                visit(item, &inner, out);
            }
        } else {
            out.insert(path.to_owned(), scalar(value));
        }
    }
    let mut out = BTreeMap::new();
    if let Ok(serde_json::Value::Object(mut map)) = serde_json::to_value(row) {
        map.remove("content");
        visit(&serde_json::Value::Object(map), "", &mut out);
    }
    out
}

fn moved(a: Option<&FieldValue>, b: Option<&FieldValue>) -> bool {
    match (a, b) {
        (Some(FieldValue::Number(x)), Some(FieldValue::Number(y))) => {
            (x - y).abs() > SCORE_TOLERANCE
        }
        _ => a != b,
    }
}

/// Every field that moved between two results, and the rows on one side only.
pub fn diff_results(base: &Results, current: &Results) -> DiffReport {
    let mut report = DiffReport::default();
    for (id, before) in &base.rows {
        let Some(after) = current.rows.get(id) else {
            continue;
        };
        report.compared += 1;
        let a = fields(before);
        let b = fields(after);
        let mut names: Vec<&String> = a.keys().chain(b.keys()).collect();
        names.sort_by(|p, q| slidock_canon::utf16::cmp(p, q));
        names.dedup();
        for name in names {
            let x = a.get(name).and_then(Option::as_ref);
            let y = b.get(name).and_then(Option::as_ref);
            if moved(x, y) {
                report.changes.push(FieldChange {
                    row: id.clone(),
                    field: name.clone(),
                    base: x.cloned(),
                    current: y.cloned(),
                });
            }
        }
    }
    let ran: BTreeSet<&PairKey> = current.stamp.corpus.keys().collect();
    for id in base
        .rows
        .keys()
        .filter(|id| !current.rows.contains_key(*id))
    {
        if ran.contains(&id.pair()) {
            report.vanished.push(id.clone());
        } else {
            report.not_run += 1;
        }
    }
    report.added = current
        .rows
        .keys()
        .filter(|id| !base.rows.contains_key(*id))
        .cloned()
        .collect();
    report.stamp = incomparable(base, current);
    report
}

/// A field's value as `diff` prints it.
pub fn field_text(value: Option<&FieldValue>) -> String {
    match value {
        None => "–".to_owned(),
        Some(FieldValue::Bool(b)) => b.to_string(),
        Some(FieldValue::Number(n)) => slidock_canon::to_js_string(*n),
        Some(FieldValue::Text(t)) => t.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{record, results};

    #[test]
    fn regression_buckets_each_movement() {
        let base = results(vec![
            record("a", Outcome::Match, Some(0.99)),
            record("b", Outcome::Close, Some(0.95)),
            record("c", Outcome::Declined, None),
            record("d", Outcome::Match, Some(0.99)),
            record("e", Outcome::Match, Some(0.99)),
        ]);
        let current = results(vec![
            record("a", Outcome::Close, Some(0.95)),
            record("b", Outcome::Match, Some(0.99)),
            record("c", Outcome::Mismatch, Some(0.5)),
            record("e", Outcome::Match, Some(0.98)),
            record("f", Outcome::Match, Some(0.99)),
        ]);
        let report = regression(&base, &current, &Scope::default());
        let ids = |rows: &[RowId]| {
            rows.iter()
                .map(|r| r.slide().as_str().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&report.regressed), ["a"]);
        assert_eq!(ids(&report.improved), ["b"]);
        assert_eq!(ids(&report.undeclined), ["c"]);
        assert_eq!(ids(&report.vanished), ["d"]);
        assert_eq!(ids(&report.new), ["f"]);
        assert_eq!(report.eroded.len(), 1);
        assert_eq!(report.eroded[0].guards, ["ssim.mean 0.99 → 0.98"]);
        assert_eq!(report.compared, 4);
        assert!(regression_failed(&report, &[]));
    }

    /// A scored row with `shadow.worstRmse` and a count of `code`.
    fn shadow_row(slide: &str, outcome: Outcome, worst: f64, code: (&str, u32)) -> RowRecord {
        use slidock_bench_schema::results::ShadowMetrics;
        let mut row = record(slide, outcome, Some(0.95));
        row.shadow = Some(ShadowMetrics {
            elements: 1,
            worst_rmse: worst,
            mean_rmse: worst,
            worst_el: "el".into(),
        });
        if code.1 > 0 {
            row.diagnostics.insert(code.0.to_owned(), code.1);
        }
        row
    }

    #[test]
    fn shadow_rings_erode_and_undrawn_shadows_regress() {
        let base = results(vec![
            shadow_row("a", Outcome::Close, 0.02, ("KN1304", 0)),
            shadow_row("b", Outcome::Close, 0.02, ("PX2111", 1)),
            shadow_row("c", Outcome::Close, 0.02, ("OD9104", 1)),
            shadow_row("d", Outcome::Close, 0.02, ("PX2101", 1)),
            shadow_row("e", Outcome::Close, 0.02, ("KN1304", 1)),
            shadow_row("f", Outcome::Close, 0.02, ("KN1304", 0)),
        ]);
        let current = results(vec![
            // Eroded: the worst ring grew past 0.01.
            shadow_row("a", Outcome::Close, 0.0301, ("KN1304", 0)),
            // Regressed although its class improved: one more shadow went undrawn.
            shadow_row("b", Outcome::Match, 0.02, ("PX2111", 2)),
            // Within the guards.
            shadow_row("c", Outcome::Close, 0.03, ("OD9104", 1)),
            // Not a shadow code.
            shadow_row("d", Outcome::Close, 0.02, ("PX2101", 3)),
            // Drawn now: fewer undrawn shadows is no regression.
            shadow_row("e", Outcome::Close, 0.01, ("KN1304", 0)),
            // Eroded: its shadow left the IR without a word, so its ring went with it.
            record("f", Outcome::Close, Some(0.95)),
        ]);
        let report = regression(&base, &current, &Scope::default());
        let ids = |rows: &[RowId]| {
            rows.iter()
                .map(|r| r.slide().as_str().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&report.regressed), ["b"]);
        assert!(report.improved.is_empty());
        assert_eq!(report.shadow_losses.len(), 1);
        assert_eq!(report.shadow_losses[0].guards, ["PX2111 1 → 2"]);
        assert_eq!(report.eroded.len(), 2);
        assert_eq!(ids(&[report.eroded[0].row.clone()]), ["a"]);
        assert_eq!(
            report.eroded[0].guards,
            ["shadow.worstRmse 0.02 → 0.0301 (el)"]
        );
        assert_eq!(ids(&[report.eroded[1].row.clone()]), ["f"]);
        assert_eq!(report.eroded[1].guards, ["shadow rings 1 → 0"]);
        assert!(regression_failed(&report, &[]));
    }

    /// A scored row with `gradient{elements, worstRmse}` and a count of `code`.
    fn gradient_row(
        slide: &str,
        outcome: Outcome,
        gradient: Option<(u32, f64)>,
        code: (&str, u32),
    ) -> RowRecord {
        use slidock_bench_schema::results::GradientMetrics;
        let mut row = record(slide, outcome, Some(0.95));
        row.gradient = gradient.map(|(elements, worst)| GradientMetrics {
            elements,
            worst_rmse: worst,
            mean_rmse: worst,
            worst_el: "el".into(),
        });
        if code.1 > 0 {
            row.diagnostics.insert(code.0.to_owned(), code.1);
        }
        row
    }

    #[test]
    fn gradient_regions_erode_and_approximated_gradients_regress() {
        let base = results(vec![
            gradient_row("a", Outcome::Close, Some((1, 0.02)), ("KN1301", 0)),
            gradient_row("b", Outcome::Close, Some((1, 0.02)), ("PX2106", 1)),
            gradient_row("c", Outcome::Close, Some((1, 0.02)), ("OD9103", 1)),
            gradient_row("d", Outcome::Close, Some((1, 0.02)), ("PX2101", 1)),
            gradient_row("e", Outcome::Close, Some((1, 0.02)), ("KN1301", 1)),
            gradient_row("f", Outcome::Close, Some((2, 0.02)), ("KN1301", 0)),
        ]);
        let current = results(vec![
            // Eroded: the worst region grew past 0.01.
            gradient_row("a", Outcome::Close, Some((1, 0.0301)), ("KN1301", 0)),
            // Regressed although its class improved: one more gradient was approximated.
            gradient_row("b", Outcome::Match, Some((1, 0.02)), ("PX2106", 2)),
            // Within the guards.
            gradient_row("c", Outcome::Close, Some((1, 0.03)), ("OD9103", 1)),
            // Not a gradient code.
            gradient_row("d", Outcome::Close, Some((1, 0.02)), ("PX2101", 3)),
            // Drawn as designed now: fewer approximations is no regression.
            gradient_row("e", Outcome::Close, Some((1, 0.01)), ("KN1301", 0)),
            // Eroded: a gradient left the IR without a word, so its region went with it.
            gradient_row("f", Outcome::Close, Some((1, 0.02)), ("KN1301", 0)),
        ]);
        let report = regression(&base, &current, &Scope::default());
        let ids = |rows: &[RowId]| {
            rows.iter()
                .map(|r| r.slide().as_str().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&report.regressed), ["b"]);
        assert!(report.improved.is_empty());
        assert!(report.shadow_losses.is_empty());
        assert_eq!(report.gradient_losses.len(), 1);
        assert_eq!(report.gradient_losses[0].guards, ["PX2106 1 → 2"]);
        assert_eq!(report.eroded.len(), 2);
        assert_eq!(ids(&[report.eroded[0].row.clone()]), ["a"]);
        assert_eq!(
            report.eroded[0].guards,
            ["gradient.worstRmse 0.02 → 0.0301 (el)"]
        );
        assert_eq!(ids(&[report.eroded[1].row.clone()]), ["f"]);
        assert_eq!(report.eroded[1].guards, ["gradient regions 2 → 1"]);
        assert!(regression_failed(&report, &[]));
    }

    /// A fonts-lane row drawn with `exact` and `fallback` glyphs out of 100.
    fn fonts_row(slide: &str, exact: u32, fallback: u32) -> RowRecord {
        use slidock_bench_schema::lanes::{FontsRowMetrics, GlyphCounts};
        let mut row = record(slide, Outcome::Match, None);
        let total = GlyphCounts {
            glyphs: 100,
            exact,
            substitute: 100 - exact - fallback,
            fallback,
            coverage_gaps: fallback,
            sources: BTreeMap::new(),
        };
        let metrics = FontsRowMetrics {
            exact_share: crate::fonts_lane::attribution::share(exact, 100),
            substitute_share: crate::fonts_lane::attribution::share(total.substitute, 100),
            fallback_share: crate::fonts_lane::attribution::share(fallback, 100),
            total,
            fallback_fonts: Vec::new(),
            families: BTreeMap::new(),
            equivalence: None,
        };
        row.outcome = crate::fonts_lane::fonts_class(&metrics).into();
        row.fonts = Some(metrics);
        row
    }

    fn fonts_results(rows: Vec<RowRecord>, font_bytes: u64) -> Results {
        use slidock_bench_schema::lanes::FontsPairReport;
        let mut results = results(rows);
        results.stamp.lane = Lane::Fonts;
        let pair = PairKey::new(PresentationId::new("demo-basics").unwrap(), Frontend::Pptx);
        results.font_pairs = Some(
            [(
                pair,
                FontsPairReport {
                    font_bytes,
                    ..FontsPairReport::default()
                },
            )]
            .into_iter()
            .collect(),
        );
        results
    }

    #[test]
    fn fonts_rows_move_by_their_class_and_guard_their_glyphs_and_bytes() {
        let base = fonts_results(
            vec![
                fonts_row("a", 100, 0),
                fonts_row("b", 90, 2),
                fonts_row("c", 80, 0),
                fonts_row("d", 100, 0),
            ],
            10_000,
        );
        let current = fonts_results(
            vec![
                fonts_row("a", 97, 3),
                fonts_row("b", 98, 0),
                fonts_row("c", 70, 0),
                fonts_row("d", 100, 0),
            ],
            11_001,
        );
        let report = regression(&base, &current, &Scope::default());
        let ids = |rows: &[RowId]| {
            rows.iter()
                .map(|r| r.slide().as_str().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&report.regressed), ["a"]);
        assert_eq!(ids(&report.improved), ["b"]);
        assert_eq!(report.eroded.len(), 1);
        assert_eq!(report.eroded[0].guards, ["exactShare 0.8 → 0.7"]);
        assert_eq!(
            report.pair_guards,
            ["demo-basics:pptx fontBytes 10000 → 11001"]
        );
        assert!(regression_failed(&report, &[]));
        let same = regression(
            &base,
            &fonts_results(base.rows.values().cloned().collect(), 11_000),
            &Scope::default(),
        );
        assert!(!regression_failed(&same, &[]), "{same:?}");
    }

    #[test]
    fn identical_results_pass_and_flagged_baseline_rows_stay_out() {
        let mut flagged = record("a", Outcome::Match, Some(0.99));
        flagged.known_loss = Some(vec![slidock_bench_schema::corpus::LossKind::Source]);
        let base = results(vec![flagged, record("b", Outcome::Match, Some(0.99))]);
        let same = regression(&base, &base, &Scope::default());
        assert!(!regression_failed(&same, &[]));
        assert_eq!(same.compared, 2);
        let mut moved = base.clone();
        if let Some(row) = moved.rows.values_mut().next() {
            row.outcome = Outcome::Mismatch.into();
        }
        let report = regression(&base, &moved, &Scope::default());
        assert_eq!(report.flagged.len(), 1);
        assert!(report.regressed.is_empty());
        assert!(!regression_failed(&report, &[]));
    }

    #[test]
    fn a_row_the_run_newly_flags_fails_the_check() {
        let base = results(vec![record("a", Outcome::Match, Some(0.99))]);
        let mut current = base.clone();
        if let Some(row) = current.rows.values_mut().next() {
            row.known_loss = Some(vec![slidock_bench_schema::corpus::LossKind::Source]);
        }
        let report = regression(&base, &current, &Scope::default());
        assert_eq!(report.newly_flagged.len(), 1);
        assert!(regression_failed(&report, &[]));
    }

    #[test]
    fn a_frontend_ssim_drop_trips_the_guard() {
        let base = results(vec![
            record("a", Outcome::Match, Some(0.990)),
            record("b", Outcome::Match, Some(0.990)),
        ]);
        let current = results(vec![
            record("a", Outcome::Match, Some(0.987)),
            record("b", Outcome::Match, Some(0.987)),
        ]);
        let report = regression(&base, &current, &Scope::default());
        assert!(report.eroded.is_empty());
        assert_eq!(
            report.frontend_guards,
            [
                "pptx ssim median 0.99 → 0.987",
                "pptx ssim p10 0.99 → 0.987"
            ]
        );
        let close = results(vec![
            record("a", Outcome::Match, Some(0.989)),
            record("b", Outcome::Match, Some(0.989)),
        ]);
        assert!(
            regression(&base, &close, &Scope::default())
                .frontend_guards
                .is_empty()
        );
    }

    #[test]
    fn diff_lists_moved_fields_beyond_the_tolerance() {
        let base = results(vec![record("a", Outcome::Match, Some(0.99))]);
        let mut current = base.clone();
        if let Some(row) = current.rows.values_mut().next() {
            if let Some(scores) = row.scores.as_mut() {
                scores.ssim.mean = 0.99005;
                scores.ssim.p1 = 0.5;
            }
            row.content = slidock_bench_schema::Sha256::of("other");
        }
        let diff = diff_results(&base, &current);
        assert_eq!(diff.compared, 1);
        let moved: Vec<&str> = diff.changes.iter().map(|c| c.field.as_str()).collect();
        assert_eq!(moved, ["scores.ssim.p1"]);
        assert!(diff.stamp.is_empty());
        assert!(diff_results(&base, &base).changes.is_empty());
    }
}
