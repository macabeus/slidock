//! The cross-field rules of results, runs and parity, each with the input that breaks it.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};
use slidock_bench_schema::compare::{self, Movement};
use slidock_bench_schema::parity::{
    AllowEntry, Allowlist, CompilerParity, CompilerRow, GoldenIdentity, ParityReference,
    ParityReport, ParityStatus, ReferenceRow, Verdict, first_difference,
};
use slidock_bench_schema::results::{
    Decline, Outcome, Reason, Results, RowOutcome, RowRecord, Scores,
};
use slidock_bench_schema::runs::RunsIndex;
use slidock_bench_schema::stamp::Stamp;
use slidock_bench_schema::{Error, Lane, RowId, Sha256};

fn data(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(name),
    )
    .unwrap()
}

fn results_value(name: &str) -> Value {
    serde_json::from_str(&data(name)).unwrap()
}

fn invalid(value: &Value) -> String {
    match Results::parse(&value.to_string()) {
        Err(Error::Invalid(issues)) => issues.to_string(),
        other => panic!("expected a rule to break, got {other:?}"),
    }
}

fn first_row(value: &mut Value) -> (&String, &mut Value) {
    value["rows"]
        .as_object_mut()
        .unwrap()
        .iter_mut()
        .next()
        .unwrap()
}

#[test]
fn a_row_sits_under_its_own_id() {
    let mut value = results_value("results.json");
    let rows = value["rows"].as_object_mut().unwrap();
    let (id, row) = rows
        .iter()
        .next()
        .map(|(k, v)| (k.clone(), v.clone()))
        .unwrap();
    rows.remove(&id);
    rows.insert("demo-basics:zz-1:odp".into(), row);
    assert!(invalid(&value).contains("the record is row"));
}

#[test]
fn a_reason_matches_its_outcome() {
    let mut value = results_value("results.json");
    let (_, row) = first_row(&mut value);
    row["reason"] = json!("render-crash");
    row["outcome"] = json!("failed");
    assert!(invalid(&value).contains("has reason"));
}

#[test]
fn lane_metrics_stay_in_their_lane() {
    let mut value = results_value("results.json");
    let (_, row) = first_row(&mut value);
    row["host"] =
        json!({ "status": "identical", "standalone": "0".repeat(64), "mounted": "0".repeat(64) });
    assert!(invalid(&value).contains("host metrics in a Static results file"));

    let mut value = results_value("results.json");
    value["fontPairs"] = json!({});
    assert!(invalid(&value).contains("fontPairs in a Static"));
}

#[test]
fn shadow_metrics_sit_on_scored_static_rows_with_a_ring() {
    let shadow = json!({ "elements": 2, "worstRmse": 0.05, "meanRmse": 0.03, "worstEl": "kn-1" });
    let mut value = results_value("results.json");
    first_row(&mut value).1["shadow"] = shadow.clone();
    Results::parse(&value.to_string()).unwrap();

    let mut value = results_value("results.json");
    value["stamp"]["lane"] = json!("fonts");
    first_row(&mut value).1["shadow"] = shadow.clone();
    assert!(invalid(&value).contains("shadow metrics in a Fonts results file"));

    let mut value = results_value("results.json");
    let (_, row) = first_row(&mut value);
    row["shadow"] = shadow.clone();
    row.as_object_mut().unwrap().remove("scores");
    assert!(invalid(&value).contains("shadow metrics on an unscored row"));

    let mut value = results_value("results.json");
    first_row(&mut value).1["shadow"] =
        json!({ "elements": 0, "worstRmse": 0, "meanRmse": 0, "worstEl": "" });
    assert!(invalid(&value).contains("shadow metrics without a ring"));
}

#[test]
fn gradient_metrics_sit_on_scored_static_rows_with_a_region() {
    let gradient = json!({ "elements": 2, "worstRmse": 0.05, "meanRmse": 0.03, "worstEl": "bg" });
    let mut value = results_value("results.json");
    first_row(&mut value).1["gradient"] = gradient.clone();
    Results::parse(&value.to_string()).unwrap();

    let mut value = results_value("results.json");
    value["stamp"]["lane"] = json!("fonts");
    first_row(&mut value).1["gradient"] = gradient.clone();
    assert!(invalid(&value).contains("gradient metrics in a Fonts results file"));

    let mut value = results_value("results.json");
    let (_, row) = first_row(&mut value);
    row["gradient"] = gradient.clone();
    row.as_object_mut().unwrap().remove("scores");
    assert!(invalid(&value).contains("gradient metrics on an unscored row"));

    let mut value = results_value("results.json");
    first_row(&mut value).1["gradient"] =
        json!({ "elements": 0, "worstRmse": 0, "meanRmse": 0, "worstEl": "" });
    assert!(invalid(&value).contains("gradient metrics without a region"));
}

#[test]
fn only_transition_rows_take_a_motion_class() {
    let mut value = results_value("results.json");
    let (_, row) = first_row(&mut value);
    row["outcome"] = json!("motion-match");
    assert!(invalid(&value).contains("motion class"));

    let mut value = results_value("motion-results.json");
    let rows = value["rows"].as_object_mut().unwrap();
    let transition = rows.keys().find(|k| k.contains("#t@")).unwrap().clone();
    rows[&transition]["outcome"] = json!("match");
    assert!(invalid(&value).contains("motion class"));
}

#[test]
fn stage_rows_take_a_motion_class_only_where_no_oracle_scores_them() {
    // The synthetic motion run has stage rows of both kinds: scored against a stages oracle, and checked alone.
    let results = Results::parse(&data("motion-results.json")).unwrap();
    let stages = |motion: bool| {
        results
            .rows
            .iter()
            .filter(|(id, row)| {
                id.to_string().contains("#s")
                    && matches!(row.outcome, RowOutcome::Motion(_)) == motion
            })
            .count()
    };
    assert!(stages(true) > 0 && stages(false) > 0);
    // A static row never takes one.
    let mut value = results_value("motion-results.json");
    let rows = value["rows"].as_object_mut().unwrap();
    let stage = rows.keys().find(|k| k.contains("#s")).unwrap().clone();
    let mut row = rows.remove(&stage).unwrap();
    row.as_object_mut().unwrap().remove("stage");
    row["outcome"] = json!("motion-match");
    let id = stage.split('#').next().unwrap().to_owned();
    rows.insert(id, row);
    assert!(invalid(&value).contains("static rows never do"));
}

#[test]
fn stage_rows_motion_pairs_and_positions_stay_in_their_lanes() {
    let motion = results_value("motion-results.json");
    // The shell lane keeps stage rows (each position of the unzipped deck), the fonts lane does not.
    for (lane, allowed) in [("shell", true), ("fonts", false)] {
        let mut value = motion.clone();
        value["stamp"]["lane"] = json!(lane);
        for key in ["timelines", "motionPairs", "positions"] {
            value.as_object_mut().unwrap().remove(key);
        }
        for row in value["rows"].as_object_mut().unwrap().values_mut() {
            row.as_object_mut().unwrap().remove("motion");
        }
        value["rows"]
            .as_object_mut()
            .unwrap()
            .retain(|k, _| !k.contains("#t@"));
        let parsed = Results::parse(&value.to_string());
        if allowed {
            parsed.unwrap();
        } else {
            assert!(invalid(&value).contains("a stage row in a Fonts"));
        }
    }
    for key in ["motionPairs", "positions"] {
        let mut value = results_value("results.json");
        value[key] = json!({});
        assert!(invalid(&value).contains(&format!("{key} in a Static")));
    }
    let mut value = motion.clone();
    let id = value["rows"]
        .as_object()
        .unwrap()
        .keys()
        .find(|k| k.contains("#t@"))
        .unwrap()
        .clone();
    value["positions"][id] = json!("0".repeat(64));
    assert!(invalid(&value).contains("is not a stage"));
}

#[test]
fn results_refuse_unknown_keys_and_numbers_off_the_wire() {
    let mut value = results_value("results.json");
    let (_, row) = first_row(&mut value);
    row["extra"] = json!(1);
    assert!(matches!(
        Results::parse(&value.to_string()),
        Err(Error::Parse { .. })
    ));

    let mut value = results_value("results.json");
    value["schema"] = json!("presentation-bench-results@1");
    let error = Results::parse(&value.to_string()).unwrap_err().to_string();
    assert!(error.starts_with("schema:"), "{error}");
}

#[test]
fn a_row_is_a_stage_or_a_transition_frame() {
    let mut value = results_value("motion-results.json");
    let rows = value["rows"].as_object_mut().unwrap();
    let stage = rows.keys().find(|k| k.contains("#s")).unwrap().clone();
    rows[&stage]["transitionPm"] = json!(250);
    assert!(invalid(&value).contains("not both"));
}

#[test]
fn a_record_id_carries_its_step() {
    let results = Results::parse(&data("motion-results.json")).unwrap();
    for (id, row) in &results.rows {
        assert_eq!(&row.id(), id);
    }
    assert!(
        results
            .rows
            .values()
            .any(|row| matches!(row.outcome, RowOutcome::Motion(_)))
    );
}

#[test]
fn runs_are_unique_and_oldest_first() {
    let text = data("runs.json");
    let mut value: Value = serde_json::from_str(&text).unwrap();
    let runs = value["runs"].as_array_mut().unwrap();
    runs.swap(0, 1);
    let error = RunsIndex::parse(&value.to_string())
        .unwrap_err()
        .to_string();
    assert!(error.contains("oldest first"), "{error}");

    let mut value: Value = serde_json::from_str(&text).unwrap();
    let runs = value["runs"].as_array_mut().unwrap();
    let first = runs[0].clone();
    runs.insert(1, first);
    let error = RunsIndex::parse(&value.to_string())
        .unwrap_err()
        .to_string();
    assert!(error.contains("listed twice"), "{error}");

    let mut value: Value = serde_json::from_str(&text).unwrap();
    let runs = value["runs"].as_array_mut().unwrap();
    let n = runs.len();
    for (i, run) in runs.iter_mut().enumerate() {
        run["recordedAt"] = json!(format!("2026-10-30T12:00:{i:02}Z"));
    }
    runs[n - 2]["recordedAt"] = json!("2026-10-30T12:00:50Z");
    runs[n - 1]["recordedAt"] = json!("2026-10-30T12:00:50.5Z");
    RunsIndex::parse(&value.to_string()).unwrap();
    let runs = value["runs"].as_array_mut().unwrap();
    runs[n - 2]["recordedAt"] = json!("2026-10-30T12:00:50.5Z");
    runs[n - 1]["recordedAt"] = json!("2026-10-30T12:00:50Z");
    let error = RunsIndex::parse(&value.to_string())
        .unwrap_err()
        .to_string();
    assert!(error.contains("oldest first"), "{error}");

    let mut value: Value = serde_json::from_str(&text).unwrap();
    value["runs"][0]["recordedAt"] = json!("yesterday");
    let error = RunsIndex::parse(&value.to_string())
        .unwrap_err()
        .to_string();
    assert!(error.contains("RFC 3339"), "{error}");
}

fn rows() -> BTreeMap<RowId, RowRecord> {
    Results::parse(&data("results.json")).unwrap().rows
}

fn reference_of(row: &RowRecord) -> ReferenceRow {
    ReferenceRow {
        html: row.html.clone(),
        render: row.render.clone(),
        outcome: row.outcome.classifier().unwrap(),
        reason: row.reason,
        code: row.code.clone(),
        data: row.data.clone(),
        declined: row.declined.clone(),
        scores: row.scores,
        diagnostics: row.diagnostics.clone(),
        timeline: None,
        transition: None,
    }
}

fn scored(outcome: Outcome) -> RowRecord {
    rows()
        .into_values()
        .find(|row| row.outcome == outcome.into() && row.scores.is_some())
        .unwrap()
}

#[test]
fn parity_statuses_follow_render_html_and_class() {
    let ours = scored(Outcome::Close);
    let reference = reference_of(&ours);
    assert_eq!(
        ParityStatus::of(&reference, Some(&ours)).0,
        ParityStatus::Identical
    );
    assert_eq!(ParityStatus::of(&reference, None).0, ParityStatus::Missing);

    let mut html = ours.clone();
    html.html = Some(Sha256::of("other"));
    assert_eq!(
        ParityStatus::of(&reference, Some(&html)).0,
        ParityStatus::PixelIdentical
    );

    let mut drift = ours.clone();
    drift.render = Some(Sha256::of("other render"));
    assert_eq!(
        ParityStatus::of(&reference, Some(&drift)).0,
        ParityStatus::DriftSameClass
    );

    let mut better = drift.clone();
    better.outcome = Outcome::Match.into();
    assert_eq!(
        ParityStatus::of(&reference, Some(&better)).0,
        ParityStatus::DriftBetter
    );

    let mut worse = drift.clone();
    worse.outcome = Outcome::Mismatch.into();
    assert_eq!(
        ParityStatus::of(&reference, Some(&worse)).0,
        ParityStatus::Regressed
    );

    let mut eroded = drift.clone();
    let scores: &mut Scores = eroded.scores.as_mut().unwrap();
    scores.ssim.mean -= 0.0051;
    let (status, guards) = ParityStatus::of(&reference, Some(&eroded));
    assert_eq!(status, ParityStatus::Regressed);
    assert_eq!(guards.len(), 1, "{guards:?}");
    assert!(guards[0].starts_with("ssim.mean "), "{guards:?}");

    let mut declined = reference.clone();
    declined.outcome = Outcome::Declined;
    declined.render = None;
    assert_eq!(
        ParityStatus::of(&declined, Some(&ours)).0,
        ParityStatus::New
    );
}

fn unscored(outcome: Outcome) -> RowRecord {
    rows()
        .into_values()
        .find(|row| row.outcome == outcome.into() && row.render.is_none())
        .unwrap()
}

#[test]
fn unscored_rows_are_identical_only_with_the_same_class_and_explanation() {
    let failed = unscored(Outcome::Failed);
    let declined = unscored(Outcome::Declined);
    assert!(failed.reason.is_some() && failed.code.is_some());
    assert!(declined.declined.is_some());
    for row in [&failed, &declined] {
        let reference = reference_of(row);
        assert_eq!(
            ParityStatus::of(&reference, Some(row)).0,
            ParityStatus::Identical,
            "{}",
            row.id()
        );
    }

    let decline = |code: &str, feature: Option<&str>, via: Option<&str>| Decline {
        code: code.into(),
        feature: feature.map(Into::into),
        via: via.map(Into::into),
    };
    let original = declined.declined.clone().unwrap();
    let feature = original.feature.as_deref();
    let variants: Vec<(&str, RowRecord, RowRecord)> = vec![
        (
            "code",
            failed.clone(),
            RowRecord {
                code: Some("IR0001".into()),
                ..failed.clone()
            },
        ),
        (
            "reason",
            failed.clone(),
            RowRecord {
                reason: Some(Reason::Lint),
                ..failed.clone()
            },
        ),
        (
            "data",
            failed.clone(),
            RowRecord {
                data: Some(BTreeMap::from([("reason".into(), "memory".into())])),
                ..failed.clone()
            },
        ),
        (
            "outcome",
            declined.clone(),
            RowRecord {
                outcome: Outcome::Failed.into(),
                ..declined.clone()
            },
        ),
        (
            "declined.code",
            declined.clone(),
            RowRecord {
                declined: Some(decline("PX4999", feature, None)),
                ..declined.clone()
            },
        ),
        (
            "declined.feature",
            declined.clone(),
            RowRecord {
                declined: Some(decline(&original.code, Some("element.table"), None)),
                ..declined.clone()
            },
        ),
        (
            "declined.via",
            declined.clone(),
            RowRecord {
                declined: Some(decline(&original.code, feature, Some("presentation"))),
                ..declined.clone()
            },
        ),
    ];
    for (field, base, other) in variants {
        let reference = reference_of(&base);
        let id = base.id();
        let (status, _) = ParityStatus::of(&reference, Some(&other));
        assert_ne!(status, ParityStatus::Identical, "{field}");
        let row = CompilerRow::compare(&id, &reference, Some(&other), &Allowlist::default());
        assert!(row.fails(true), "{field}");
    }
}

#[test]
fn new_is_a_scored_row_the_reference_declined_or_failed_not_one_it_could_not_render() {
    let ours = scored(Outcome::Close);
    for (outcome, status) in [
        (Outcome::Declined, ParityStatus::New),
        (Outcome::Failed, ParityStatus::New),
        (Outcome::RenderError, ParityStatus::DriftBetter),
    ] {
        let mut reference = reference_of(&ours);
        reference.outcome = outcome;
        reference.render = None;
        reference.scores = None;
        assert_eq!(
            ParityStatus::of(&reference, Some(&ours)).0,
            status,
            "{outcome:?}"
        );
    }
}

#[test]
fn erosion_guards_hold_their_bounds() {
    assert_eq!(compare::erosion::SSIM_MEAN, -0.005);
    assert_eq!(compare::erosion::TEXT_RECALL, -0.01);
    assert_eq!(compare::erosion::TEXT_P95_PM, 1.4);
    assert_eq!(compare::erosion::COLOR_MEAN, 1.0);
    assert_eq!(compare::FRONTEND_SSIM_DROP, 0.002);

    let base = scored(Outcome::Match);
    type Nudge = fn(&mut Scores, f64);
    let guards: [(&str, Nudge, f64, f64); 4] = [
        ("ssim.mean", |s, d| s.ssim.mean += d, -0.004, -0.006),
        ("text.recall", |s, d| s.text.recall += d, -0.009, -0.011),
        (
            "text.p95Pm",
            |s, d| s.text.p95_pm = s.text.p95_pm.map(|p| p + d),
            1.3,
            1.5,
        ),
        ("color.mean", |s, d| s.color.mean += d, 0.99, 1.01),
    ];
    assert!(base.scores.unwrap().text.p95_pm.is_some());
    for (name, nudge, inside, outside) in guards {
        let mut current = base.clone();
        nudge(current.scores.as_mut().unwrap(), inside);
        assert!(
            compare::eroded(&base, &current).is_empty(),
            "{name} {inside}"
        );
        let mut current = base.clone();
        nudge(current.scores.as_mut().unwrap(), outside);
        let broken = compare::eroded(&base, &current);
        assert_eq!(broken.len(), 1, "{name}: {broken:?}");
        assert!(
            broken[0].starts_with(&format!("{name} ")),
            "{name}: {broken:?}"
        );
    }
    assert_eq!(
        Movement::between(Outcome::Match, Outcome::Close),
        Movement::Regressed
    );
}

/// A reference keyed under the stamp of `results.json`, holding `rows`.
fn reference_for(rows: &BTreeMap<RowId, RowRecord>) -> (ParityReference, Stamp) {
    let stamp = Results::parse(&data("results.json")).unwrap().stamp;
    let metrics = &stamp.metrics;
    let reference = ParityReference {
        schema: Default::default(),
        golden: GoldenIdentity {
            commit: "c".into(),
            manifest: Sha256::of("golden manifest"),
        },
        harness: stamp.harness.clone(),
        renderer_recipe: stamp.renderer.recipe.clone(),
        font_set_id: stamp.fonts.font_set_id.clone(),
        metrics: BTreeMap::from([
            ("color".into(), metrics.color.clone()),
            ("pixel".into(), metrics.pixel.clone()),
            ("ssim".into(), metrics.ssim.clone()),
            ("text".into(), metrics.text.clone()),
        ]),
        classifier: stamp.classifier.clone(),
        rows: rows
            .iter()
            .map(|(id, row)| (id.clone(), reference_of(row)))
            .collect(),
    };
    (reference, stamp)
}

fn compare_all(
    reference: &ParityReference,
    ours: &BTreeMap<RowId, RowRecord>,
    allowlist: &Allowlist,
) -> BTreeMap<RowId, CompilerRow> {
    reference
        .rows
        .iter()
        .map(|(id, row)| {
            (
                id.clone(),
                CompilerRow::compare(id, row, ours.get(id), allowlist),
            )
        })
        .collect()
}

#[test]
fn parity_check_fails_on_unreviewed_rows_and_passes_listed_ones() {
    let all = rows();
    let (id, ours) = all.iter().find(|(_, row)| row.scores.is_some()).unwrap();
    let only = BTreeMap::from([(id.clone(), ours.clone())]);
    let (reference, stamp) = reference_for(&only);
    let mut drift = ours.clone();
    drift.render = Some(Sha256::of("drift"));
    let drifted = BTreeMap::from([(id.clone(), drift.clone())]);

    let empty = Allowlist::default();
    let parity = CompilerParity::build(compare_all(&reference, &drifted, &empty), &drifted, false);
    assert_eq!(parity.failing, std::slice::from_ref(id));
    assert_eq!(Verdict::check(&reference, &stamp, &parity), Verdict::Fail);
    assert_eq!(Verdict::check(&reference, &stamp, &parity).exit_code(), 1);

    let entry = |status| AllowEntry {
        row: id.clone(),
        reason: "reviewed".into(),
        path: "$.slides".into(),
        review_by: "2027-01-01".into(),
        status,
    };
    for (allowed, fails) in [
        (None, false),
        (Some(ParityStatus::DriftSameClass), false),
        (Some(ParityStatus::Regressed), true),
    ] {
        let list = Allowlist(vec![entry(allowed)]);
        let row = CompilerRow::compare(id, &reference.rows[id], Some(&drift), &list);
        assert_eq!(row.fails(false), fails, "{allowed:?}");
        assert!(row.fails(true));
    }

    let identical = CompilerParity::build(compare_all(&reference, &only, &empty), &only, true);
    assert!(identical.failing.is_empty());
    assert_eq!(
        Verdict::check(&reference, &stamp, &identical),
        Verdict::Pass
    );
    let none = BTreeMap::new();
    let only_missing = CompilerParity::build(compare_all(&reference, &none, &empty), &none, false);
    assert_eq!(
        Verdict::check(&reference, &stamp, &only_missing),
        Verdict::NothingCompared
    );
    assert_eq!(
        Verdict::check(&reference, &stamp, &only_missing).exit_code(),
        3
    );
}

#[test]
fn a_reference_that_does_not_cover_the_run_fails_on_the_rows_it_lacks() {
    let ours = rows();
    let mut referenced = ours.clone();
    let extra = referenced.pop_last().unwrap().0;
    let empty = Allowlist::default();
    // Covering every row of the run: nothing unreferenced.
    let (full, stamp) = reference_for(&ours);
    let covered = CompilerParity::build(compare_all(&full, &ours, &empty), &ours, false);
    assert!(covered.unreferenced.is_empty());
    assert_eq!(Verdict::check(&full, &stamp, &covered), Verdict::Pass);
    // A narrowed reference: the row it lacks is unreferenced and fails, strict or not.
    let (reference, stamp) = reference_for(&referenced);
    for strict in [false, true] {
        let parity = CompilerParity::build(compare_all(&reference, &ours, &empty), &ours, strict);
        assert_eq!(parity.unreferenced, std::slice::from_ref(&extra));
        assert!(!parity.rows.contains_key(&extra));
        assert_eq!(parity.failing, std::slice::from_ref(&extra));
        assert_eq!(Verdict::check(&reference, &stamp, &parity), Verdict::Fail);
    }
}

#[test]
fn parity_check_refuses_a_run_under_another_harness_renderer_font_set_or_scoring() {
    let ours = rows();
    let (reference, stamp) = reference_for(&ours);
    let parity = CompilerParity::build(
        compare_all(&reference, &ours, &Allowlist::default()),
        &ours,
        false,
    );
    assert!(reference.mismatches(&stamp).is_empty());
    assert_eq!(Verdict::check(&reference, &stamp, &parity), Verdict::Pass);

    let mut other = stamp.clone();
    other.compiler.content = Sha256::of("another compiler");
    other.renderer.image_id = "sha256:other".into();
    assert!(reference.mismatches(&other).is_empty());

    let mut other = stamp.clone();
    other.harness = Sha256::of("another bench");
    other.renderer.recipe = Sha256::of("another recipe");
    other.fonts.font_set_id = Sha256::of("another font set");
    other.metrics.text = "text@3".into();
    other.classifier = "classifier@3".into();
    other.lane = Lane::Motion;
    let fields = [
        "harness",
        "renderer.recipe",
        "fonts.fontSetId",
        "metrics",
        "classifier",
        "lane",
    ];
    assert_eq!(reference.mismatches(&other), fields);
    let verdict = Verdict::check(&reference, &other, &parity);
    assert_eq!(verdict, Verdict::Refused(fields.map(String::from).to_vec()));
    assert_eq!(verdict.exit_code(), 2);

    let mut extra_metric = reference.clone();
    extra_metric
        .metrics
        .insert("motion".into(), "motion@1".into());
    assert_eq!(extra_metric.mismatches(&stamp), ["metrics"]);
}

#[test]
fn compiler_rows_count_diagnostic_deltas() {
    let (id, ours) = rows().into_iter().next().unwrap();
    let mut reference = reference_of(&ours);
    reference.diagnostics.insert("KN1006".into(), 2);
    let row = CompilerRow::compare(&id, &reference, Some(&ours), &Allowlist::default());
    assert_eq!(row.diagnostics, BTreeMap::from([("KN1006".to_owned(), -2)]));
}

#[test]
fn the_allowlist_reads_the_committed_shape_and_checks_it() {
    let committed = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/parity/allow.json"),
    )
    .unwrap();
    Allowlist::parse(&committed).unwrap();
    let entry = json!({ "row": "demo-basics:pp-9001:pptx", "reason": "r", "path": "$.x", "reviewBy": "2027-02-28" });
    assert!(Allowlist::parse(&json!([entry]).to_string()).is_ok());
    for (key, bad) in [
        ("reviewBy", json!("2027-13-01")),
        ("reviewBy", json!("27-01-01")),
        ("reason", json!("")),
        ("status", json!("identical")),
        ("row", json!("demo-basics:pp-9001")),
    ] {
        let mut broken = entry.clone();
        broken[key] = bad.clone();
        assert!(
            Allowlist::parse(&json!([broken]).to_string()).is_err(),
            "{key}: {bad}"
        );
    }
    let twice = json!([entry, entry]).to_string();
    assert!(
        Allowlist::parse(&twice)
            .unwrap_err()
            .to_string()
            .contains("listed twice")
    );
}

#[test]
fn first_difference_names_the_first_path_in_canonical_order() {
    let a = json!({ "b": [1, { "x": 2.0 }], "a": 1, "c": true });
    assert_eq!(first_difference(&a, &a.clone()), None);
    assert_eq!(
        first_difference(&a, &json!({ "b": [1, { "x": 2 }], "a": 1, "c": true })),
        None
    );
    assert_eq!(
        first_difference(&a, &json!({ "b": [1, { "x": 3 }], "a": 1, "c": false })).unwrap(),
        "$.b[1].x"
    );
    assert_eq!(
        first_difference(&a, &json!({ "b": [1], "a": 1, "c": true })).unwrap(),
        "$.b[1]"
    );
    assert_eq!(
        first_difference(&a, &json!({ "b": [1, { "x": 2 }], "c": true })).unwrap(),
        "$.a"
    );
    assert_eq!(first_difference(&json!(1), &json!("1")).unwrap(), "$");
}

#[test]
fn the_reference_and_report_formats_refuse_other_versions_and_fields() {
    let reference = json!({
        "schema": "slidock-parity-ref@2",
        "golden": { "commit": "c", "manifest": "0".repeat(64) },
        "harness": "5".repeat(64),
        "rendererRecipe": "1".repeat(64),
        "fontSetId": "2".repeat(64),
        "metrics": {},
        "classifier": "classifier@2",
        "rows": {},
    });
    ParityReference::parse(&reference.to_string()).unwrap();
    let mut old = reference.clone();
    old["schema"] = json!("slidock-parity-ref@1");
    assert!(ParityReference::parse(&old.to_string()).is_err());
    let mut renders_from = reference.clone();
    renders_from["rendersFrom"] = json!("rekey");
    assert!(ParityReference::parse(&renders_from.to_string()).is_err());
    let mut unkeyed = reference.clone();
    unkeyed.as_object_mut().unwrap().remove("harness");
    assert!(ParityReference::parse(&unkeyed.to_string()).is_err());
    let mut unpinned = reference.clone();
    unpinned["golden"]
        .as_object_mut()
        .unwrap()
        .remove("manifest");
    assert!(ParityReference::parse(&unpinned.to_string()).is_err());

    let report = json!({
        "schema": "slidock-parity@2",
        "reference": {
            "sha256": "3".repeat(64),
            "golden": { "commit": "c", "manifest": "0".repeat(64) },
            "rendererRecipe": "1".repeat(64),
            "fontSetId": "2".repeat(64),
        },
        "run": "4".repeat(64),
    });
    ParityReport::parse(&report.to_string()).unwrap();
    let mut old = report.clone();
    old["schema"] = json!("slidock-parity@1");
    assert!(ParityReport::parse(&old.to_string()).is_err());
    let mut harness = report.clone();
    harness["harness"] = json!({});
    assert!(ParityReport::parse(&harness.to_string()).is_err());
}

#[test]
fn a_fonts_class_says_whether_fallback_fonts_drew() {
    let text = data("fonts-results.json");
    let results = Results::parse(&text).unwrap();
    assert!(
        results
            .rows
            .values()
            .any(|row| matches!(row.outcome, RowOutcome::Fonts(_)))
    );
    let mut value: Value = serde_json::from_str(&text).unwrap();
    let rows = value["rows"].as_object_mut().unwrap();
    let complete = rows
        .iter()
        .find(|(_, row)| row["outcome"] == "fonts-complete")
        .map(|(id, _)| id.clone())
        .unwrap();
    rows[&complete]["fonts"]["total"]["fallback"] = json!(3);
    assert!(invalid(&value).contains("drawn by fallback fonts"));

    let mut value: Value = serde_json::from_str(&text).unwrap();
    let (_, row) = first_row(&mut value);
    row.as_object_mut().unwrap().remove("fonts");
    assert!(invalid(&value).contains("without fonts metrics"));

    let mut value = results_value("results.json");
    let (_, row) = first_row(&mut value);
    row["outcome"] = json!("fonts-complete");
    assert!(invalid(&value).contains("a fonts class in a Static results file"));
}
