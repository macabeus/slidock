//! `run` and `baseline`. A full run writes a new run folder and points `runs/latest` at it. `--only`, `--presentation`,
//! `--frontend` and `--class` are overlays: they redo those rows and update them in `runs/latest/results.json`, so a
//! later `--class mismatch` still sees the whole row set.

use std::path::PathBuf;

use clap::Args;
use slidock_bench_schema::corpus::{CorpusManifest, Presentation, Source};
use slidock_bench_schema::results::{Outcome, Results};
use slidock_bench_schema::{Frontend, PresentationId, RowId, Tier};

use crate::compile::{CompileSettings, compiler_binary};
use crate::corpus::{Corpus, Selection, load_manifest, refuse_holdout, select_pairs};
use crate::error::{BenchError, Exit, Result};
use crate::hash::FileHashes;
use crate::oracle::resolve::OracleResolver;
use crate::results::{
    latest_run_dir, merge_results, read_results, refuse_drift, results_path, set_latest,
    write_results,
};
use crate::runner::{ExecuteOptions, PairJob, RowContext, execute_pairs};
use crate::setup::{Context, list, parse_list, parse_tier, start_session};
use crate::stamp::compiler_stamp;

#[derive(Debug, Args)]
pub struct RunArgs {
    /// Rows to redo (`<presentation>:<slide>:<frontend>`), compiling only their slides.
    #[arg(long)]
    only: Option<String>,
    #[arg(long)]
    presentation: Option<String>,
    #[arg(long)]
    frontend: Option<String>,
    /// Tiers to select [default: smoke,real; any but the holdout with --only or --presentation].
    #[arg(long)]
    tier: Option<String>,
    /// Redoes the rows the latest run classed so.
    #[arg(long)]
    class: Option<String>,
    /// Reuses nothing: file hashes, oracle rasters and words are computed again.
    #[arg(long)]
    no_cache: bool,
    #[arg(long, default_value_t = 4)]
    concurrency: usize,
    /// Scores with a font set that differs from the lock, stamping `fonts.drift`.
    #[arg(long)]
    allow_font_drift: bool,
    /// `static` (the default); `host`: the static run's rows mounted by the player in a shadow root, against their
    /// standalone renders; `fonts`: each deck bundled with its fonts and every glyph of every row attributed to
    /// the face that drew it, in a renderer with no font of its own; `motion`: each deck played by its own player,
    /// every transition and build checked against the deck, its IR and the probes' expectations; or `shell`: each
    /// deck's zip opened from `file://`, showing the motion run's frames at every position.
    #[arg(long, default_value = "static")]
    lane: String,
    /// The host lane's static run, or the shell lane's motion run: `latest`, a run key or a path.
    #[arg(long, default_value = "latest")]
    from: String,
    /// The host lane mounts each slide after its last build (exit builds applied) instead of with every element
    /// shown.
    #[arg(long)]
    end_state: bool,
    /// The renderer of the fonts, motion and shell lanes: `container` (the renderer image; for the fonts lane with
    /// only a last-resort font) or `native` (the same Chromium build on this computer, `SLIDOCK_BENCH_CHROMIUM` or
    /// Playwright's install).
    #[arg(long, default_value = "container")]
    renderer: String,
}

/// What a run selects and how.
pub struct RunRequest {
    pub only: Option<Vec<RowId>>,
    pub presentations: Option<Vec<PresentationId>>,
    pub frontends: Option<Vec<Frontend>>,
    pub tiers: Vec<Tier>,
    pub outcome: Option<Outcome>,
    pub no_cache: bool,
    pub concurrency: usize,
    pub allow_font_drift: bool,
}

fn parse_tiers(value: Option<&str>, named: bool) -> Result<Vec<Tier>> {
    match list(value) {
        Some(items) => items
            .iter()
            .map(|t| parse_tier(t).ok_or_else(|| BenchError::usage(format!("unknown tier {t}"))))
            .collect(),
        None if named => Ok(vec![Tier::Smoke, Tier::Real, Tier::Probe]),
        None => Ok(Tier::DEFAULT.to_vec()),
    }
}

fn parse_frontends(value: Option<&str>) -> Result<Option<Vec<Frontend>>> {
    list(value)
        .map(|items| {
            items
                .iter()
                .map(|f| {
                    Frontend::from_wire(f)
                        .ok_or_else(|| BenchError::usage(format!("unknown frontend {f}")))
                })
                .collect()
        })
        .transpose()
}

impl RunArgs {
    fn request(&self) -> Result<RunRequest> {
        let outcome = self
            .class
            .as_deref()
            .map(|c| {
                serde_json::from_value::<Outcome>(serde_json::Value::String(c.to_owned()))
                    .map_err(|_| BenchError::usage(format!("--class is one of match, close, mismatch, declined, render-error, failed; not {c}")))
            })
            .transpose()?;
        if self.concurrency == 0 {
            return Err(BenchError::usage("--concurrency is a positive integer"));
        }
        Ok(RunRequest {
            only: parse_list(self.only.as_deref(), "row")?,
            presentations: parse_list(self.presentation.as_deref(), "presentation")?,
            frontends: parse_frontends(self.frontend.as_deref())?,
            tiers: parse_tiers(
                self.tier.as_deref(),
                self.only.is_some() || self.presentation.is_some(),
            )?,
            outcome,
            no_cache: self.no_cache,
            concurrency: self.concurrency,
            allow_font_drift: self.allow_font_drift,
        })
    }
}

/// The jobs of a request: frozen rows of the selected pairs, narrowed by `--only` and `--class`.
fn select_jobs<'a>(
    manifest: &'a CorpusManifest,
    request: &RunRequest,
    previous: Option<&Results>,
) -> Result<Vec<PairJob<'a>>> {
    let selection = Selection {
        tiers: request.tiers.clone(),
        presentations: request.presentations.clone(),
        frontends: request.frontends.clone(),
    };
    let known: Vec<RowId> = manifest
        .expected_rows(&request.tiers)
        .into_iter()
        .map(|r| r.id)
        .collect();
    for id in request.only.iter().flatten() {
        if !known.contains(id) {
            return Err(BenchError::usage(format!(
                "{id} is not a frozen row of the selected tiers"
            )));
        }
    }
    let mut jobs = Vec::new();
    for (presentation, source) in select_pairs(manifest, &selection) {
        let mut rows = presentation.rows(source);
        if let Some(only) = &request.only {
            rows.retain(|row| only.contains(&row.id));
        }
        if let Some(outcome) = request.outcome {
            rows.retain(|row| {
                previous
                    .and_then(|p| p.rows.get(&row.id))
                    .and_then(|r| r.outcome.classifier())
                    == Some(outcome)
            });
        }
        if rows.is_empty() {
            continue;
        }
        let only = request.only.as_ref().map(|_| {
            rows.iter()
                .map(|row| row.id.slide().as_str().to_owned())
                .collect()
        });
        jobs.push(PairJob {
            presentation,
            source,
            rows,
            only,
        });
    }
    Ok(jobs)
}

fn log_line(line: &str) {
    eprintln!("{line}");
}

/// Runs a request and writes its results.
pub async fn run_bench(ctx: &Context, request: &RunRequest) -> Result<(Results, PathBuf)> {
    let manifest = load_manifest(&ctx.env.manifest_path)?;
    refuse_holdout(
        &manifest,
        &request.tiers,
        request.presentations.as_deref().unwrap_or_default(),
        request.only.as_deref().unwrap_or_default(),
        "run",
    )?;
    let corpus_dir = ctx.env.require_corpus_dir()?;
    let overlay = request.only.is_some()
        || request.presentations.is_some()
        || request.frontends.is_some()
        || request.outcome.is_some();
    let latest = latest_run_dir(&ctx.cache.runs);
    let previous = match (&latest, overlay) {
        (Some(dir), true) if results_path(dir).exists() => Some(read_results(&results_path(dir))?),
        _ => None,
    };
    if request.outcome.is_some() && previous.is_none() {
        return Err(BenchError::usage(
            "--class needs an earlier full run in runs/latest",
        ));
    }
    let jobs = select_jobs(&manifest, request, previous.as_ref())?;
    let selection = Selection {
        tiers: request.tiers.clone(),
        presentations: request.presentations.clone(),
        frontends: request.frontends.clone(),
    };
    let unrecorded: Vec<String> = select_pairs(&manifest, &selection)
        .into_iter()
        .filter(|(_, source)| source.rows.is_none())
        .map(|(presentation, source)| format!("{}:{}", presentation.id, source.frontend))
        .collect();
    if !unrecorded.is_empty() {
        eprintln!(
            "no frozen rows yet (record them with `corpus record-rows`): {}",
            unrecorded.join(", ")
        );
    }
    let binary = compiler_binary(
        &ctx.env.repo_root,
        &ctx.env.target_dir,
        ctx.env.compiler.as_deref(),
    )
    .await?;
    let pairs: Vec<(&Presentation, &Source)> = jobs
        .iter()
        .map(|job| (job.presentation, job.source))
        .collect();
    let session = start_session(ctx, &pairs, &request.tiers, request.allow_font_drift).await?;
    if let Some(previous) = &previous {
        refuse_drift(&previous.stamp, &session.stamp)?;
    }
    let run_dir = match (&previous, &latest) {
        (Some(_), Some(latest)) => latest.clone(),
        _ => ctx.cache.runs.join(session.stamp.run_key()?.as_str()),
    };
    std::fs::create_dir_all(&run_dir).map_err(BenchError::io(&run_dir))?;
    let corpus = Corpus::new(
        corpus_dir,
        FileHashes::new(&ctx.cache.hashes, !request.no_cache),
        &manifest,
    );
    let resolver = OracleResolver::new(
        session.renderer.clone(),
        session.poppler.clone(),
        ctx.env.keynote_dir.clone(),
        request.no_cache,
    );
    let settings = CompileSettings {
        binary,
        fonts_dir: ctx.cache.fonts.clone(),
        quiet: false,
    };
    let context = RowContext::new(
        Some(session.browser.clone()),
        ctx.cache.gif_frames.clone(),
        log_line,
    );
    let options = ExecuteOptions {
        run_dir: &run_dir,
        corpus: &corpus,
        settings: &settings,
        oracle: &resolver,
        content: session.stamp.compiler.content.clone(),
        concurrency: request.concurrency,
        reuse_compiled: false,
    };
    let executed = execute_pairs(&jobs, &options, &context).await?;
    let results = merge_results(&session.stamp, previous.as_ref(), executed.records)?;
    write_results(&results_path(&run_dir), &results)?;
    if previous.is_none() {
        let key = run_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        set_latest(&ctx.cache.runs, &key)?;
    }
    Ok((results, run_dir))
}

/// One line per frontend: rows and classes.
pub fn print_summary(results: &Results) {
    let groups = results
        .summary
        .by_frontend
        .iter()
        .map(|(f, s)| (f.to_string(), s))
        .chain(
            results
                .summary
                .reference
                .iter()
                .map(|(f, s)| (format!("{f} (reference)"), s)),
        );
    for (frontend, s) in groups {
        let classes: Vec<String> = Outcome::ALL
            .iter()
            .map(|o| {
                let name = serde_json::to_value(o)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default();
                format!("{name} {}", s.count(*o))
            })
            .collect();
        let median = s
            .ssim
            .median
            .map_or_else(|| "–".to_owned(), slidock_canon::to_js_string);
        println!(
            "{frontend}: {} rows: {}; ssim median {median}",
            s.rows,
            classes.join(", ")
        );
    }
}

pub async fn run(ctx: &Context, args: RunArgs) -> Result<Exit> {
    match args.lane.as_str() {
        "static" if args.end_state => {
            return Err(BenchError::usage("--end-state is the host lane's"));
        }
        "static" | "host" if args.renderer != "container" => {
            return Err(BenchError::usage(
                "--renderer is the fonts, motion and shell lanes' option",
            ));
        }
        "static" => {}
        "host" => {
            if args.class.is_some() || args.tier.is_some() || args.no_cache || args.allow_font_drift
            {
                return Err(BenchError::usage(
                    "the host lane takes --from, --only, --presentation, --frontend and --concurrency",
                ));
            }
            let request = crate::host::HostRequest {
                from: args.from.clone(),
                presentations: parse_list(args.presentation.as_deref(), "presentation")?,
                frontends: parse_frontends(args.frontend.as_deref())?,
                only: parse_list(args.only.as_deref(), "row")?,
                concurrency: args.concurrency.max(1),
                end_state: args.end_state,
            };
            return crate::host::host_lane(ctx, &request).await;
        }
        "fonts" => {
            if args.class.is_some() || args.no_cache || args.allow_font_drift || args.end_state {
                return Err(BenchError::usage(
                    "the fonts lane takes --tier, --only, --presentation, --frontend, --concurrency and --renderer",
                ));
            }
            let renderer = match args.renderer.as_str() {
                "container" => crate::fonts_lane::RendererChoice::Container,
                "native" => crate::fonts_lane::RendererChoice::Native(None),
                other => {
                    return Err(BenchError::usage(format!(
                        "--renderer is container or native, not {other}"
                    )));
                }
            };
            let tiers = match args.tier.as_deref() {
                Some(_) => parse_tiers(args.tier.as_deref(), false)?,
                None => vec![Tier::Smoke, Tier::Real, Tier::Probe],
            };
            let request = crate::fonts_lane::FontsRequest {
                tiers,
                presentations: parse_list(args.presentation.as_deref(), "presentation")?,
                frontends: parse_frontends(args.frontend.as_deref())?,
                only: parse_list(args.only.as_deref(), "row")?,
                concurrency: args.concurrency.max(1),
                renderer,
            };
            return crate::fonts_lane::fonts_lane(ctx, &request).await;
        }
        "motion" | "shell" => {
            if args.class.is_some()
                || args.no_cache
                || args.allow_font_drift
                || args.end_state
                || args.only.is_some()
            {
                return Err(BenchError::usage(
                    "the motion and shell lanes take --tier, --presentation, --frontend, --concurrency and --renderer (the shell lane also --from)",
                ));
            }
            let renderer = match args.renderer.as_str() {
                "container" => crate::motion_lane::RendererChoice::Container,
                "native" => crate::motion_lane::RendererChoice::Native(None),
                other => {
                    return Err(BenchError::usage(format!(
                        "--renderer is container or native, not {other}"
                    )));
                }
            };
            let tiers = match args.tier.as_deref() {
                Some(_) => parse_tiers(args.tier.as_deref(), false)?,
                None => vec![Tier::Smoke, Tier::Real, Tier::Probe],
            };
            let request = crate::motion_lane::MotionRequest {
                tiers,
                presentations: parse_list(args.presentation.as_deref(), "presentation")?,
                frontends: parse_frontends(args.frontend.as_deref())?,
                concurrency: args.concurrency.max(1),
                renderer,
            };
            return if args.lane == "motion" {
                crate::motion_lane::motion_lane(ctx, &request).await
            } else {
                crate::motion_lane::shell::shell_lane(ctx, &request, &args.from).await
            };
        }
        other => {
            return Err(BenchError::usage(format!(
                "--lane is static, host, fonts, motion or shell, not {other}"
            )));
        }
    }
    let request = args.request()?;
    let (results, run_dir) = run_bench(ctx, &request).await?;
    print_summary(&results);
    println!("results: {}", results_path(&run_dir).display());
    Ok(Exit::Ok)
}

/// A baseline is the output of exactly the code it names: a compiler and bench equal to HEAD, then the default tiers
/// run in full.
pub async fn baseline(ctx: &Context) -> Result<Exit> {
    let compiler = compiler_stamp(&ctx.env.repo_root).await?;
    if compiler.dirty {
        return Err(BenchError::refused(
            "the compiler or bench differs from HEAD; commit first (a baseline names a commit)",
        ));
    }
    // The determinism gate first: its report's hash is stamped into the baseline.
    let (gate, determinism) = crate::commands::determinism::run_determinism(
        ctx,
        &crate::commands::determinism::DeterminismRequest {
            presentations: None,
            tiers: Tier::DEFAULT.to_vec(),
            score: true,
        },
    )
    .await?;
    crate::commands::determinism::print_report(&gate);
    if !gate.passed {
        return Err(BenchError::refused(
            "the determinism gate failed; a baseline must be reproducible",
        ));
    }
    let request = RunRequest {
        only: None,
        presentations: None,
        frontends: None,
        tiers: Tier::DEFAULT.to_vec(),
        outcome: None,
        no_cache: false,
        concurrency: 4,
        allow_font_drift: false,
    };
    let (mut results, run_dir) = run_bench(ctx, &request).await?;
    if results.stamp.compiler.dirty || results.stamp.partial {
        return Err(BenchError::refused("the run is not a clean full run"));
    }
    results.stamp.determinism = Some(determinism);
    write_results(&results_path(&run_dir), &results)?;
    let path = ctx.env.baseline_path();
    write_results(&path, &results)?;
    print_summary(&results);
    println!("baseline written to {}", path.display());
    Ok(Exit::Ok)
}
