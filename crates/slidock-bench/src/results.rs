//! `results.json`: the summaries, merging an overlay into the run it updates, and the run folders:
//! `<cache>/runs/<runKey>/` holds a run's compiled presentations, reports, IR, renders, row records and `results.json`;
//! `runs/latest` points at the newest full run.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use slidock_bench_metrics::{median, percentile, quantise};
use slidock_bench_schema::results::{
    Distribution, FrontendSummary, Mean, Outcome, Results, ResultsSchema, RowRecord, Summary,
};
use slidock_bench_schema::stamp::Stamp;
use slidock_bench_schema::{Frontend, RowId};

use crate::cache::{temp_sibling, write_atomic};
use crate::error::{BenchError, Result};

pub const LATEST: &str = "latest";

fn quantised(value: f64) -> Option<f64> {
    (!value.is_nan()).then(|| quantise(value))
}

/// Class counts, ssim median and 10th percentile, and mean text recall over a set of rows.
pub fn summarise(rows: &[&RowRecord]) -> FrontendSummary {
    let mut summary = FrontendSummary {
        rows: rows.len() as u32,
        ..FrontendSummary::default()
    };
    for row in rows {
        match row.outcome.classifier() {
            Some(Outcome::Match) => summary.matched += 1,
            Some(Outcome::Close) => summary.close += 1,
            Some(Outcome::Mismatch) => summary.mismatch += 1,
            Some(Outcome::Declined) => summary.declined += 1,
            Some(Outcome::RenderError) => summary.render_error += 1,
            Some(Outcome::Failed) => summary.failed += 1,
            None => {}
        }
    }
    let scored: Vec<_> = rows.iter().filter_map(|row| row.scores.as_ref()).collect();
    if !scored.is_empty() {
        let mut ssims: Vec<f64> = scored.iter().map(|s| s.ssim.mean).collect();
        // Float64Array.prototype.sort: numeric, NaN last.
        ssims.sort_by(|a, b| {
            a.partial_cmp(b)
                .unwrap_or_else(|| a.is_nan().cmp(&b.is_nan()))
        });
        summary.ssim = Distribution {
            median: quantised(median(&ssims)),
            p10: quantised(percentile(&ssims, 0.1)),
        };
        let total = scored.iter().fold(0.0, |sum, s| sum + s.text.recall);
        summary.text_recall = Mean {
            mean: quantised(total / scored.len() as f64),
        };
    }
    summary
}

/// The summary section: per frontend (reference rows apart) and the decline histogram by code.
pub fn summary(rows: &BTreeMap<RowId, RowRecord>) -> Summary {
    let mut summary = Summary::default();
    for frontend in Frontend::ALL {
        let own: Vec<&RowRecord> = rows
            .values()
            .filter(|row| row.frontend == frontend)
            .collect();
        let main: Vec<&RowRecord> = own
            .iter()
            .copied()
            .filter(|row| row.reference.is_none())
            .collect();
        let references: Vec<&RowRecord> = own
            .iter()
            .copied()
            .filter(|row| row.reference.is_some())
            .collect();
        if !main.is_empty() {
            summary.by_frontend.insert(frontend, summarise(&main));
        }
        if !references.is_empty() {
            summary.reference.insert(frontend, summarise(&references));
        }
    }
    for row in rows.values() {
        if let (Some(declined), None) = (&row.declined, row.reference) {
            *summary.declines.entry(declined.code.clone()).or_default() += 1;
        }
    }
    summary
}

/// The stamp fields, beyond the compiler, in which an overlay's session differs from the run it updates.
pub fn overlay_drift(previous: &Stamp, current: &Stamp) -> Vec<String> {
    let mut fields = Vec::new();
    let mut check = |name: String, equal: bool| {
        if !equal {
            fields.push(name);
        }
    };
    check("harness".into(), previous.harness == current.harness);
    check("renderer".into(), previous.renderer == current.renderer);
    check("fonts".into(), previous.fonts == current.fonts);
    check("metrics".into(), previous.metrics == current.metrics);
    check(
        "classifier".into(),
        previous.classifier == current.classifier,
    );
    check("deps".into(), previous.deps == current.deps);
    check("host".into(), previous.host == current.host);
    check("lane".into(), previous.lane == current.lane);
    for (pair, digest) in &current.corpus {
        if let Some(before) = previous.corpus.get(pair) {
            check(format!("corpus[{pair}]"), before == digest);
        }
    }
    fields
}

pub fn refuse_drift(previous: &Stamp, current: &Stamp) -> Result<()> {
    let drift = overlay_drift(previous, current);
    if drift.is_empty() {
        Ok(())
    } else {
        Err(BenchError::refused(format!(
            "runs/latest was scored under another {}: run the selection whole",
            drift.join(", ")
        )))
    }
}

/// A full run's results, or an overlay's: the earlier rows with these replaced, the stamp merged.
pub fn merge_results(
    stamp: &Stamp,
    previous: Option<&Results>,
    records: BTreeMap<RowId, RowRecord>,
) -> Result<Results> {
    let mut rows = BTreeMap::new();
    let mut merged = stamp.clone();
    if let Some(previous) = previous {
        refuse_drift(&previous.stamp, stamp)?;
        rows = previous.rows.clone();
        let mut corpus = previous.stamp.corpus.clone();
        corpus.extend(stamp.corpus.clone());
        merged.corpus = corpus;
        merged.selection = previous.stamp.selection.clone();
    }
    rows.extend(records);
    merged.partial = rows
        .values()
        .any(|row| row.content != merged.compiler.content);
    Ok(Results {
        schema: ResultsSchema::Value,
        summary: summary(&rows),
        stamp: merged,
        rows,
        holdout: None,
        timelines: None,
        font_pairs: None,
        shell_pairs: None,
        motion_pairs: None,
        positions: None,
    })
}

pub fn results_path(run_dir: &Path) -> PathBuf {
    run_dir.join("results.json")
}

pub fn read_results(path: &Path) -> Result<Results> {
    let text = std::fs::read_to_string(path).map_err(BenchError::io(path))?;
    Results::parse(&text)
        .map_err(|error| BenchError::failed(format!("{}: {error}", path.display())))
}

pub fn write_results(path: &Path, results: &Results) -> Result<()> {
    write_atomic(path, results.to_canonical()?)
}

/// The folder `runs/latest` points at, if any.
pub fn latest_run_dir(runs: &Path) -> Option<PathBuf> {
    let link = runs.join(LATEST);
    let target = std::fs::read_link(&link).ok()?;
    let dir = runs.join(target);
    dir.is_dir().then_some(dir)
}

/// Points `runs/latest` at a run folder, replacing the link atomically.
pub fn set_latest(runs: &Path, key: &str) -> Result<()> {
    std::fs::create_dir_all(runs).map_err(BenchError::io(runs))?;
    let link = runs.join(LATEST);
    let temp = temp_sibling(&link);
    std::os::unix::fs::symlink(key, &temp).map_err(BenchError::io(&temp))?;
    std::fs::rename(&temp, &link).map_err(BenchError::io(&link))
}

/// Where a results file named on the command line lives: `baseline` (the static lane's baseline), `latest`
/// (`runs/latest`), a run key (`runs/<key>/`), or a path to a results file or a run folder.
pub fn resolve_results(runs: &Path, baseline: &Path, spec: &str) -> Result<PathBuf> {
    let path = match spec {
        "baseline" => baseline.to_path_buf(),
        "latest" => results_path(&latest_run_dir(runs).ok_or_else(|| {
            BenchError::refused("the cache has no runs/latest; run the bench first")
        })?),
        key if key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()) => {
            results_path(&runs.join(key))
        }
        other => {
            let path = PathBuf::from(other);
            if path.is_dir() {
                results_path(&path)
            } else {
                path
            }
        }
    };
    if path.is_file() {
        Ok(path)
    } else {
        Err(BenchError::refused(format!(
            "no results at {} ({spec})",
            path.display()
        )))
    }
}

/// A row's file name: colons become `__`.
pub fn row_file_name(row: &RowId) -> String {
    row.to_string().replace(':', "__")
}

/// Where a run keeps one row's record, render and measure.
pub struct RowPaths {
    pub record: PathBuf,
    pub render: PathBuf,
    pub measure: PathBuf,
}

pub fn row_paths(run_dir: &Path, row: &RowId) -> RowPaths {
    let name = row_file_name(row);
    RowPaths {
        record: run_dir.join("rows").join(format!("{name}.json")),
        render: run_dir.join("renders").join(format!("{name}.png")),
        measure: run_dir.join("measure").join(format!("{name}.json")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slidock_bench_schema::results::{ColorScore, OraclePage, Scores, SsimScore, TextScore};
    use slidock_bench_schema::{PresentationId, Sha256, SlideId};

    fn row(slide: &str, outcome: Outcome, ssim: Option<f64>) -> RowRecord {
        RowRecord {
            presentation: PresentationId::new("demo").unwrap(),
            slide: SlideId::new(slide).unwrap(),
            frontend: Frontend::Pptx,
            oracle: OraclePage { page: 1 },
            outcome: outcome.into(),
            reason: None,
            code: None,
            data: None,
            declined: None,
            scores: ssim.map(|mean| Scores {
                ssim: SsimScore { mean, p1: mean },
                color: ColorScore {
                    mean: 1.0,
                    p99: 2.0,
                },
                pixel: 1.0,
                text: TextScore {
                    oracle_words: 1,
                    dom_words: 1,
                    matched: 1,
                    recall: mean,
                    precision: 1.0,
                    median_pm: None,
                    p95_pm: None,
                    height_ratio_median: None,
                },
            }),
            diagnostics: BTreeMap::new(),
            html: None,
            render: None,
            content: Sha256::of("x"),
            reference: None,
            known_loss: None,
            stage: None,
            transition_pm: None,
            host: None,
            fonts: None,
            motion: None,
            shadow: None,
            gradient: None,
            shell: None,
        }
    }

    #[test]
    fn summarises_classes_and_distributions() {
        let rows = [
            row("pp-1", Outcome::Match, Some(0.99)),
            row("pp-2", Outcome::Close, Some(0.95)),
            row("pp-3", Outcome::Mismatch, Some(0.5)),
            row("pp-4", Outcome::Failed, None),
        ];
        let refs: Vec<&RowRecord> = rows.iter().collect();
        let summary = summarise(&refs);
        assert_eq!(
            (
                summary.rows,
                summary.matched,
                summary.close,
                summary.mismatch,
                summary.failed
            ),
            (4, 1, 1, 1, 1)
        );
        assert_eq!(summary.ssim.median, Some(0.95));
        assert_eq!(summary.ssim.p10, Some(0.5));
        assert_eq!(
            summary.text_recall.mean,
            Some(quantise((0.99 + 0.95 + 0.5) / 3.0))
        );
        let empty = summarise(&[]);
        assert_eq!(empty.ssim.median, None);
    }

    #[test]
    fn row_files_use_double_underscores() {
        let id: RowId = "pptx-talk:pp-256:pptx".parse().unwrap();
        assert_eq!(row_file_name(&id), "pptx-talk__pp-256__pptx");
    }

    #[test]
    fn latest_moves_atomically() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a")).unwrap();
        std::fs::create_dir_all(dir.path().join("b")).unwrap();
        set_latest(dir.path(), "a").unwrap();
        assert_eq!(latest_run_dir(dir.path()), Some(dir.path().join("a")));
        set_latest(dir.path(), "b").unwrap();
        assert_eq!(latest_run_dir(dir.path()), Some(dir.path().join("b")));
    }
}
