//! `determinism`: the gate a baseline must pass. See [`crate::determinism`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::Args;
use slidock_bench_schema::corpus::{Presentation, Source};
use slidock_bench_schema::{PresentationId, RowId, Sha256, Tier};

use crate::compile::{CompileSettings, compiler_binary};
use crate::corpus::{Corpus, Selection, load_manifest, refuse_holdout, select_pairs};
use crate::determinism::{
    Builds, DeterminismReport, Legs, REPORT_SCHEMA, ScoreReport, WasmFrom, compare_scored_compile,
    forbidden_strings, in_work_dir, pair_dir, run_legs, write_report,
};
use crate::error::{BenchError, Exit, Result};
use crate::exec::{ExecOptions, args, exec_text};
use crate::hash::{FileHashes, sha256_file};
use crate::oracle::resolve::OracleResolver;
use crate::results::row_paths;
use crate::runner::{ExecuteOptions, PairJob, RowContext, execute_pairs};
use crate::setup::{Context, parse_list, parse_tier, start_session};

#[derive(Debug, Args)]
pub struct DeterminismArgs {
    #[arg(long)]
    presentation: Option<String>,
    /// Tiers to select [default: smoke,real].
    #[arg(long)]
    tier: Option<String>,
    /// Also scores every row twice (the second pass in reverse order) and compares the records and renders, and the
    /// scored compile with leg A's.
    #[arg(long)]
    score: bool,
}

/// What a determinism run selects.
pub struct DeterminismRequest {
    pub presentations: Option<Vec<PresentationId>>,
    pub tiers: Vec<Tier>,
    pub score: bool,
}

fn log_line(line: &str) {
    eprintln!("{line}");
}

/// Scores every row twice, the second time in reverse order on the first pass's compile, and lists the rows whose
/// record or render moved and the pairs whose scored compile differs from leg A's.
async fn score_twice(
    ctx: &Context,
    pairs: &[(&Presentation, &Source, PathBuf, Vec<RowId>)],
    work: &Path,
) -> Result<ScoreReport> {
    let manifest = load_manifest(&ctx.env.manifest_path)?;
    let selected: Vec<(&Presentation, &Source)> =
        pairs.iter().map(|(p, s, _, _)| (*p, *s)).collect();
    let session = start_session(ctx, &selected, &Tier::DEFAULT, false).await?;
    let corpus = Corpus::new(
        ctx.env.require_corpus_dir()?,
        FileHashes::new(&ctx.cache.hashes, true),
        &manifest,
    );
    let binary = compiler_binary(
        &ctx.env.repo_root,
        &ctx.env.target_dir,
        ctx.env.compiler.as_deref(),
    )
    .await?;
    let settings = CompileSettings {
        binary,
        fonts_dir: ctx.cache.fonts.clone(),
        quiet: true,
    };
    let resolver = OracleResolver::new(
        session.renderer.clone(),
        session.poppler.clone(),
        ctx.env.keynote_dir.clone(),
        false,
    );
    let first_dir = work.join("score-1");
    let second_dir = work.join("score-2");
    std::fs::create_dir_all(&second_dir).map_err(BenchError::io(&second_dir))?;
    for name in ["compiled", "reports", "ir"] {
        let link = second_dir.join(name);
        std::os::unix::fs::symlink(first_dir.join(name), &link).map_err(BenchError::io(&link))?;
    }
    let mut passes = Vec::new();
    for (dir, reverse) in [(&first_dir, false), (&second_dir, true)] {
        let mut jobs: Vec<PairJob<'_>> = pairs
            .iter()
            .map(|(presentation, source, _, _)| {
                let mut rows = presentation.rows(source);
                if reverse {
                    rows.reverse();
                }
                PairJob {
                    presentation,
                    source,
                    rows,
                    only: None,
                }
            })
            .collect();
        if reverse {
            jobs.reverse();
        }
        let context = RowContext::new(
            Some(session.browser.clone()),
            ctx.cache.gif_frames.clone(),
            log_line,
        );
        let options = ExecuteOptions {
            run_dir: dir,
            corpus: &corpus,
            settings: &settings,
            oracle: &resolver,
            content: session.stamp.compiler.content.clone(),
            concurrency: 4,
            reuse_compiled: reverse,
        };
        let executed = execute_pairs(&jobs, &options, &context).await?;
        passes.push(executed.records);
    }
    let mut report = ScoreReport::default();
    if let [first, second] = passes.as_slice() {
        for (id, record) in first {
            report.rows += 1;
            let same_record = second.get(id).is_some_and(|other| {
                slidock_bench_schema::to_canonical(record).ok()
                    == slidock_bench_schema::to_canonical(other).ok()
            });
            let same_render = std::fs::read(row_paths(&first_dir, id).render).ok()
                == std::fs::read(row_paths(&second_dir, id).render).ok();
            if !(same_record && same_render) {
                report.differing.push(id.clone());
            }
        }
    }
    for (presentation, source, _, _) in pairs {
        let leg = pair_dir(work, "a", presentation, source);
        let frontend = source.frontend.as_str();
        if let Some(difference) =
            compare_scored_compile(&leg, &first_dir, presentation.id.as_str(), frontend)?
        {
            report
                .compile_differences
                .insert(format!("{}:{frontend}", presentation.id), difference);
        }
    }
    Ok(report)
}

/// The wasm package the B leg runs: `SLIDOCK_WASM_PKG` as it is, or the working copy's, built into `out` as
/// `pnpm wasm:build` builds it (the player it embeds is the one the native binary embeds).
async fn wasm_package(ctx: &Context, out: &Path) -> Result<(PathBuf, WasmFrom)> {
    if let Some(pkg) = std::env::var("SLIDOCK_WASM_PKG")
        .ok()
        .filter(|v| !v.is_empty())
    {
        let pkg = PathBuf::from(pkg);
        if !pkg.join(WASM_FILE).is_file() {
            return Err(BenchError::refused(format!(
                "SLIDOCK_WASM_PKG: {} holds no {WASM_FILE}",
                pkg.display()
            )));
        }
        return Ok((pkg, WasmFrom::Prebuilt));
    }
    log_line("building the wasm compiler for leg B");
    let options = ExecOptions {
        cwd: Some(ctx.env.repo_root.clone()),
        env: BTreeMap::from([(
            "CARGO_TARGET_DIR".to_owned(),
            ctx.env.target_dir.to_string_lossy().into_owned(),
        )]),
        ..ExecOptions::timeout(Duration::from_secs(60 * 60))
    };
    let out_dir = out.to_string_lossy().into_owned();
    exec_text(
        "wasm-pack",
        &args([
            "build",
            "crates/slidock-wasm",
            "--target",
            "web",
            "--profile",
            "wasm",
            "--no-pack",
            "--out-dir",
            &out_dir,
            "--out-name",
            "slidock_wasm",
        ]),
        &options,
    )
    .await?;
    Ok((out.to_path_buf(), WasmFrom::Built))
}

const WASM_FILE: &str = "slidock_wasm_bg.wasm";

/// Runs the gate and writes its report; returns the report and its hash.
pub async fn run_determinism(
    ctx: &Context,
    request: &DeterminismRequest,
) -> Result<(DeterminismReport, Sha256)> {
    let manifest = load_manifest(&ctx.env.manifest_path)?;
    refuse_holdout(
        &manifest,
        &request.tiers,
        request.presentations.as_deref().unwrap_or_default(),
        &[],
        "determinism",
    )?;
    let binary = compiler_binary(
        &ctx.env.repo_root,
        &ctx.env.target_dir,
        ctx.env.compiler.as_deref(),
    )
    .await?;
    let corpus = Corpus::new(
        ctx.env.require_corpus_dir()?,
        FileHashes::new(&ctx.cache.hashes, true),
        &manifest,
    );
    let selection = Selection {
        tiers: request.tiers.clone(),
        presentations: request.presentations.clone(),
        frontends: None,
    };
    let mut pairs = Vec::new();
    for (presentation, source) in select_pairs(&manifest, &selection) {
        let file = corpus.source_file(source)?.ok_or_else(|| {
            BenchError::refused(format!(
                "{}: {} is not in the corpus",
                presentation.id, source.file
            ))
        })?;
        let rows: Vec<RowId> = presentation
            .rows(source)
            .into_iter()
            .map(|r| r.id)
            .collect();
        pairs.push((presentation, source, file, rows));
    }
    if pairs.is_empty() {
        return Err(BenchError::usage("the selection holds no pair"));
    }
    let root = ctx.cache.root.join("determinism");
    let work = root.join("work");
    let forbidden = forbidden_strings(&ctx.env.cache_dir, &ctx.env.corpus_dir);
    let (builds, reports, score) = in_work_dir(&work, async {
        let (pkg, wasm_from) = wasm_package(ctx, &work.join("wasm")).await?;
        let builds = Builds {
            native: Sha256::new(sha256_file(&binary)?)?,
            wasm: Sha256::new(sha256_file(&pkg.join(WASM_FILE))?)?,
            wasm_from,
        };
        let legs = Legs {
            binary: &binary,
            script: &ctx.env.bench_dir.join("wasm-cli.mjs"),
            pkg: &pkg,
            fonts: &ctx.cache.fonts,
            work: &work,
            forbidden: &forbidden,
        };
        let reports: BTreeMap<String, _> = run_legs(&legs, &pairs, log_line).await?;
        let score = if request.score {
            Some(score_twice(ctx, &pairs, &work).await?)
        } else {
            None
        };
        Ok((builds, reports, score))
    })
    .await?;
    let compiler = crate::stamp::compiler_stamp(&ctx.env.repo_root)
        .await?
        .content;
    let passed =
        reports.values().all(|r| r.passed()) && score.as_ref().is_none_or(ScoreReport::passed);
    let report = DeterminismReport {
        schema: REPORT_SCHEMA,
        compiler,
        builds,
        pairs: reports,
        score,
        passed,
    };
    let (sha, path) = write_report(&root, &report)?;
    eprintln!("determinism report: {}", path.display());
    Ok((report, sha))
}

pub fn print_report(report: &DeterminismReport) {
    for (pair, r) in &report.pairs {
        let wasm = match (&r.wasm, &r.wasm_difference) {
            (crate::determinism::WasmStatus::NativeOnly, _) => "native-only".to_owned(),
            (_, None) => "wasm = native".to_owned(),
            (_, Some(d)) => format!("wasm differs at {} +{} ({})", d.path, d.offset, d.detail),
        };
        let native = r.native_difference.as_ref().map_or_else(
            || "native twice equal".to_owned(),
            |d| format!("native differs at {} +{} ({})", d.path, d.offset, d.detail),
        );
        let only = r.only.iter().filter(|o| o.difference.is_none()).count();
        let scrub = r
            .scrub
            .as_ref()
            .map_or_else(String::new, |d| format!("; scrub: {} {}", d.path, d.detail));
        let failed: Vec<String> = r
            .native
            .iter()
            .chain(&r.reverse)
            .chain(r.wasm_run.iter())
            .filter_map(|run| run.problem.clone())
            .collect();
        let failed = if failed.is_empty() {
            String::new()
        } else {
            format!("; failed: {}", failed.join(", "))
        };
        println!(
            "{pair}: {native}; {wasm}; --only {only}/{} equal{scrub}{failed}",
            r.only.len()
        );
    }
    if let Some(score) = &report.score {
        println!(
            "scored twice: {} rows, {} differ",
            score.rows,
            score.differing.len()
        );
        for (pair, d) in &score.compile_differences {
            println!(
                "{pair}: the scored compile differs from leg A at {} +{} ({})",
                d.path, d.offset, d.detail
            );
        }
    }
    println!(
        "native {}, wasm {} ({:?})",
        report.builds.native.as_str().get(..12).unwrap_or_default(),
        report.builds.wasm.as_str().get(..12).unwrap_or_default(),
        report.builds.wasm_from
    );
    println!(
        "determinism: {}",
        if report.passed { "pass" } else { "FAIL" }
    );
}

pub async fn determinism(ctx: &Context, args: DeterminismArgs) -> Result<Exit> {
    let tiers = match crate::setup::list(args.tier.as_deref()) {
        Some(items) => items
            .iter()
            .map(|t| parse_tier(t).ok_or_else(|| BenchError::usage(format!("unknown tier {t}"))))
            .collect::<Result<Vec<_>>>()?,
        None => Tier::DEFAULT.to_vec(),
    };
    let request = DeterminismRequest {
        presentations: parse_list(args.presentation.as_deref(), "presentation")?,
        tiers,
        score: args.score,
    };
    let (report, _) = run_determinism(ctx, &request).await?;
    print_report(&report);
    Ok(Exit::check(report.passed))
}
