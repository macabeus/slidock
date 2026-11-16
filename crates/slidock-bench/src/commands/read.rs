//! Cheap reads of a results file, the static baseline by default: `show <row>`, `list` and `histogram`, the decline
//! work list. Rows flagged reference or known loss stay out of the last two unless `--all`.

use std::collections::BTreeMap;

use clap::Args;
use slidock_bench_schema::api::HistogramEntry;
use slidock_bench_schema::results::{Outcome, Results, RowRecord};
use slidock_bench_schema::{Frontend, RowId};

use crate::corpus::{load_manifest, refuse_holdout};
use crate::error::{BenchError, Exit, Result};
use crate::results::{read_results, resolve_results};
use crate::setup::Context;

#[derive(Debug, Args)]
pub struct ResultsArg {
    /// The results read: `baseline`, `latest`, a run key, or a path.
    #[arg(long, default_value = "baseline")]
    pub results: String,
}

impl ResultsArg {
    pub fn load(&self, ctx: &Context) -> Result<Results> {
        let path = resolve_results(&ctx.cache.runs, &ctx.env.baseline_path(), &self.results)?;
        read_results(&path)
    }
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    row: RowId,
    #[command(flatten)]
    results: ResultsArg,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    #[arg(long)]
    class: Option<String>,
    /// Rows declined with, failed with, or carrying a diagnostic of this code.
    #[arg(long)]
    code: Option<String>,
    #[arg(long)]
    frontend: Option<String>,
    /// Includes reference and known-loss rows.
    #[arg(long)]
    all: bool,
    #[command(flatten)]
    results: ResultsArg,
}

#[derive(Debug, Args)]
pub struct HistogramArgs {
    #[arg(long)]
    all: bool,
    #[command(flatten)]
    results: ResultsArg,
}

fn outcome_name(outcome: Outcome) -> String {
    serde_json::to_value(outcome)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn n(value: f64) -> String {
    slidock_canon::to_js_string(value)
}

/// One row as `list` prints it: class, then the decline, the failure or the scores, then its flags.
pub fn row_line(id: &RowId, row: &RowRecord) -> String {
    let outcome = row.outcome.classifier();
    let detail = match (&row.declined, row.reason, &row.scores) {
        (Some(declined), _, _) if outcome == Some(Outcome::Declined) => {
            let mut text = format!(" {}", declined.code);
            if let Some(feature) = &declined.feature {
                text.push_str(&format!(" {feature}"));
            }
            if let Some(via) = &declined.via {
                text.push_str(&format!(" via {via}"));
            }
            text
        }
        (_, Some(reason), _) => {
            let reason = serde_json::to_value(reason)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default();
            match &row.code {
                Some(code) => format!(" {reason} {code}"),
                None => format!(" {reason}"),
            }
        }
        (_, _, Some(s)) => format!(
            " ssim {} (p1 {}) ΔE {} recall {}",
            n(s.ssim.mean),
            n(s.ssim.p1),
            n(s.color.mean),
            n(s.text.recall)
        ),
        _ => String::new(),
    };
    let mut flags = Vec::new();
    if row.reference.is_some() {
        flags.push("reference".to_owned());
    }
    for kind in row.known_loss.iter().flatten() {
        let kind = serde_json::to_value(kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        flags.push(format!("knownLoss:{kind}"));
    }
    let flags = if flags.is_empty() {
        String::new()
    } else {
        format!("  [{}]", flags.join(", "))
    };
    let class = outcome.map_or_else(|| "motion".to_owned(), outcome_name);
    format!("{id}  {class}{detail}{flags}")
}

pub fn show(ctx: &Context, args: &ShowArgs) -> Result<Exit> {
    let manifest = load_manifest(&ctx.env.manifest_path)?;
    refuse_holdout(&manifest, &[], &[], std::slice::from_ref(&args.row), "show")?;
    let results = args.results.load(ctx)?;
    let row = results
        .rows
        .get(&args.row)
        .ok_or_else(|| BenchError::refused(format!("{} is not in the results", args.row)))?;
    print!("{}", slidock_bench_schema::to_canonical(row)?);
    Ok(Exit::Ok)
}

pub fn list(ctx: &Context, args: &ListArgs) -> Result<Exit> {
    let results = args.results.load(ctx)?;
    let class = args
        .class
        .as_deref()
        .map(|c| {
            serde_json::from_value::<Outcome>(serde_json::Value::String(c.to_owned()))
                .map_err(|_| BenchError::usage(format!("unknown class {c}")))
        })
        .transpose()?;
    let frontend = args
        .frontend
        .as_deref()
        .map(|f| {
            Frontend::from_wire(f).ok_or_else(|| BenchError::usage(format!("unknown frontend {f}")))
        })
        .transpose()?;
    for (id, row) in &results.rows {
        let code_matches = args.code.as_ref().is_none_or(|code| {
            row.declined.as_ref().is_some_and(|d| &d.code == code)
                || row.code.as_ref() == Some(code)
                || row.diagnostics.contains_key(code)
        });
        if (args.all || !row.flagged())
            && class.is_none_or(|c| row.outcome.classifier() == Some(c))
            && frontend.is_none_or(|f| row.frontend == f)
            && code_matches
        {
            println!("{}", row_line(id, row));
        }
    }
    Ok(Exit::Ok)
}

/// Declined rows counted by code and feature, most rows first, then by code and feature.
pub fn decline_histogram(results: &Results, all: bool) -> Vec<HistogramEntry> {
    let mut counts: BTreeMap<(String, String), u32> = BTreeMap::new();
    for row in results.rows.values() {
        let Some(declined) = &row.declined else {
            continue;
        };
        if !all && row.flagged() {
            continue;
        }
        *counts
            .entry((
                declined.code.clone(),
                declined.feature.clone().unwrap_or_default(),
            ))
            .or_default() += 1;
    }
    let mut entries: Vec<HistogramEntry> = counts
        .into_iter()
        .map(|((code, feature), rows)| HistogramEntry {
            code,
            feature,
            rows,
        })
        .collect();
    entries.sort_by(|a, b| {
        b.rows
            .cmp(&a.rows)
            .then_with(|| slidock_canon::utf16::cmp(&a.code, &b.code))
            .then_with(|| slidock_canon::utf16::cmp(&a.feature, &b.feature))
    });
    entries
}

pub fn histogram(ctx: &Context, args: &HistogramArgs) -> Result<Exit> {
    let results = args.results.load(ctx)?;
    for entry in decline_histogram(&results, args.all) {
        println!("{:>5}  {}  {}", entry.rows, entry.code, entry.feature);
    }
    Ok(Exit::Ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{record, results};

    #[test]
    fn rows_print_their_class_and_detail() {
        let row = record("s1", Outcome::Match, Some(0.99));
        assert_eq!(
            row_line(&row.id(), &row),
            "demo-basics:s1:pptx  match ssim 0.99 (p1 0.99) ΔE 1 recall 1"
        );
        let declined = record("s2", Outcome::Declined, None);
        assert_eq!(
            row_line(&declined.id(), &declined),
            "demo-basics:s2:pptx  declined PX4001"
        );
    }

    #[test]
    fn the_histogram_counts_declines_most_first_and_skips_flagged_rows() {
        let mut flagged = record("s3", Outcome::Declined, None);
        flagged.reference = Some(slidock_bench_schema::True);
        let mut other = record("s4", Outcome::Declined, None);
        other.declined = Some(slidock_bench_schema::results::Decline {
            code: "AA1000".into(),
            feature: Some("chart".into()),
            via: None,
        });
        let r = results(vec![
            record("s1", Outcome::Declined, None),
            record("s2", Outcome::Declined, None),
            flagged,
            other,
        ]);
        let entries = decline_histogram(&r, false);
        let rows: Vec<(&str, u32)> = entries.iter().map(|e| (e.code.as_str(), e.rows)).collect();
        assert_eq!(rows, [("PX4001", 2), ("AA1000", 1)]);
        assert_eq!(decline_histogram(&r, true).first().map(|e| e.rows), Some(3));
    }
}
