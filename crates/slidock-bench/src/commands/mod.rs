//! The command line. Exit codes: 0 ok, 1 a failed check, 2 refused or bad usage, 3
//! nothing compared.

mod budgets;
mod compare;
mod corpus;
pub mod determinism;
pub mod explain;
mod fonts;
mod oracle;
pub mod parity;
mod read;
mod renderer;
mod run;
mod status;
mod ui;

use std::path::PathBuf;

use clap::{CommandFactory, Parser, Subcommand};

use crate::env::BenchEnv;
use crate::error::{Exit, Result};
use crate::setup::Context;

#[derive(Debug, Parser)]
#[command(
    name = "slidock-bench",
    about = "The slidock fidelity bench",
    after_help = "Exit codes: 0 ok, 1 a failed check, 2 refused or bad usage, 3 nothing compared.\n\
                  Private data lives under $SLIDOCK_PRIVATE (default ~/.slidock): SLIDOCK_CORPUS_DIR, \
                  SLIDOCK_BENCH_CACHE, SLIDOCK_CORPUS_MANIFEST, SLIDOCK_KEYNOTE_ORACLES, SLIDOCK_BENCH_BASELINE \
                  override its parts; the goldens are under $SLIDOCK_PRIVATE/golden."
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compile, render, score and classify the corpus.
    Run(run::RunArgs),
    /// A clean full run of the default tiers, written as the baseline.
    Baseline,
    /// The bench's state: paths, corpus, fonts, renderer, oracles, the latest run and the baseline.
    Status,
    /// Builds and verifies oracles (attended for Keynote decks).
    Oracle(oracle::OracleArgs),
    /// The corpus manifest.
    Corpus {
        #[command(subcommand)]
        action: CorpusAction,
    },
    /// The pinned font set.
    Fonts {
        #[command(subcommand)]
        action: FontsAction,
    },
    /// The renderer container.
    Renderer {
        #[command(subcommand)]
        action: RendererAction,
    },
    /// Serves the bench UI and its read-only API on 127.0.0.1.
    Ui(ui::UiArgs),
    /// Writes the bench UI as one HTML file with the data inlined.
    Report(ui::ReportArgs),
    /// Why a row scored as it did: per-element findings, heatmap and side-by-side, rendered again from a run.
    Explain(explain::ExplainArgs),
    /// Native CLI against wasm under Node, in other orders, zones and locales: the same bytes.
    Determinism(determinism::DeterminismArgs),
    /// Compares a run with the baseline: how each row's class moved, and the erosion and frontend guards.
    Regression(compare::RegressionArgs),
    /// Every field that moved between two results.
    Diff(compare::DiffArgs),
    /// Prints one row's record.
    Show(read::ShowArgs),
    /// Lists rows, filtered.
    List(read::ListArgs),
    /// Declined rows by code and feature.
    Histogram(read::HistogramArgs),
    /// Drops old runs from the cache: keeps `runs/latest`, every baseline's run, the runs explain folders name, and
    /// the newest runs of each lane. Every run also collects once the cache passes 20 GiB.
    Gc {
        /// Finished runs kept per lane.
        #[arg(long, default_value_t = crate::gc::DEFAULT_KEEP)]
        keep: usize,
    },
    /// LibreOffice's round trips of the motion probes (Docker): each saved as ODP and PPTX, and every transition and
    /// step of the result compared with the source's, into `bench/motion/lo-roundtrip.json`.
    LoRoundtrip {
        /// Only checks that the committed report is the one of the current probes (by their sha256), without Docker:
        /// exit 1 when it is stale.
        #[arg(long)]
        check: bool,
    },
    /// Parity with slidock's goldens.
    Parity {
        #[command(subcommand)]
        action: ParityAction,
    },
    /// The wasm and player sizes, and each pair's compile time and peak memory natively and as wasm in headless
    /// Chromium through the app's compile worker: stamped into a run of this compiler, held by `bench/budgets.json`.
    Budgets(budgets::BudgetsArgs),
    /// Runs a command and prints its time and its peak resident set as JSON (the budgets' native leg).
    #[command(hide = true)]
    MeasureChild {
        #[arg(long)]
        timeout_secs: u64,
        #[arg(last = true, required = true)]
        command: Vec<std::ffi::OsString>,
    },
}

#[derive(Debug, Subcommand)]
enum CorpusAction {
    /// Prints where each corpus file resolves, by sha256.
    Locate,
}

#[derive(Debug, Subcommand)]
enum FontsAction {
    /// Materialises the lock in the cache, verifying every hash.
    Sync,
    /// Re-hashes every entry from its source into the lock, for review.
    Lock,
    /// Compares the cache with the lock.
    Status,
    /// Links the synced files into a folder under their own names.
    Link {
        dir: PathBuf,
        /// Only the fonts from pinned release URLs.
        #[arg(long)]
        releases: bool,
    },
    /// Records the fonts lane's Google stand-in (`bench/fonts/fonts-lane.lock.json`) from the CLI's own fetches over
    /// every lane deck, for review.
    LaneLock,
}

#[derive(Debug, Subcommand)]
enum RendererAction {
    /// Builds the image.
    Build,
    /// Starts (or reuses) the renderer and prints its identity.
    Start,
    /// Removes the renderer.
    Stop,
    /// Starts (or reuses) the renderer and prints its identity.
    Status,
}

#[derive(Debug, Subcommand)]
enum ParityAction {
    /// Renders and scores the goldens' static-lane output with this bench: the reference `report` and `check`
    /// compare against, in the cache.
    Reference(parity::ReferenceArgs),
    /// B and C: a run's rows against the golden reference (document bytes, render sha256, the first differing IR
    /// path against the golden IR), written to the run's parity.json.
    Report(parity::CompilerArgs),
    /// `report`, then exits with the gate's verdict.
    Check(parity::CompilerArgs),
}

fn gc(ctx: &Context, keep: usize) -> Result<Exit> {
    if keep == 0 {
        return Err(crate::error::BenchError::usage(
            "--keep is a positive integer: the newest run of each lane always stays",
        ));
    }
    let protected = crate::gc::protected_runs(&ctx.cache, &ctx.env.baseline_dir);
    let report = crate::gc::collect(&ctx.cache, keep, &protected)?;
    for key in &report.removed {
        println!("removed runs/{key}");
    }
    println!("{}", crate::gc::describe(&report));
    Ok(Exit::Ok)
}

/// The top-level usage text.
pub fn usage() -> String {
    Cli::command().render_usage().to_string()
}

pub async fn run(cli: Cli) -> Result<Exit> {
    if let Command::MeasureChild {
        timeout_secs,
        command,
    } = cli.command
    {
        return budgets::measure_child_command(timeout_secs, command).await;
    }
    let ctx = Context::new(BenchEnv::from_process()?);
    match cli.command {
        Command::Run(args) => {
            let exit = run::run(&ctx, args).await;
            crate::gc::after_run(&ctx.cache, &ctx.env.baseline_dir);
            exit
        }
        Command::Baseline => {
            let exit = run::baseline(&ctx).await;
            crate::gc::after_run(&ctx.cache, &ctx.env.baseline_dir);
            exit
        }
        Command::Gc { keep } => gc(&ctx, keep),
        Command::LoRoundtrip { check: true } => crate::lo_roundtrip::check(&ctx),
        Command::LoRoundtrip { check: false } => crate::lo_roundtrip::lo_roundtrip(&ctx).await,
        Command::Status => status::status(&ctx).await,
        Command::Oracle(args) => oracle::oracle(&ctx, args).await,
        Command::Corpus { action } => match action {
            CorpusAction::Locate => corpus::locate(&ctx),
        },
        Command::Fonts { action } => match action {
            FontsAction::Sync => fonts::sync(&ctx).await,
            FontsAction::Lock => fonts::lock(&ctx).await,
            FontsAction::Status => fonts::status(&ctx),
            FontsAction::Link { dir, releases } => fonts::link(&ctx, &dir, releases),
            FontsAction::LaneLock => crate::fonts_lane::record::record_lane_lock(&ctx).await,
        },
        Command::Renderer { action } => match action {
            RendererAction::Build => renderer::build(&ctx).await,
            RendererAction::Start | RendererAction::Status => renderer::start(&ctx).await,
            RendererAction::Stop => renderer::stop(&ctx).await,
        },
        Command::Ui(args) => ui::ui(&ctx, args).await,
        Command::Report(args) => ui::report(&ctx, args).await,
        Command::Explain(args) => explain::explain(&ctx, args).await,
        Command::Determinism(args) => determinism::determinism(&ctx, args).await,
        Command::Regression(args) => compare::regression_command(&ctx, &args),
        Command::Diff(args) => compare::diff_command(&ctx, &args),
        Command::Show(args) => read::show(&ctx, &args),
        Command::List(args) => read::list(&ctx, &args),
        Command::Histogram(args) => read::histogram(&ctx, &args),
        Command::Budgets(args) => budgets::budgets(&ctx, args).await,
        Command::MeasureChild { .. } => Err(crate::error::BenchError::usage(
            "measure-child runs on its own",
        )),
        Command::Parity { action } => match action {
            ParityAction::Reference(args) => parity::reference(&ctx, args).await,
            ParityAction::Report(args) => parity::compiler(&ctx, args, false).await,
            ParityAction::Check(args) => parity::compiler(&ctx, args, true).await,
        },
    }
}
