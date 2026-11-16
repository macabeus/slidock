//! `regression` and `diff`. The base is slidock's own baseline, which lives with the private data rather than in git:
//! `--base` names it (`baseline`, the default), a run, or a results file. Incomparable stamps exit 2, a comparison of
//! no row exits 3, so a run of another tier stays distinct from a pass.

use std::collections::BTreeSet;

use clap::Args;
use slidock_bench_schema::results::Results;
use slidock_bench_schema::{PresentationId, RowId};

use crate::compare::{
    Scope, diff_results, field_text, incomparable, regression, regression_failed,
};
use crate::corpus::{holdout_ids, load_manifest};
use crate::error::{BenchError, Exit, Result};
use crate::results::{read_results, resolve_results};
use crate::setup::{Context, parse_list};

#[derive(Debug, Args)]
pub struct Sides {
    /// The base: `baseline`, `latest`, a run key, or a results file or run folder.
    #[arg(long, default_value = "baseline")]
    base: String,
    /// The results compared with it [default: latest].
    #[arg(long, default_value = "latest")]
    results: String,
    /// Writes the report as canonical JSON to this file.
    #[arg(long)]
    json: Option<std::path::PathBuf>,
}

impl Sides {
    fn load(&self, ctx: &Context) -> Result<(Results, Results)> {
        let baseline = ctx.env.baseline_path();
        let base = read_results(&resolve_results(&ctx.cache.runs, &baseline, &self.base)?)?;
        let current = read_results(&resolve_results(&ctx.cache.runs, &baseline, &self.results)?)?;
        Ok((base, current))
    }
}

#[derive(Debug, Args)]
pub struct RegressionArgs {
    #[command(flatten)]
    sides: Sides,
    #[arg(long)]
    only: Option<String>,
    #[arg(long)]
    presentation: Option<String>,
    /// Lifted declines accepted for review.
    #[arg(long)]
    accept_undeclines: Option<String>,
}

#[derive(Debug, Args)]
pub struct DiffArgs {
    #[command(flatten)]
    sides: Sides,
}

fn class(results: &Results, id: &RowId) -> String {
    results
        .rows
        .get(id)
        .and_then(|row| serde_json::to_value(row.outcome).ok())
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "–".to_owned())
}

/// Drops the holdout's rows from both sides: they are never compared outside the gate.
fn without_holdout(ctx: &Context, results: &mut Results) -> Result<()> {
    let holdout: BTreeSet<PresentationId> = holdout_ids(&load_manifest(&ctx.env.manifest_path)?);
    results
        .rows
        .retain(|id, _| !holdout.contains(id.presentation()));
    results
        .stamp
        .corpus
        .retain(|pair, _| !holdout.contains(pair.presentation()));
    Ok(())
}

fn write_report<T: serde::Serialize>(path: Option<&std::path::Path>, report: &T) -> Result<()> {
    if let Some(path) = path {
        crate::cache::write_atomic(path, slidock_bench_schema::to_canonical(report)?)?;
    }
    Ok(())
}

pub fn regression_command(ctx: &Context, args: &RegressionArgs) -> Result<Exit> {
    let (mut base, mut current) = args.sides.load(ctx)?;
    without_holdout(ctx, &mut base)?;
    without_holdout(ctx, &mut current)?;
    let scope = Scope {
        only: parse_list(args.only.as_deref(), "row")?,
        presentations: parse_list(args.presentation.as_deref(), "presentation")?,
    };
    let accept: Vec<RowId> =
        parse_list(args.accept_undeclines.as_deref(), "row")?.unwrap_or_default();
    let unknown: Vec<String> = scope
        .only
        .iter()
        .flatten()
        .filter(|id| !base.rows.contains_key(*id))
        .map(ToString::to_string)
        .chain(
            scope
                .presentations
                .iter()
                .flatten()
                .filter(|p| !base.rows.keys().any(|id| id.presentation() == *p))
                .map(ToString::to_string),
        )
        .collect();
    if !unknown.is_empty() {
        return Err(BenchError::refused(format!(
            "the base has no {}",
            unknown.join(", ")
        )));
    }
    let problems = incomparable(&base, &current);
    if !problems.is_empty() {
        eprintln!(
            "incomparable: {} differ; re-baseline or compare like with like",
            problems.join(", ")
        );
        return Ok(Exit::Refused);
    }
    let mut report = regression(&base, &current, &scope);
    report.incomparable = problems;
    let buckets = [
        ("regressed", &report.regressed),
        ("improved", &report.improved),
        ("undeclined", &report.undeclined),
        ("vanished", &report.vanished),
        ("new", &report.new),
    ];
    for (bucket, rows) in buckets {
        for row in rows {
            println!(
                "{bucket:<10} {row}  {} → {}",
                class(&base, row),
                class(&current, row)
            );
        }
    }
    for lost in &report.shadow_losses {
        println!("shadows    {}  {}", lost.row, lost.guards.join("; "));
    }
    for lost in &report.gradient_losses {
        println!("gradients  {}  {}", lost.row, lost.guards.join("; "));
    }
    for eroded in &report.eroded {
        println!("eroded     {}  {}", eroded.row, eroded.guards.join("; "));
    }
    for guard in &report.frontend_guards {
        println!("guard      {guard}");
    }
    for guard in &report.pair_guards {
        println!("guard      {guard}");
    }
    for row in &report.flagged {
        println!("flagged    {row}  (reference or known loss; not counted)");
    }
    for row in &report.newly_flagged {
        println!("newflag    {row}  (flagged by this run only)");
    }
    println!(
        "{} rows compared, {} rows not run",
        report.compared, report.not_run
    );
    write_report(args.sides.json.as_deref(), &report)?;
    if report.compared == 0 {
        return Ok(Exit::NothingCompared);
    }
    Ok(Exit::check(!regression_failed(&report, &accept)))
}

pub fn diff_command(ctx: &Context, args: &DiffArgs) -> Result<Exit> {
    let (mut base, mut current) = args.sides.load(ctx)?;
    without_holdout(ctx, &mut base)?;
    without_holdout(ctx, &mut current)?;
    let diff = diff_results(&base, &current);
    for field in &diff.stamp {
        println!("stamp  {field} differs");
    }
    for row in &diff.vanished {
        println!("{row}  vanished");
    }
    for row in &diff.added {
        println!("{row}  new");
    }
    for change in &diff.changes {
        println!(
            "{}  {}: {} → {}",
            change.row,
            change.field,
            field_text(change.base.as_ref()),
            field_text(change.current.as_ref())
        );
    }
    println!(
        "{} rows compared, {} fields moved, {} rows not run",
        diff.compared,
        diff.changes.len(),
        diff.not_run
    );
    write_report(args.sides.json.as_deref(), &diff)?;
    if diff.compared == 0 {
        return Ok(Exit::NothingCompared);
    }
    if !diff.stamp.is_empty() {
        return Ok(Exit::Refused);
    }
    let moved = !diff.changes.is_empty() || !diff.vanished.is_empty() || !diff.added.is_empty();
    Ok(Exit::check(!moved))
}
