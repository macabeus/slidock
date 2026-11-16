//! `status`: where the bench reads and writes, and the state of each part (corpus, fonts, renderer, oracles, the
//! latest run and the baseline). It starts nothing and reads no corpus file.

use slidock_bench_schema::Tier;
use slidock_bench_schema::corpus::Oracle;

use crate::corpus::load_manifest;
use crate::error::{Exit, Result};
use crate::fonts::{font_set_state, load_font_lock};
use crate::oracle::keynote::export_path;
use crate::renderer::docker::{image_tag, recipe_hash, renderer_running};
use crate::results::{latest_run_dir, read_results, results_path};
use crate::setup::Context;
use slidock_bench_schema::Sha256;
use slidock_bench_schema::stamp::Stamp;

/// Why a baseline cannot serve `regression` and `diff` for this bench: made by another harness (they refuse with
/// exit 2), or not written by `baseline` (no determinism report, a partial run). Empty when it can.
pub fn baseline_problems(stamp: &Stamp, harness: &Sha256) -> Vec<&'static str> {
    let mut problems = Vec::new();
    if &stamp.harness != harness {
        problems.push("made by another harness");
    }
    if stamp.determinism.is_none() {
        problems.push("no determinism report");
    }
    if stamp.partial {
        problems.push("partial");
    }
    problems
}

pub async fn status(ctx: &Context) -> Result<Exit> {
    let env = &ctx.env;
    println!("repository   {}", env.repo_root.display());
    println!("corpus       {}", env.corpus_dir.display());
    println!("manifest     {}", env.manifest_path.display());
    println!("cache        {}", env.cache_dir.display());
    println!("keynote      {}", env.keynote_dir.display());
    println!("baseline     {}", env.baseline_path().display());

    match load_manifest(&env.manifest_path) {
        Ok(manifest) => {
            for presentation in &manifest.presentations {
                if presentation.tier == Tier::Holdout {
                    println!("  {} (holdout)", presentation.id);
                    continue;
                }
                let rows: Vec<String> = presentation
                    .sources
                    .iter()
                    .map(|s| format!("{} {}", s.frontend, s.rows.as_ref().map_or(0, Vec::len)))
                    .collect();
                let (kind, recorded, export) = match &presentation.oracle {
                    Oracle::PdfFile(o) => (
                        "pdf-file",
                        o.page_hashes.as_ref().map_or(0, |h| h.len()),
                        None,
                    ),
                    Oracle::KeynoteExport(o) => {
                        let source = presentation.sources.iter().find(|s| s.file == o.source);
                        let exported = source.and_then(|s| s.sha256.as_ref()).map(|sha| {
                            export_path(&env.keynote_dir, sha.as_str(), &o.exporter).is_file()
                        });
                        (
                            "keynote-export",
                            o.page_hashes.as_ref().map_or(0, |h| h.len()),
                            exported,
                        )
                    }
                };
                let export = match export {
                    Some(true) => ", export on disk",
                    Some(false) => ", NO export on disk",
                    None => "",
                };
                println!(
                    "  {} ({:?}): {}; oracle {kind}, {recorded} raster key(s) recorded{export}",
                    presentation.id,
                    presentation.tier,
                    rows.join(", ")
                );
            }
        }
        Err(error) => println!("  manifest unreadable: {error}"),
    }

    let lock = load_font_lock(&env.fonts_lock_path)?;
    let fonts = font_set_state(&lock, &ctx.cache)?;
    println!(
        "fonts        {} of {} synced, set {}{}",
        lock.fonts.len() - fonts.missing.len(),
        lock.fonts.len(),
        fonts.present_id,
        if fonts.present_id == fonts.lock_id {
            ""
        } else {
            " (differs from the lock)"
        }
    );
    let recipe = recipe_hash(&env.bench_dir)?;
    match renderer_running(&env.bench_dir, &ctx.cache.oracle).await {
        Ok(Some(name)) => println!("renderer     {} running ({})", image_tag(&recipe), name),
        Ok(None) => println!("renderer     {} not running", image_tag(&recipe)),
        Err(error) => println!("renderer     unknown: {error}"),
    }
    let harness = crate::stamp::harness_hash(&env.repo_root, None).await?;
    for (label, path) in [
        (
            "latest run",
            latest_run_dir(&ctx.cache.runs).map(|dir| results_path(&dir)),
        ),
        ("baseline", Some(env.baseline_path())),
    ] {
        match path.filter(|p| p.exists()) {
            Some(path) => match read_results(&path) {
                Ok(results) => {
                    let summary: Vec<String> = results
                        .summary
                        .by_frontend
                        .iter()
                        .map(|(f, s)| {
                            format!("{f} {}/{}/{} of {}", s.matched, s.close, s.mismatch, s.rows)
                        })
                        .collect();
                    println!(
                        "{label:<12} {} (content {}{}): {}",
                        path.display(),
                        results
                            .stamp
                            .compiler
                            .content
                            .as_str()
                            .get(..12)
                            .unwrap_or_default(),
                        if results.stamp.partial {
                            ", partial"
                        } else {
                            ""
                        },
                        summary.join("; ")
                    );
                    let problems = baseline_problems(&results.stamp, &harness);
                    if label == "baseline" && !problems.is_empty() {
                        println!(
                            "             stale ({}): run `slidock-bench baseline`",
                            problems.join(", ")
                        );
                    }
                }
                Err(error) => println!("{label:<12} unreadable: {error}"),
            },
            None => println!("{label:<12} none"),
        }
    }
    Ok(Exit::Ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_baseline_is_stale_under_another_harness_or_without_its_gate() {
        let text = include_str!("../../../slidock-bench-schema/tests/data/results.json");
        let mut stamp = slidock_bench_schema::results::Results::parse(text)
            .unwrap()
            .stamp;
        stamp.partial = false;
        stamp.determinism = Some(Sha256::of("gate"));
        let harness = stamp.harness.clone();
        assert!(baseline_problems(&stamp, &harness).is_empty());
        assert_eq!(
            baseline_problems(&stamp, &Sha256::of("other")),
            ["made by another harness"]
        );
        stamp.determinism = None;
        assert_eq!(
            baseline_problems(&stamp, &harness),
            ["no determinism report"]
        );
    }
}
