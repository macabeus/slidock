//! Parity with slidock's goldens (levels B and C; [`crate::golden`]).
//!
//! `parity reference` renders and scores the golden static-lane output of every corpus pair, unchanged, with this
//! bench, into `<cache>/parity/golden-<commit12>/` (a run folder: the output linked in, the renders and records
//! written), and writes `reference.json` there: the hash-only reference, keyed under this bench's renderer recipe,
//! font set, metric versions and classifier.
//!
//! `parity report | check` compares a run's rows with that reference (the composed document's sha256, then the
//! render's) and the run's IR with the golden IR, container by container, writes `runs/<key>/parity.json` and stamps
//! the report's hash and the golden's identity into the run's results. Both name rows, so they stay private.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clap::Args;
use slidock_bench_schema::corpus::{Presentation, Source};
use slidock_bench_schema::parity::{
    GoldenIdentity, ParityReference, ParityReport, ReferenceRow, ReferenceSchema, ReportReference,
    ReportSchema,
};
use slidock_bench_schema::results::Results;
use slidock_bench_schema::{Sha256, Tier};

use crate::cache::write_atomic;
use crate::compile::{CompileSettings, CompileStatus, pair_paths};
use crate::corpus::{Corpus, Selection, load_manifest, select_pairs};
use crate::error::{BenchError, Exit, Result};
use crate::golden::GoldenSet;
use crate::hash::FileHashes;
use crate::oracle::resolve::OracleResolver;
use crate::results::{merge_results, results_path, write_results};
use crate::runner::{ExecuteOptions, PairJob, RowContext, execute_pairs};
use crate::setup::{Context, parse_list, start_session};

/// The reference file in a reference folder.
const REFERENCE_FILE: &str = "reference.json";

#[derive(Debug, Args)]
pub struct ReferenceArgs {
    /// Only these presentations; the reference then holds only their rows, and a check of a run with other rows
    /// fails on them.
    #[arg(long)]
    presentation: Option<String>,
    #[arg(long, default_value_t = 4)]
    concurrency: usize,
}

/// Where `parity reference` renders a golden set: `<cache>/parity/golden-<commit12>`.
pub fn reference_dir(cache_root: &Path, golden: &GoldenIdentity) -> PathBuf {
    let commit12 = golden.commit.get(..12).unwrap_or(&golden.commit);
    cache_root.join("parity").join(format!("golden-{commit12}"))
}

/// The golden set the committed manifest pins, or a refusal naming why it cannot be read.
fn open_golden(ctx: &Context) -> Result<GoldenSet> {
    GoldenSet::open(&ctx.env.golden_manifest, &ctx.env.golden_dir)?.map_err(|reason| {
        BenchError::refused(format!(
            "{reason}; record the goldens (`cargo xtask golden --record`) or set SLIDOCK_PRIVATE"
        ))
    })
}

/// The reference folder of the committed manifest's set, when `parity reference` rendered that very set: the folder
/// `explain` and the UI read the golden renders from. Reads the manifest only, not the golden files.
pub fn current_reference_dir(ctx: &Context) -> Option<PathBuf> {
    let text = std::fs::read(&ctx.env.golden_manifest).ok()?;
    let commit = serde_json::from_slice::<serde_json::Value>(&text)
        .ok()?
        .get("commit")?
        .as_str()?
        .to_owned();
    let golden = GoldenIdentity {
        commit,
        manifest: Sha256::of(&text),
    };
    let dir = reference_dir(&ctx.cache.root, &golden);
    (read_reference(&dir).ok()?.golden == golden).then_some(dir)
}

fn read_reference(dir: &Path) -> Result<ParityReference> {
    let path = dir.join(REFERENCE_FILE);
    let text = std::fs::read_to_string(&path).map_err(BenchError::io(&path))?;
    ParityReference::parse(&text)
        .map_err(|error| BenchError::failed(format!("{}: {error}", path.display())))
}

/// Lays a golden pair's static-lane output out as a run folder holds a compile: the documents, report and IR linked,
/// and the status the output shows (`ok` with a document set, `refused` without).
pub(crate) fn link_golden_pair(
    golden: &Path,
    run_dir: &Path,
    presentation: &str,
    frontend: &str,
) -> Result<()> {
    let paths = pair_paths(run_dir, presentation, frontend);
    for (from, to) in [
        (golden.to_path_buf(), &paths.compiled),
        (golden.join("report.json"), &paths.report),
        (golden.join("presentation-ir.json"), &paths.ir),
    ] {
        crate::cache::remove(to)?;
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(BenchError::io(parent))?;
        }
        if from.exists() {
            std::os::unix::fs::symlink(&from, to).map_err(BenchError::io(to))?;
        }
    }
    let status = if golden.join("presentation.json").is_file() {
        CompileStatus::Ok
    } else {
        CompileStatus::Refused
    };
    crate::cache::write_json(&paths.status, &serde_json::json!({ "status": status }))
}

/// The hash-only reference of a scored golden run.
fn reference_of(results: &Results, golden: GoldenIdentity) -> Result<ParityReference> {
    let stamp = &results.stamp;
    let mut rows = BTreeMap::new();
    for (id, row) in &results.rows {
        let outcome = row.outcome.classifier().ok_or_else(|| {
            BenchError::failed(format!("{id}: the golden run's row has no static class"))
        })?;
        rows.insert(
            id.clone(),
            ReferenceRow {
                html: row.html.clone(),
                render: row.render.clone(),
                outcome,
                reason: row.reason,
                code: row.code.clone(),
                data: row.data.clone(),
                declined: row.declined.clone(),
                scores: row.scores,
                diagnostics: row.diagnostics.clone(),
                timeline: None,
                transition: None,
            },
        );
    }
    let metrics = &stamp.metrics;
    Ok(ParityReference {
        schema: ReferenceSchema::Value,
        golden,
        harness: stamp.harness.clone(),
        renderer_recipe: stamp.renderer.recipe.clone(),
        font_set_id: stamp.fonts.font_set_id.clone(),
        metrics: BTreeMap::from([
            ("color".to_owned(), metrics.color.clone()),
            ("pixel".to_owned(), metrics.pixel.clone()),
            ("ssim".to_owned(), metrics.ssim.clone()),
            ("text".to_owned(), metrics.text.clone()),
        ]),
        classifier: stamp.classifier.clone(),
        rows,
    })
}

fn log_line(line: &str) {
    eprintln!("{line}");
}

fn short(commit: &str) -> &str {
    commit.get(..12).unwrap_or(commit)
}

pub async fn reference(ctx: &Context, args: ReferenceArgs) -> Result<Exit> {
    let golden = open_golden(ctx)?;
    let manifest = load_manifest(&ctx.env.manifest_path)?;
    let selection = Selection {
        tiers: Tier::DEFAULT.to_vec(),
        presentations: parse_list(args.presentation.as_deref(), "presentation")?,
        frontends: None,
    };
    let pairs: Vec<(&Presentation, &Source)> = select_pairs(&manifest, &selection);
    let absent: Vec<String> = pairs
        .iter()
        .filter(|(p, s)| {
            golden
                .static_pair(p.id.as_str(), s.frontend.as_str())
                .is_none()
        })
        .map(|(p, s)| format!("{}.{}", p.id, s.frontend))
        .collect();
    if !absent.is_empty() {
        return Err(BenchError::refused(format!(
            "the goldens hold no static-lane output of {}",
            absent.join(", ")
        )));
    }
    let run_dir = reference_dir(&ctx.cache.root, &golden.identity);
    for (presentation, source) in &pairs {
        let (id, frontend) = (presentation.id.as_str(), source.frontend.as_str());
        if let Some(dir) = golden.static_pair(id, frontend) {
            link_golden_pair(&dir, &run_dir, id, frontend)?;
        }
    }

    let session = start_session(ctx, &pairs, &Tier::DEFAULT, false).await?;
    let corpus = Corpus::new(
        ctx.env.require_corpus_dir()?,
        FileHashes::new(&ctx.cache.hashes, true),
        &manifest,
    );
    let resolver = OracleResolver::new(
        session.renderer.clone(),
        session.poppler.clone(),
        ctx.env.keynote_dir.clone(),
        false,
    );
    let jobs: Vec<PairJob<'_>> = pairs
        .iter()
        .map(|(presentation, source)| PairJob {
            presentation,
            source,
            rows: presentation.rows(source),
            only: None,
        })
        .collect();
    let settings = CompileSettings {
        binary: PathBuf::new(),
        fonts_dir: ctx.cache.fonts.clone(),
        quiet: true,
    };
    let context = RowContext::new(
        Some(session.browser.clone()),
        ctx.cache.gif_frames.clone(),
        log_line,
    );
    // The golden output is named by its manifest: the compile it came from is pinned there, not built here.
    let mut stamp = session.stamp.clone();
    stamp.compiler.content = golden.identity.manifest.clone();
    stamp.compiler.commit = golden.identity.commit.clone();
    stamp.compiler.dirty = false;
    let options = ExecuteOptions {
        run_dir: &run_dir,
        corpus: &corpus,
        settings: &settings,
        oracle: &resolver,
        content: stamp.compiler.content.clone(),
        concurrency: args.concurrency,
        reuse_compiled: true,
    };
    let executed = execute_pairs(&jobs, &options, &context).await?;
    let results = merge_results(&stamp, None, executed.records)?;
    write_results(&results_path(&run_dir), &results)?;
    let reference = reference_of(&results, golden.identity.clone())?;
    let path = run_dir.join(REFERENCE_FILE);
    write_atomic(&path, reference.to_canonical()?)?;
    for (frontend, summary) in &results.summary.by_frontend {
        println!(
            "{frontend}: {} rows: match {}, close {}, mismatch {}",
            summary.rows, summary.matched, summary.close, summary.mismatch
        );
    }
    println!(
        "reference of golden {} ({} rows of {} of its {} pairs): {}",
        short(&golden.identity.commit),
        reference.rows.len(),
        pairs.len(),
        golden.static_pairs().len(),
        path.display()
    );
    Ok(Exit::Ok)
}

#[derive(Debug, Args)]
pub struct CompilerArgs {
    /// The slidock run compared: `latest`, a run key or a path.
    #[arg(long, default_value = "latest")]
    run: String,
    /// Requires every row to be identical.
    #[arg(long)]
    strict: bool,
}

/// `parity report` and `parity check` (levels B and C): the run's rows against the golden reference, written to
/// `runs/<key>/parity.json`, and the report's hash stamped into the run's results. `check` exits with the verdict: 1
/// on a row that needs review and is not allowed or a row outside the holdout the reference lacks, 2 when the run is
/// stamped under another harness, renderer, font set or metrics than the reference, 3 when no row was compared. Both
/// refuse a reference rendered from another golden manifest than the committed one.
pub async fn compiler(ctx: &Context, args: CompilerArgs, check: bool) -> Result<Exit> {
    use crate::parity::{ParityInputs, compiler_parity};
    use crate::results::resolve_results;
    use slidock_bench_schema::parity::{Allowlist, ParityStatus, Verdict};
    use slidock_bench_schema::stamp::ParityStamp;

    let golden = open_golden(ctx)?;
    let reference_dir = reference_dir(&ctx.cache.root, &golden.identity);
    let mut reference = match read_reference(&reference_dir) {
        Ok(reference) if reference.golden == golden.identity => reference,
        Ok(_) => {
            return Err(BenchError::refused(format!(
                "{} was rendered from another golden manifest: run `slidock-bench parity reference`",
                reference_dir.join(REFERENCE_FILE).display()
            )));
        }
        Err(error) => {
            return Err(BenchError::refused(format!(
                "{error}: run `slidock-bench parity reference` first"
            )));
        }
    };
    let reference_sha = Sha256::of(reference.to_canonical()?);

    let results_file = resolve_results(&ctx.cache.runs, &ctx.env.baseline_path(), &args.run)?;
    let run_dir = results_file
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| BenchError::failed("the results have no run folder"))?;
    let mut run = crate::results::read_results(&results_file)?;
    let holdout = crate::corpus::holdout_ids(&load_manifest(&ctx.env.manifest_path)?);
    reference
        .rows
        .retain(|id, _| !holdout.contains(id.presentation()));
    let mut compared = run.clone();
    compared
        .rows
        .retain(|id, _| !holdout.contains(id.presentation()));
    let allow_path = ctx.env.bench_dir.join("parity/allow.json");
    let allowlist = Allowlist::parse(
        &std::fs::read_to_string(&allow_path).map_err(BenchError::io(&allow_path))?,
    )?;
    let inputs = ParityInputs {
        run_dir: &run_dir,
        reference_renders: Some(reference_dir.as_path()),
        golden: &golden,
    };
    let (compiler, ir) = compiler_parity(&reference, &compared, &allowlist, &inputs, args.strict)?;
    let report = ParityReport {
        schema: ReportSchema::Value,
        reference: ReportReference {
            sha256: reference_sha.clone(),
            golden: golden.identity.clone(),
            renderer_recipe: reference.renderer_recipe.clone(),
            font_set_id: reference.font_set_id.clone(),
        },
        run: run.stamp.run_key()?,
        compiler: Some(compiler.clone()),
        ir: Some(ir.clone()),
    };
    let report_path = run_dir.join("parity.json");
    write_atomic(&report_path, report.to_canonical()?)?;
    run.stamp.parity = Some(ParityStamp {
        golden: golden.identity.clone(),
        reference: reference_sha,
        report: report.sha256()?,
    });
    write_results(&results_file, &run)?;

    let count = |status: ParityStatus| compiler.counts.get(&status).copied().unwrap_or(0);
    println!(
        "B compiler parity over {} reference rows (golden {}):",
        compiler.rows.len(),
        short(&golden.identity.commit)
    );
    for status in [
        ParityStatus::Identical,
        ParityStatus::PixelIdentical,
        ParityStatus::DriftBetter,
        ParityStatus::DriftSameClass,
        ParityStatus::Regressed,
        ParityStatus::Missing,
        ParityStatus::New,
    ] {
        let name = serde_json::to_value(status)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        println!("  {name:<16} {}", count(status));
    }
    println!("  unreferenced     {}", compiler.unreferenced.len());
    if !compiler.unreferenced.is_empty() {
        eprintln!(
            "the reference does not cover {} of the run's rows: run `slidock-bench parity reference` over their presentations",
            compiler.unreferenced.len()
        );
    }
    for id in compiler.failing.iter().take(20) {
        if let Some(row) = compiler.rows.get(id) {
            println!(
                "  needs review: {id} {:?} ir {}",
                row.status,
                row.first_ir_diff.as_deref().unwrap_or("–")
            );
        }
    }
    let mut paths: BTreeMap<String, u32> = BTreeMap::new();
    for row in compiler.rows.values() {
        if let Some(path) = &row.first_ir_diff {
            let head: String = path.split('.').take(3).collect::<Vec<_>>().join(".");
            *paths.entry(head).or_default() += 1;
        }
    }
    for (path, rows) in &paths {
        println!("  first IR difference {path}: {rows} rows");
    }
    let mut ir_counts: BTreeMap<String, u32> = BTreeMap::new();
    for entry in ir.containers.values() {
        let name = serde_json::to_value(entry.status)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        *ir_counts.entry(name).or_default() += 1;
    }
    println!(
        "C IR parity over {} containers: {}",
        ir.containers.len(),
        ir_counts
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("report: {}", report_path.display());
    let verdict = Verdict::check(&reference, &run.stamp, &compiler);
    if let Verdict::Refused(fields) = &verdict {
        eprintln!(
            "refused: the run is stamped under another {} than the reference",
            fields.join(", ")
        );
    }
    if !check {
        return Ok(match verdict {
            Verdict::Refused(_) => Exit::Refused,
            _ => Exit::Ok,
        });
    }
    Ok(match verdict {
        Verdict::Pass => Exit::Ok,
        Verdict::Fail => Exit::Failed,
        Verdict::Refused(_) => Exit::Refused,
        Verdict::NothingCompared => Exit::NothingCompared,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use slidock_bench_schema::RowId;
    use slidock_bench_schema::results::{MotionOutcome, RowOutcome, RowRecord};

    #[test]
    fn the_reference_folder_is_named_by_the_golden_commit() {
        let golden = GoldenIdentity {
            commit: "0123456789abcdef0123456789abcdef01234567".into(),
            manifest: Sha256::of("m"),
        };
        assert_eq!(
            reference_dir(Path::new("/cache"), &golden),
            Path::new("/cache/parity/golden-0123456789ab")
        );
    }

    #[test]
    fn a_golden_pair_links_as_a_compile_with_its_status() {
        let dir = tempfile::tempdir().unwrap();
        let golden = dir.path().join("golden/static/demo.pptx");
        std::fs::create_dir_all(&golden).unwrap();
        std::fs::write(golden.join("report.json"), "{\"diagnostics\":[]}").unwrap();
        std::fs::write(golden.join("presentation-ir.json"), "{}").unwrap();
        let run = dir.path().join("run");
        link_golden_pair(&golden, &run, "demo", "pptx").unwrap();
        let paths = pair_paths(&run, "demo", "pptx");
        assert_eq!(std::fs::read_link(&paths.compiled).unwrap(), golden);
        assert_eq!(std::fs::read_to_string(&paths.ir).unwrap(), "{}");
        let status = |paths: &crate::compile::PairPaths| {
            serde_json::from_str::<serde_json::Value>(
                &std::fs::read_to_string(&paths.status).unwrap(),
            )
            .unwrap()["status"]
                .clone()
        };
        assert_eq!(status(&paths), "refused");
        std::fs::write(golden.join("presentation.json"), "{}").unwrap();
        link_golden_pair(&golden, &run, "demo", "pptx").unwrap();
        assert_eq!(status(&paths), "ok");
    }

    #[test]
    fn the_reference_keeps_each_row_and_refuses_a_row_without_a_static_class() {
        let mut results: Results = slidock_bench_schema::parse(include_str!(
            "../../../slidock-bench-schema/tests/data/results.json"
        ))
        .unwrap();
        let golden = GoldenIdentity {
            commit: "c".into(),
            manifest: Sha256::of("m"),
        };
        let reference = reference_of(&results, golden.clone()).unwrap();
        assert_eq!(reference.rows.len(), results.rows.len());
        assert_eq!(reference.renderer_recipe, results.stamp.renderer.recipe);
        for (id, row) in &results.rows {
            let kept = &reference.rows[id];
            assert_eq!((&kept.html, &kept.render), (&row.html, &row.render));
            assert_eq!(Some(kept.outcome), row.outcome.classifier());
        }
        let (id, row): (&RowId, &mut RowRecord) = results.rows.iter_mut().next().unwrap();
        let id = id.clone();
        row.outcome = RowOutcome::Motion(MotionOutcome::MotionMatch);
        let error = reference_of(&results, golden).unwrap_err();
        assert!(error.to_string().contains(&id.to_string()), "{error}");
    }
}
